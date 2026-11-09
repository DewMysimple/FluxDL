use super::{hidden_command, stop_child};
use crate::errors::{AppError, AppResult, ErrorCode, ErrorPhase};
use std::{
    future::Future,
    path::PathBuf,
    process::{ExitStatus, Stdio},
    time::Duration,
};
use tokio::io::{AsyncRead, AsyncReadExt};

pub struct CaptureOutput {
    pub success: bool,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
}

async fn bounded_read<R: AsyncRead + Unpin>(
    mut reader: R,
    limit: usize,
    phase: ErrorPhase,
) -> AppResult<(Vec<u8>, bool)> {
    let mut output = Vec::new();
    let mut chunk = [0u8; 8192];
    let mut truncated = false;
    loop {
        let n = reader.read(&mut chunk).await.map_err(|error| {
            AppError::new(ErrorCode::ProcessRead, phase, "读取引擎输出失败。")
                .with_details(error.to_string())
        })?;
        if n == 0 {
            break;
        }
        let keep = n.min(limit.saturating_sub(output.len()));
        output.extend_from_slice(&chunk[..keep]);
        truncated |= keep < n;
    }
    Ok((output, truncated))
}

async fn collect_output<W, A, B>(
    wait: W,
    stdout: A,
    stderr: B,
    stdout_limit: usize,
    stderr_limit: usize,
    phase: ErrorPhase,
) -> AppResult<CaptureOutput>
where
    W: Future<Output = AppResult<ExitStatus>>,
    A: AsyncRead + Unpin,
    B: AsyncRead + Unpin,
{
    // A reader error cancels the wait immediately; the owner then cleans up its child.
    let (status, (out, stdout_truncated), (err, stderr_truncated)) = tokio::try_join!(
        wait,
        bounded_read(stdout, stdout_limit, phase),
        bounded_read(stderr, stderr_limit, phase)
    )?;
    Ok(CaptureOutput {
        success: status.success(),
        exit_code: status.code(),
        stdout: String::from_utf8_lossy(&out).into_owned(),
        stderr: String::from_utf8_lossy(&err).into_owned(),
        stdout_truncated,
        stderr_truncated,
    })
}

pub async fn capture(
    path: PathBuf,
    args: Vec<String>,
    timeout: Duration,
    stdout_limit: usize,
    stderr_limit: usize,
    phase: ErrorPhase,
) -> AppResult<CaptureOutput> {
    // The owning task survives cancellation of a probe/IPC waiter and always reaps its child.
    tokio::spawn(async move {
        capture_owned(path, args, timeout, stdout_limit, stderr_limit, phase).await
    })
    .await
    .map_err(|error| {
        AppError::new(ErrorCode::ProcessWait, phase, "引擎运行任务异常结束。")
            .with_details(error.to_string())
    })?
}

