use super::{
    hidden_command, lines::read_lines, reap_child, stop_child, OutputStream, ProcessEvent,
};
use crate::errors::{AppError, AppResult, ErrorCode, ErrorPhase};
use std::{
    path::PathBuf,
    process::{ExitStatus, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{
    sync::{mpsc, Notify},
    task::JoinHandle,
};

#[derive(Default)]
struct CancelState {
    requested: AtomicBool,
    pid: AtomicU32,
    wake: Notify,
    #[cfg(test)]
    cleanup_fault: std::sync::Mutex<Option<Arc<TestCleanupFault>>>,
}
#[derive(Clone, Default)]
pub struct Cancellation(Arc<CancelState>);
impl Cancellation {
    pub fn cancel(&self) {
        self.0.requested.store(true, Ordering::SeqCst);
        self.0.wake.notify_one();
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.requested.load(Ordering::SeqCst)
    }
    pub fn process_id(&self) -> Option<u32> {
        match self.0.pid.load(Ordering::SeqCst) {
            0 => None,
            id => Some(id),
        }
    }
    async fn cancelled(&self) {
        loop {
            let notified = self.0.wake.notified();
            if self.is_cancelled() {
                return;
            }
            notified.await;
        }
    }
}
pub struct ProcessSpec {
    pub executable: PathBuf,
    pub args: Vec<String>,
    pub directory: PathBuf,
}
#[derive(Debug)]
pub struct ProcessOutcome {
    pub status: Option<ExitStatus>,
    pub cancelled: bool,
}
pub struct DownloadHandle {
    events: mpsc::Receiver<ProcessEvent>,
    owner: Option<JoinHandle<AppResult<ProcessOutcome>>>,
    cancel: Cancellation,
    finished: bool,
}
impl DownloadHandle {
    pub async fn recv(&mut self) -> Option<ProcessEvent> {
        self.events.recv().await
    }
    pub async fn finish(mut self) -> AppResult<ProcessOutcome> {
        let owner = self.owner.take().expect("download owner is consumed once");
        let result = owner.await.map_err(|error| {
            AppError::new(
                ErrorCode::ProcessWait,
                ErrorPhase::Download,
                "下载进程管理任务异常结束。",
            )
            .with_details(error.to_string())
        });
        self.finished = true;
        result?
    }
}
impl Drop for DownloadHandle {
    fn drop(&mut self) {
        if !self.finished {
            self.cancel.cancel();
        }
    }
}

#[cfg(test)]
#[derive(Default)]
struct TestCleanupFault {
    released: AtomicBool,
    attempts: std::sync::atomic::AtomicUsize,
}
#[cfg(test)]
pub struct CleanupTestGate {
    state: Arc<TestCleanupFault>,
    cancel: Cancellation,
}
#[cfg(test)]
impl CleanupTestGate {
    pub fn release(&self) {
        self.state.released.store(true, Ordering::SeqCst);
        self.cancel.0.wake.notify_one();
    }
    pub fn attempts(&self) -> usize {
        self.state.attempts.load(Ordering::SeqCst)
    }
}
#[cfg(test)]
impl Drop for CleanupTestGate {
    fn drop(&mut self) {
        self.release();
    }
}
impl Cancellation {
    #[cfg(test)]
    pub fn hold_cleanup(&self) -> CleanupTestGate {
        let state = Arc::new(TestCleanupFault::default());
        *self.0.cleanup_fault.lock().expect("test cleanup gate") = Some(state.clone());
        CleanupTestGate {
            state,
            cancel: self.clone(),
        }
    }
    fn cleanup_fault(&self) -> Option<AppError> {
        #[cfg(test)]
        if let Some(state) = self
            .0
            .cleanup_fault
            .lock()
            .expect("test cleanup gate")
            .as_ref()
        {
            state.attempts.fetch_add(1, Ordering::SeqCst);
            if !state.released.load(Ordering::SeqCst) {
                return Some(AppError::new(
                    ErrorCode::ProcessCleanup,
                    ErrorPhase::Download,
                    "注入的停止故障：仍由原 owner 隔离，等待重试。",
                ));
            }
        }
        None
    }
}
pub fn start_download(spec: ProcessSpec, cancel: Cancellation) -> DownloadHandle {
    start_download_traced(spec, cancel, None)
}
pub fn start_download_traced(
    spec: ProcessSpec,
    cancel: Cancellation,
    trace: Option<crate::diagnostics::Trace>,
) -> DownloadHandle {
    let (sender, events) = mpsc::channel(128);
    let signal = cancel.clone();
    let owner = tokio::spawn(async move { run(spec, signal, sender, trace).await });
    DownloadHandle {
        events,
        owner: Some(owner),
        cancel,
        finished: false,
    }
}

async fn run(
    spec: ProcessSpec,
    cancel: Cancellation,
    sender: mpsc::Sender<ProcessEvent>,
    trace: Option<crate::diagnostics::Trace>,
) -> AppResult<ProcessOutcome> {
    let phase = ErrorPhase::Download;
    if cancel.is_cancelled() {
        return Ok(ProcessOutcome {
            status: None,
            cancelled: true,
        });
    }
    use crate::diagnostics::Stage;
    if let Some(t) = &trace {
        t.begin(Stage::DownloadEngineSpawn);
    }
    let spawned = hidden_command(spec.executable)
        .args(spec.args)
        .current_dir(spec.directory)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            AppError::new(
                ErrorCode::ProcessSpawn,
                phase,
                "无法启动 yt-dlp，请检查引擎路径与权限。",
            )
            .with_details(error.to_string())
        });
    if let Some(t) = &trace {
        t.result(Stage::DownloadEngineSpawn, &spawned);
    }
    let mut child = spawned?;
    if let Some(t) = &trace {
        t.begin(Stage::DownloadFirstProgress);
    }
    cancel
        .0
        .pid
        .store(child.id().unwrap_or(0), Ordering::SeqCst);
    let mut failure = None;
    #[cfg(windows)]
    let mut job = match crate::process_job::ProcessJob::assign(&child) {
        Ok(job) => Some(job),
        Err(error) => {
            failure = Some(
                AppError::new(
                    ErrorCode::ProcessCleanup,
                    phase,
                    "无法建立安全下载进程组；原 owner 将继续停止并回收进程。",
                )
                .with_details(error),
            );
            cancel.cancel();
            None
        }
    };
    let (lines, mut receiver) = mpsc::channel(128);
    let mut readers = Vec::new();
    if let Some(stdout) = child.stdout.take() {
        readers.push(tokio::spawn(read_lines(
            stdout,
            OutputStream::Stdout,
            lines.clone(),
            phase,
        )));
    }
    if let Some(stderr) = child.stderr.take() {
        readers.push(tokio::spawn(read_lines(
            stderr,
            OutputStream::Stderr,
            lines.clone(),
            phase,
        )));
    }
    drop(lines);
    let mut exit = None;
    let mut closed = false;
    let mut stop_requested = false;
    let mut stop_now = false;
    let mut wait_faulted = false;
    let mut last_delivered = None;
    let mut pending_fault = failure.clone();
    let mut retry = tokio::time::interval(Duration::from_secs(1));
    retry.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut drain = std::pin::pin!(tokio::time::sleep(Duration::from_secs(86400 * 365)));
    while exit.is_none() || !closed {
        tokio::select! {
            permit=sender.reserve(),if pending_fault.is_some()=>{
                match permit {
                    Ok(permit)=>{let error=pending_fault.take().expect("pending fault");permit.send(ProcessEvent::ReadFault(error.clone()));last_delivered=Some(error);}
                    Err(_)=>{pending_fault=None;cancel.cancel();}
                }
            }
            status=child.wait(),if exit.is_none()&&!wait_faulted=>{
                match status {
                    Ok(status)=>{
                        exit=Some(status);
                        #[cfg(windows)] drop(job.take());
                        drain.as_mut().reset(tokio::time::Instant::now()+Duration::from_secs(3));
                    }
                    Err(error)=>{
                        let error=AppError::new(ErrorCode::ProcessWait,phase,"读取下载退出状态失败；原 owner 将继续停止并回收进程。").with_details(error.to_string());
                        queue_fault(&mut pending_fault,&last_delivered,&error);
                        failure.get_or_insert(error);cancel.cancel();stop_requested=true;stop_now=true;wait_faulted=true;
                    }
                }
            }
            _=cancel.cancelled(),if !stop_requested&&exit.is_none()=>{stop_requested=true;stop_now=true;}
            _=cancel.0.wake.notified(),if stop_requested&&exit.is_none()=>{stop_now=true;}
            _=retry.tick(),if stop_requested&&exit.is_none()=>{stop_now=true;wait_faulted=false;}
            event=receiver.recv(),if !closed=>{
                match event {
                    Some(event)=>{
                        if let ProcessEvent::ReadFault(error)=&event {
                            queue_fault(&mut pending_fault,&last_delivered,error);
                            failure.get_or_insert(error.clone());cancel.cancel();
                            continue;
                        }
                        tokio::select! {
                            result=sender.send(event)=>{if result.is_err(){cancel.cancel();}}
                            _=cancel.cancelled()=>{}
                        }
                    }
                    None=>closed=true,
                }
            }
            _=&mut drain,if exit.is_some()&&!closed=>{failure=Some(AppError::new(ErrorCode::ProcessRead,phase,"引擎退出后输出管道未及时关闭。"));break;}
        }
        if stop_now && exit.is_none() {
            stop_now = false;
            let stopped = if let Some(error) = cancel.cleanup_fault() {
                Err(error)
            } else {
                #[cfg(windows)]
                let contained = job.as_ref().is_some_and(|job| job.terminate().is_ok());
                #[cfg(not(windows))]
                let contained = false;
                if contained {
                    reap_child(&mut child, phase).await
                } else {
                    stop_child(&mut child, phase).await
                }
            };
            if let Err(error) = stopped {
                queue_fault(&mut pending_fault, &last_delivered, &error);
                failure.get_or_insert(error);
                // Isolation remains live. A failed stop never clears PID or releases this owner.
                retry.reset();
            }
        }
    }
    #[cfg(windows)]
    drop(job.take());
    // This point is reached only after an actual wait result, never after an unconfirmed stop.
    drop(receiver);
    for reader in readers {
        let aborted = !reader.is_finished();
        if aborted {
            reader.abort();
        }
        if let Err(error) = reader.await {
            if !(aborted && error.is_cancelled()) {
                failure = Some(
                    AppError::new(ErrorCode::ProcessRead, phase, "引擎输出读取任务异常结束。")
                        .with_details(error.to_string()),
                );
            }
        }
    }
    cancel.0.pid.store(0, Ordering::SeqCst);
    if let Some(error) = failure {
        return Err(error);
    }
    Ok(ProcessOutcome {
        status: exit,
        cancelled: cancel.is_cancelled(),
    })
}

