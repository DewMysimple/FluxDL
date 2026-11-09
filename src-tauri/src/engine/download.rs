//! yt-dlp output adapter. Manager receives facts and safe log lines, never raw pipes.
use super::{build_download_args, parse_progress_fact, process_failure, ParsedProgress};
use crate::{
    diagnostics::LogRedactor,
    errors::{AppError, AppResult, ErrorCode, ErrorPhase},
    models::{DownloadTask, EngineDiagnostics, Settings},
    process::{self, Cancellation, OutputStream, ProcessEvent, ProcessOutcome, ProcessSpec},
};
use std::{collections::HashMap, path::PathBuf};

#[derive(Debug)]
pub enum DownloadEvent {
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
    redactors: HashMap<OutputStream, LogRedactor>,
    errors: Vec<String>,
}
pub fn start_download(
    task: &DownloadTask,
    settings: &Settings,
    diagnostics: &EngineDiagnostics,
    cancel: Cancellation,
) -> AppResult<EngineDownload> {
    let args = build_download_args(task, settings, diagnostics)?;
    let executable = diagnostics.ytdlp.path.as_ref().ok_or_else(|| {
        AppError::new(
            ErrorCode::DependencyUnavailable,
            ErrorPhase::Download,
            "未找到 yt-dlp，请在设置中选择引擎。",
        )
    })?;
    let process = process::start_download(
        ProcessSpec {
            executable: PathBuf::from(executable),
            args,
            directory: PathBuf::from(&task.output_dir),
        },
        cancel,
    );
    Ok(EngineDownload {
        process,
        redactors: HashMap::new(),
        errors: Vec::new(),
    })
}
impl EngineDownload {
    #[cfg(test)]
    pub(crate) fn from_process(process: process::DownloadHandle) -> Self {
        Self {
            process,
            redactors: HashMap::new(),
            errors: Vec::new(),
        }
    }
    pub async fn recv(&mut self) -> Option<DownloadEvent> {
        loop {
            match self.process.recv().await? {
                ProcessEvent::Line(output) => {
                    if !output.truncated {
                        if let Some(fact) = parse_progress_fact(&output.text) {
                            return Some(DownloadEvent::Progress {
                                stream: output.stream,
                                sequence: output.sequence,
                                fact,
                            });
                        }
                    }
                    let redactor = self.redactors.entry(output.stream).or_default();
                    let line = if output.truncated {
                        // A split credential/JSON record must never be treated as a partial log/progress fact.
                        redactor.omit_line().to_owned()
                    } else {
                        redactor.redact(&output.text)
                    };
                    if line.trim().is_empty() {
                        continue;
                    }
                    if output.stream == OutputStream::Stderr {
                        self.errors.push(line.clone());
                        if self.errors.len() > 12 {
                            self.errors.remove(0);
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
            return Err(process_failure(
                ErrorPhase::Download,
                &self.errors.join("\n"),
                result.status.and_then(|status| status.code()),
            ));
        }
        Ok(result)
    }
}