async fn capture_owned(
    path: PathBuf,
    args: Vec<String>,
    timeout: Duration,
    stdout_limit: usize,
    stderr_limit: usize,
    phase: ErrorPhase,
) -> AppResult<CaptureOutput> {
    let mut child = hidden_command(path)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            AppError::new(
                ErrorCode::ProcessSpawn,
                phase,
                "无法运行引擎，请检查所选文件与权限。",
            )
            .with_details(error.to_string())
        })?;
    #[cfg(windows)]
    let mut job = match crate::process_job::ProcessJob::assign(&child) {
        Ok(job) => Some(job),
        Err(error) => {
            let cleanup = stop_child(&mut child, phase).await;
            cleanup?;
            return Err(
                AppError::new(ErrorCode::ProcessCleanup, phase, "无法建立引擎进程隔离。")
                    .with_details(error),
            );
        }
    };
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| AppError::new(ErrorCode::ProcessRead, phase, "引擎标准输出不可用。"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| AppError::new(ErrorCode::ProcessRead, phase, "引擎错误输出不可用。"))?;
    let result = tokio::time::timeout(timeout, async {
        let wait = async {
            let status = child.wait().await.map_err(|error| {
                AppError::new(ErrorCode::ProcessWait, phase, "等待引擎失败。")
                    .with_details(error.to_string())
            });
            #[cfg(windows)]
            drop(job.take());
            status
        };
        collect_output(wait, stdout, stderr, stdout_limit, stderr_limit, phase).await
    })
    .await;
    match result {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(mut error)) => {
            #[cfg(windows)]
            drop(job.take());
            if let Err(cleanup) = stop_child(&mut child, phase).await {
                let source = error.details.as_deref().unwrap_or("");
                error = error
                    .clone()
                    .with_details(format!("{source}\n清理：{}", cleanup.safe_text()));
            }
            Err(error)
        }
        Err(_) => {
            #[cfg(windows)]
            drop(job.take());
            stop_child(&mut child, phase).await?;
            Err(AppError::new(
                ErrorCode::ProcessTimeout,
                phase,
                format!(
                    "引擎在 {} 秒内未完成，请检查网络后重试。",
                    timeout.as_secs()
                ),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn reader_fault_is_not_masked_by_a_hanging_child_or_timeout() {
        struct Broken;
        impl AsyncRead for Broken {
            fn poll_read(
                self: std::pin::Pin<&mut Self>,
                _: &mut std::task::Context<'_>,
                _: &mut tokio::io::ReadBuf<'_>,
            ) -> std::task::Poll<std::io::Result<()>> {
                std::task::Poll::Ready(Err(std::io::Error::other("fixture capture read fault")))
            }
        }
        let wait = std::future::pending::<AppResult<ExitStatus>>();
        let error = match tokio::time::timeout(
            Duration::from_millis(100),
            collect_output(wait, Broken, tokio::io::empty(), 16, 16, ErrorPhase::Probe),
        )
        .await
        .expect("read fault must return before a child wait")
        {
            Err(error) => error,
            Ok(_) => panic!("broken reader was ignored"),
        };
        assert_eq!(error.code, ErrorCode::ProcessRead);
        assert_eq!(error.phase, ErrorPhase::Probe);
        assert!(error.safe_text().contains("fixture capture read fault"));
    }

    #[cfg(windows)]
    fn powershell() -> PathBuf {
        PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into()))
            .join("System32/WindowsPowerShell/v1.0/powershell.exe")
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn real_stdout_and_stderr_over_limits_are_both_drained_before_success() {
        let output=capture(powershell(),vec!["-NoProfile".into(),"-NonInteractive".into(),"-Command".into(),"1..2000 | ForEach-Object { [Console]::Out.WriteLine(('O' * 96)); [Console]::Error.WriteLine(('E' * 96)) }; exit 0".into()],Duration::from_secs(8),1024,2048,ErrorPhase::Probe).await.unwrap();
        assert!(output.success);
        assert_eq!(output.exit_code, Some(0));
        assert_eq!(output.stdout.len(), 1024);
        assert_eq!(output.stderr.len(), 2048);
        assert!(output.stdout_truncated && output.stderr_truncated);
        // Each real pipe exceeds its OS buffer many times; success requires
        // concurrent draining after the retained memory limits have been reached.
        assert!(output.stdout.starts_with("OOOO"));
        assert!(output.stderr.starts_with("EEEE"));
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn real_capture_timeout_reaps_the_child_confirmed_by_a_fixed_kernel_handle() {
        use windows_sys::Win32::{
            Foundation::{CloseHandle, WAIT_OBJECT_0, WAIT_TIMEOUT},
            System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE},
        };
        let directory =
            std::env::temp_dir().join(format!("fluxdl-capture-timeout-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let script = directory.join("fixture.ps1");
        let marker = directory.join("owned-pid.txt");
        std::fs::write(&script,"param([string]$Marker)\n[IO.File]::WriteAllText($Marker,[string]$PID)\nStart-Sleep -Seconds 60\n").unwrap();
        let marker_arg = marker.to_string_lossy().into_owned();
        let script_arg = script.to_string_lossy().into_owned();
        let task = tokio::spawn(capture(
            powershell(),
            vec![
                "-NoProfile".into(),
                "-NonInteractive".into(),
                "-File".into(),
                script_arg,
                "-Marker".into(),
                marker_arg,
            ],
            Duration::from_secs(2),
            1024,
            1024,
            ErrorPhase::Probe,
        ));
        let pid = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Ok(value) = tokio::fs::read_to_string(&marker).await {
                    if let Ok(pid) = value.trim().parse::<u32>() {
                        break pid;
                    }
                }
                assert!(
                    !task.is_finished(),
                    "fixture must publish its own PID before the capture timeout"
                );
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let process = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
        assert!(!process.is_null());
        assert_eq!(unsafe { WaitForSingleObject(process, 0) }, WAIT_TIMEOUT);
        let error = match task.await.unwrap() {
            Err(error) => error,
            Ok(_) => panic!("the live fixture must exceed the capture deadline"),
        };
        assert_eq!(error.code, ErrorCode::ProcessTimeout);
        assert_eq!(error.phase, ErrorPhase::Probe);
        assert_eq!(unsafe { WaitForSingleObject(process, 1000) }, WAIT_OBJECT_0);
        unsafe {
            CloseHandle(process);
        }
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn exited_parent_with_an_open_inherited_pipe_still_has_a_bounded_capture_deadline() {
        use std::os::windows::process::ExitStatusExt;
        struct OpenPipe;
        impl AsyncRead for OpenPipe {
            fn poll_read(
                self: std::pin::Pin<&mut Self>,
                _: &mut std::task::Context<'_>,
                _: &mut tokio::io::ReadBuf<'_>,
            ) -> std::task::Poll<std::io::Result<()>> {
                std::task::Poll::Pending
            }
        }
        let wait = std::future::ready(Ok(ExitStatus::from_raw(0)));
        assert!(tokio::time::timeout(
            Duration::from_millis(50),
            collect_output(
                wait,
                OpenPipe,
                tokio::io::empty(),
                16,
                16,
                ErrorPhase::Probe
            )
        )
        .await
        .is_err());
    }
}