fn queue_fault(
    pending: &mut Option<AppError>,
    last_delivered: &Option<AppError>,
    error: &AppError,
) {
    if pending.is_none() && last_delivered.as_ref() != Some(error) {
        *pending = Some(error.clone());
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    #[tokio::test]
    async fn cancellation_stops_the_owned_child_even_when_the_output_consumer_is_full() {
        let powershell =
            PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into()))
                .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let cancel = Cancellation::default();
        let handle=start_download(ProcessSpec { executable:powershell,args:vec!["-NoProfile".into(),"-NonInteractive".into(),"-Command".into(),"1..4000 | ForEach-Object { Write-Output ('output-' + $_) }; Start-Sleep -Seconds 60".into()],directory:std::env::temp_dir() },cancel.clone());
        tokio::time::timeout(Duration::from_secs(5), async {
            while cancel.process_id().is_none() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while handle.events.capacity() != 0 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the downstream event channel must actually be full");
        cancel.cancel();
        let outcome = tokio::time::timeout(Duration::from_secs(6), handle.finish())
            .await
            .expect("full output must not lose the cancel wake")
            .unwrap();
        assert!(outcome.cancelled);
        assert!(cancel.process_id().is_none());
    }
    #[tokio::test]
    async fn dropping_a_finish_waiter_cancels_without_aborting_the_child_owner() {
        let powershell =
            PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into()))
                .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let cancel = Cancellation::default();
        let handle = start_download(
            ProcessSpec {
                executable: powershell,
                args: vec![
                    "-NoProfile".into(),
                    "-NonInteractive".into(),
                    "-Command".into(),
                    "Start-Sleep -Seconds 60".into(),
                ],
                directory: std::env::temp_dir(),
            },
            cancel.clone(),
        );
        tokio::time::timeout(Duration::from_secs(5), async {
            while cancel.process_id().is_none() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let waiter = tokio::spawn(handle.finish());
        tokio::task::yield_now().await;
        waiter.abort();
        assert!(waiter.await.unwrap_err().is_cancelled());
        assert!(cancel.is_cancelled());
        tokio::time::timeout(Duration::from_secs(6), async {
            while cancel.process_id().is_some() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the detached owner must reap its child after its waiter is dropped");
    }
    #[tokio::test]
    async fn failed_cleanup_is_visible_and_keeps_the_actual_child_owned_until_recovery() {
        for attempts in [1, 3] {
            let powershell = PathBuf::from(
                std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into()),
            )
            .join("System32/WindowsPowerShell/v1.0/powershell.exe");
            let cancel = Cancellation::default();
            let gate = cancel.hold_cleanup();
            let script = if attempts == 3 {
                "1..4000 | ForEach-Object { Write-Output ('backpressure-' + $_) }; Start-Sleep -Seconds 60"
            } else {
                "Start-Sleep -Seconds 60"
            };
            let mut handle = start_download(
                ProcessSpec {
                    executable: powershell,
                    args: vec![
                        "-NoProfile".into(),
                        "-NonInteractive".into(),
                        "-Command".into(),
                        script.into(),
                    ],
                    directory: std::env::temp_dir(),
                },
                cancel.clone(),
            );
            tokio::time::timeout(Duration::from_secs(5), async {
                while cancel.process_id().is_none() {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            let process = unsafe {
                windows_sys::Win32::System::Threading::OpenProcess(
                    windows_sys::Win32::System::Threading::PROCESS_SYNCHRONIZE,
                    0,
                    cancel.process_id().unwrap(),
                )
            };
            assert!(!process.is_null());
            if attempts == 3 {
                tokio::time::timeout(Duration::from_secs(5), async {
                    while handle.events.capacity() != 0 {
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                })
                .await
                .expect("diagnostic delivery test requires an actually full downstream");
            }
            cancel.cancel();
            if attempts == 3 {
                tokio::time::timeout(Duration::from_secs(2), async {
                    while gate.attempts() == 0 {
                        tokio::task::yield_now().await;
                    }
                })
                .await
                .expect("full downstream cannot block stop attempts");
            }
            tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    match handle.recv().await {
                        Some(ProcessEvent::ReadFault(error)) => {
                            assert_eq!(error.code, ErrorCode::ProcessCleanup);
                            break;
                        }
                        Some(ProcessEvent::Line(_)) => {}
                        None => {
                            panic!("cleanup diagnostic must be delivered after capacity returns")
                        }
                    }
                }
            })
            .await
            .expect("pending cleanup diagnostic was lost to backpressure");
            tokio::time::timeout(Duration::from_secs(4), async {
                while gate.attempts() < attempts {
                    cancel.cancel();
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
            assert!(cancel.process_id().is_some());
            assert!(!handle.owner.as_ref().unwrap().is_finished());
            assert_eq!(
                unsafe { windows_sys::Win32::System::Threading::WaitForSingleObject(process, 0) },
                windows_sys::Win32::Foundation::WAIT_TIMEOUT
            );
            loop {
                match tokio::time::timeout(Duration::from_millis(50), handle.recv()).await {
                    Err(_) => break,
                    Ok(Some(ProcessEvent::Line(_))) => {}
                    _ => panic!(
                        "identical cleanup errors must be coalesced while the owner remains live"
                    ),
                }
            }
            gate.release();
            while tokio::time::timeout(Duration::from_secs(6), handle.recv())
                .await
                .unwrap()
                .is_some()
            {}
            let error = handle
                .finish()
                .await
                .expect_err("the cleanup failure must remain visible in the final result");
            assert_eq!(error.code, ErrorCode::ProcessCleanup);
            assert!(cancel.process_id().is_none());
            assert_eq!(
                unsafe {
                    windows_sys::Win32::System::Threading::WaitForSingleObject(process, 1000)
                },
                windows_sys::Win32::Foundation::WAIT_OBJECT_0
            );
            unsafe {
                windows_sys::Win32::Foundation::CloseHandle(process);
            }
        }
    }
}
