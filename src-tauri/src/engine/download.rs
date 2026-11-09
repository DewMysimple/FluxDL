//! yt-dlp output adapter. Manager receives facts and safe log lines, never raw pipes.
use super::{build_download_args, parse_progress_fact, ParsedProgress};
use crate::{
    diagnostics::engine_output_summary,
    errors::{AppError, AppResult, ErrorCode, ErrorPhase},
    models::{DownloadTask, EngineDiagnostics, Settings},
    process::{self, Cancellation, OutputStream, ProcessEvent, ProcessOutcome, ProcessSpec},
};
use std::path::PathBuf;

#[derive(Debug)]
pub enum DownloadEvent {
    BeforeDownload {
        stream: OutputStream,
        sequence: u64,
        observed_at: std::time::Instant,
    },
    Progress {
        stream: OutputStream,
        sequence: u64,
        fact: ParsedProgress,
    },
    Log {
        stream: OutputStream,
        sequence: u64,
        line: String,
    },
    Diagnostic(AppError),
}
pub struct EngineDownload {
    process: process::DownloadHandle,
    last_failure: Option<crate::errors::ErrorKind>,
    before_marker: String,
}
pub fn start_download(
    task: &DownloadTask,
    settings: &Settings,
    diagnostics: &EngineDiagnostics,
    cancel: Cancellation,
) -> AppResult<EngineDownload> {
    start_download_traced(task, settings, diagnostics, cancel, None)
}
pub fn start_download_traced(
    task: &DownloadTask,
    settings: &Settings,
    diagnostics: &EngineDiagnostics,
    cancel: Cancellation,
    trace: Option<crate::diagnostics::Trace>,
) -> AppResult<EngineDownload> {
    let mut args = build_download_args(task, settings, diagnostics)?;
    let marker = if diagnostics.ytdlp.before_dl_supported {
        format!("__FLUX_BEFORE_{}__%", uuid::Uuid::new_v4().simple())
    } else {
        String::new()
    };
    // A template escape makes this a literal, rather than a missing field name.
    let end = args.len() - 2;
    if !marker.is_empty() {
        args.splice(
            end..end,
            ["--print".into(), format!("before_dl:{}%", marker)],
        );
    }
    let executable = diagnostics.ytdlp.path.as_ref().ok_or_else(|| {
        AppError::new(
            ErrorCode::DependencyUnavailable,
            ErrorPhase::Download,
            "未找到 yt-dlp，请在设置中选择引擎。",
        )
    })?;
    let process = process::start_download_traced(
        ProcessSpec {
            executable: PathBuf::from(executable),
            args,
            directory: PathBuf::from(&task.output_dir),
        },
        cancel,
        trace,
    );
    Ok(EngineDownload {
        process,
        last_failure: None,
        before_marker: marker,
    })
}
impl EngineDownload {
    #[cfg(test)]
    pub(crate) fn from_process(process: process::DownloadHandle) -> Self {
        Self {
            process,
            last_failure: None,
            before_marker: String::new(),
        }
    }
    pub async fn recv(&mut self) -> Option<DownloadEvent> {
        loop {
            match self.process.recv().await? {
                ProcessEvent::Line(output) => {
                    if output.stream == OutputStream::Stdout
                        && !output.truncated
                        && !self.before_marker.is_empty()
                        && output.text == self.before_marker
                    {
                        return Some(DownloadEvent::BeforeDownload {
                            stream: output.stream,
                            sequence: output.sequence,
                            observed_at: output.observed_at,
                        });
                    }
                    if !output.truncated {
                        if let Some(fact) = parse_progress_fact(&output.text) {
                            return Some(DownloadEvent::Progress {
                                stream: output.stream,
                                sequence: output.sequence,
                                fact,
                            });
                        }
                    }
                    let line = if output.truncated {
                        // A split credential/JSON record must never be treated as a partial log/progress fact.
                        "引擎输出单行超过安全上限，已省略并继续读取。".to_owned()
                    } else {
                        engine_output_summary(&output.text).to_owned()
                    };
                    if line.trim().is_empty() {
                        continue;
                    }
                    if output.stream == OutputStream::Stderr {
                        let kind = super::failure_kind(&output.text);
                        if kind != crate::errors::ErrorKind::Unknown || self.last_failure.is_none()
                        {
                            self.last_failure = Some(kind);
                        }
                    }
                    return Some(DownloadEvent::Log {
                        stream: output.stream,
                        sequence: output.sequence,
                        line,
                    });
                }
                ProcessEvent::ReadFault(error) => {
                    return Some(DownloadEvent::Diagnostic(error));
                }
            }
        }
    }
    pub async fn finish(self) -> AppResult<ProcessOutcome> {
        let result = self.process.finish().await?;
        if !result.cancelled && !result.status.is_some_and(|status| status.success()) {
            return Err(AppError::new(
                ErrorCode::ProcessExit,
                ErrorPhase::Download,
                super::failure_summary(
                    self.last_failure
                        .unwrap_or(crate::errors::ErrorKind::Unknown),
                ),
            )
            .with_kind(
                self.last_failure
                    .unwrap_or(crate::errors::ErrorKind::Unknown),
            )
            .with_details(format!(
                "phase=download, exit={}",
                result
                    .status
                    .and_then(|s| s.code())
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "unknown".into())
            )));
        }
        Ok(result)
    }
}
