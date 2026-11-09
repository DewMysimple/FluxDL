use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub download_dir: String,
    pub engine_path: String,
    pub ffmpeg_path: String,
    pub default_mode: String,
    pub quality: String,
    pub theme: String,
    pub density: String,
    pub reduced_motion: bool,
    pub close_behavior: String,
    pub completion_action: String,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            download_dir: dirs::download_dir()
                .unwrap_or_else(std::env::temp_dir)
                .join("FluxDL")
                .to_string_lossy()
                .into(),
            engine_path: String::new(),
            ffmpeg_path: String::new(),
            default_mode: "video".into(),
            quality: "1080".into(),
            theme: "dark".into(),
            density: "comfortable".into(),
            reduced_motion: false,
            close_behavior: "tray".into(),
            completion_action: "none".into(),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EngineInfo {
    #[serde(default)]
    pub before_dl_supported: bool,
    pub path: Option<String>,
    pub version: Option<String>,
    pub source: String,
    pub available: bool,
    pub message: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EngineDiagnostics {
    pub ytdlp: EngineInfo,
    pub ffmpeg: EngineInfo,
    pub ffprobe: EngineInfo,
    pub runtime: EngineInfo,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormatOption {
    pub format_id: String,
    pub ext: String,
    pub label: String,
    pub height: Option<u32>,
    pub has_video: bool,
    pub has_audio: bool,
    pub filesize: Option<u64>,
    pub requires_ffmpeg: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trusted_preview_ref: Option<String>,
    pub url: String,
    pub title: String,
    pub media_id: String,
    pub source: String,
    pub duration: Option<f64>,
    pub thumbnail: Option<String>,
    pub is_playlist: bool,
    pub formats: Vec<FormatOption>,
    pub warnings: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnqueueInput {
    pub media: MediaInfo,
    pub mode: String,
    pub quality: String,
    pub format_id: Option<String>,
    pub output_dir: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    Queued,
    Resolving,
    Downloading,
    Postprocessing,
    Completed,
    Failed,
    Cancelling,
    Cancelled,
    Interrupted,
}
impl TaskStatus {
    pub fn active(self) -> bool {
        matches!(
            self,
            Self::Queued
                | Self::Resolving
                | Self::Downloading
                | Self::Postprocessing
                | Self::Cancelling
        )
    }
    pub fn running(self) -> bool {
        self.active() && self != Self::Queued
    }
    pub fn retryable(self) -> bool {
        matches!(self, Self::Failed | Self::Cancelled | Self::Interrupted)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadTask {
    pub id: String,
    pub media: MediaInfo,
    pub mode: String,
    pub quality: String,
    pub format_id: Option<String>,
    pub output_dir: String,
    pub status: TaskStatus,
    pub progress: Option<f64>,
    pub speed: Option<f64>,
    pub eta: Option<f64>,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub final_path: Option<String>,
    pub error: Option<String>,
    pub logs: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
    pub file_exists: bool,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSnapshot {
    pub session_id: String,
    pub revision: u64,
    pub settings: Settings,
    pub engines: EngineDiagnostics,
    pub tasks: Vec<DownloadTask>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskDetails {
    pub session_id: String,
    pub revision: u64,
    pub task: DownloadTask,
}
