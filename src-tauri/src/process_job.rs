//! Windows process lifetime guard shared by version probes, metadata extraction,
//! and downloads. A child inherits job membership; application exit closes every
//! job handle and terminates remaining yt-dlp / FFmpeg / JavaScript descendants.

pub(crate) struct ProcessJob(windows_sys::Win32::Foundation::HANDLE);
// Windows job handles may be used or closed from any thread; ownership stays here.
unsafe impl Send for ProcessJob {}
unsafe impl Sync for ProcessJob {}

impl ProcessJob {
    pub(crate) fn assign(child: &tokio::process::Child) -> Result<Self, String> {
        use windows_sys::Win32::System::JobObjects::*;
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let job = Self(handle);
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if unsafe {
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const _,
                std::mem::size_of_val(&info) as u32,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let process = child.raw_handle().ok_or("子进程已退出。")?;
        if unsafe { AssignProcessToJobObject(handle, process as _) } == 0 {
            let error = std::io::Error::last_os_error();
            // Very short version/metadata commands can exit between spawn and
            // assignment. The caller still checks exit status and drains pipes
            // under its timeout; only a genuinely live uncontained process fails.
            if unsafe {
                windows_sys::Win32::System::Threading::WaitForSingleObject(process as _, 0)
            } == windows_sys::Win32::Foundation::WAIT_OBJECT_0
            {
                return Ok(job);
            }
            return Err(error.to_string());
        }
        Ok(job)
    }

    pub(crate) fn terminate(&self) -> Result<(), String> {
        if unsafe { windows_sys::Win32::System::JobObjects::TerminateJobObject(self.0, 1) } == 0 {
            Err(std::io::Error::last_os_error().to_string())
        } else {
            Ok(())
        }
    }
}

impl Drop for ProcessJob {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ProcessJob;
    use std::{path::PathBuf, process::Stdio, time::Duration};
    use tokio::process::Command;
    #[cfg(windows)]
    #[tokio::test]
    async fn windows_job_drop_kills_parent_and_descendant() {
        use tokio::io::AsyncBufReadExt;
        use windows_sys::Win32::{
            Foundation::{CloseHandle, WAIT_OBJECT_0, WAIT_TIMEOUT},
            System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE},
        };
        let powershell =
            PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into()))
                .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let mut command = Command::new(powershell);
        command.args(["-NoProfile","-NonInteractive","-Command","$p = Start-Process -FilePath (Join-Path $PSHOME 'powershell.exe') -ArgumentList @('-NoProfile','-NonInteractive','-Command','Start-Sleep -Seconds 60') -WindowStyle Hidden -PassThru; Write-Output $p.Id; Start-Sleep -Seconds 60"]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true).creation_flags(0x08000000);
        let mut parent = command.spawn().unwrap();
        let job = ProcessJob::assign(&parent).unwrap();
        let mut reader = tokio::io::BufReader::new(parent.stdout.take().unwrap());
        let mut line = String::new();
        tokio::time::timeout(Duration::from_secs(10), reader.read_line(&mut line))
            .await
            .unwrap()
            .unwrap();
        let descendant_pid: u32 = line.trim().parse().unwrap();
        let descendant = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, descendant_pid) };
        assert!(!descendant.is_null());
        assert_eq!(unsafe { WaitForSingleObject(descendant, 0) }, WAIT_TIMEOUT);
        drop(job);
        tokio::time::timeout(Duration::from_secs(5), parent.wait())
            .await
            .unwrap()
            .unwrap();
        // KILL_ON_JOB_CLOSE can report exit code zero. Timely process completion
        // and a signalled descendant handle establish termination, not the code.
        assert_eq!(
            unsafe { WaitForSingleObject(descendant, 2000) },
            WAIT_OBJECT_0
        );
        unsafe {
            CloseHandle(descendant);
        }
    }
}
