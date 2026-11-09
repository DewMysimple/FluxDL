//! Internal errors are classified at their operation boundary, never from display text.
use serde::{Deserialize, Serialize};

pub type AppResult<T> = Result<T, AppError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    InvalidInput,
    TaskNotFound,
    StateConflict,
    ShuttingDown,
    DependencyUnavailable,
    ProcessSpawn,
    ProcessTimeout,
    ProcessRead,
    ProcessWait,
    ProcessCleanup,
    ProcessExit,
    EngineProtocol,
    OutputValidation,
    PersistenceRejected,
    PersistenceUnconfirmed,
    WindowAction,
    SystemOpen,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorPhase {
    Validate,
    Startup,
    Detect,
    Probe,
    Download,
    Postprocess,
    Persist,
    Window,
    Shutdown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppError {
    pub code: ErrorCode,
    pub phase: ErrorPhase,
    pub message: String,
    pub details: Option<String>,
}
impl AppError {
    pub fn new(code: ErrorCode, phase: ErrorPhase, message: impl AsRef<str>) -> Self {
        Self {
            code,
            phase,
            message: crate::diagnostics::redact_log(message.as_ref()),
            details: None,
        }
    }
    pub fn with_details(mut self, details: impl AsRef<str>) -> Self {
        let safe = crate::diagnostics::redact_log(details.as_ref());
        if !safe.trim().is_empty() {
            self.details = Some(safe);
        }
        self
    }
    /// Adds operation context without discarding the source classification.
    pub fn with_phase(mut self, phase: ErrorPhase) -> Self {
        self.phase = phase;
        self
    }
    pub fn safe_text(&self) -> String {
        match &self.details {
            Some(details) => format!("{}\n诊断：{details}", self.message),
            None => self.message.clone(),
        }
    }
    pub fn into_ipc(self, format: Option<ErrorFormat>) -> IpcError {
        match format {
            Some(ErrorFormat::Structured) => IpcError::Structured(ErrorDto {
                version: 1,
                code: self.code,
                phase: self.phase,
                message: self.message,
                details: self.details,
            }),
            None => IpcError::Legacy(self.safe_text()),
        }
    }
}
impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.safe_text())
    }
}
impl std::error::Error for AppError {}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ErrorFormat {
    Structured,
}
#[derive(Debug, Serialize)]
pub struct ErrorDto {
    version: u8,
    code: ErrorCode,
    phase: ErrorPhase,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    details: Option<String>,
}
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum IpcError {
    Legacy(String),
    Structured(ErrorDto),
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ipc_negotiates_only_failure_shape_and_never_serializes_null_details() {
        let error = AppError::new(ErrorCode::InvalidInput, ErrorPhase::Validate, "链接无效");
        assert_eq!(
            serde_json::to_value(error.clone().into_ipc(None)).unwrap(),
            "链接无效"
        );
        let dto = serde_json::to_value(error.into_ipc(Some(ErrorFormat::Structured))).unwrap();
        assert_eq!(
            dto,
            serde_json::json!({"version":1,"code":"INVALID_INPUT","phase":"validate","message":"链接无效"})
        );
    }
}
