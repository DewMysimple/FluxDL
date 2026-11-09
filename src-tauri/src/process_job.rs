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
