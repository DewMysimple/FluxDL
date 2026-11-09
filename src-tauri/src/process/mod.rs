//! Concrete subprocess ownership. No queue, SQLite, or yt-dlp protocol rules live here.
mod capture;
mod download;
mod lines;
pub use capture::{capture, CaptureOutput};
pub use download::{start_download, Cancellation, DownloadHandle, ProcessOutcome, ProcessSpec};
pub use lines::{OutputLine, OutputStream, ProcessEvent};

use crate::errors::{AppError, AppResult, ErrorCode, ErrorPhase};
use std::{ffi::OsStr, path::PathBuf, process::Stdio, time::Duration};
use tokio::process::{Child, Command};

pub fn hidden_command(path: impl AsRef<OsStr>) -> Command {
    let mut command = Command::new(path);
    command.stdin(Stdio::null()).kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    command
}

async fn kill_tree(pid: u32, phase: ErrorPhase) -> AppResult<()> {
    #[cfg(windows)]
    {
        let system = std::env::var_os("SystemRoot")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        tokio::time::timeout(
            Duration::from_secs(5),
            hidden_command(system.join("System32/taskkill.exe"))
                .args(["/PID", &pid.to_string(), "/T", "/F"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status(),
        )
        .await
        .map_err(|_| AppError::new(ErrorCode::ProcessCleanup, phase, "终止引擎进程树超时。"))?
        .map_err(|error| {
            AppError::new(ErrorCode::ProcessCleanup, phase, "无法终止引擎进程树。")
                .with_details(error.to_string())
        })?;
        // Nonzero can mean the child exited just before taskkill. The owned child is awaited below.
    }
    #[cfg(not(windows))]
    let _ = (pid, phase);
    Ok(())
}

async fn stop_child(child: &mut Child, phase: ErrorPhase) -> AppResult<()> {
    if child.try_wait().is_ok_and(|status| status.is_some()) {
        return Ok(());
    }
    let fallback = match child.id() {
        Some(pid) => kill_tree(pid, phase).await,
        None => Ok(()),
    };
    reap_child(child, phase).await?;
    fallback
}
async fn reap_child(child: &mut Child, phase: ErrorPhase) -> AppResult<()> {
    let _ = child.start_kill();
    tokio::time::timeout(Duration::from_secs(6), child.wait())
        .await
        .map_err(|_| AppError::new(ErrorCode::ProcessCleanup, phase, "等待引擎进程停止超时。"))?
        .map_err(|error| {
            AppError::new(ErrorCode::ProcessCleanup, phase, "无法回收引擎进程。")
                .with_details(error.to_string())
        })?;
    Ok(())
}
