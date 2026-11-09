//! No arbitrary text enters the diagnostic store. All contextual fields are typed.
use crate::errors::ErrorCode;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Session,
    SessionEnvironment,
    ParseUiRequest,
    ParseIpcDispatch,
    ParseValidate,
    ParseEngineSpawn,
    ParseMetadataExtract,
    ParseResponse,
    ParseTotal,
    EnqueueUiRequest,
    EnqueueValidate,
    EnqueueMetadataRecheck,
    EnqueuePersist,
    EnqueueDbQueue,
    EnqueueDbWrite,
    EnqueueToProgress,
    QueueWait,
    DownloadEngineSpawn,
    DownloadFirstProgress,
    DownloadFirstOutput,
    DownloadBeforeDl,
    DownloadBeforeDlToProgress,
    DownloadProcessExit,
    PreviewCacheDecision,
    EnqueueTrustedValidate,
    DownloadActive,
    DownloadPostprocess,
    DownloadTerminal,
    PersistenceIssue,
    DownloadProcessIssue,
    DownloadTotal,
}
impl Stage {
    pub fn label(self) -> &'static str {
        match self {
            Self::SessionEnvironment => "本次会话构建与引擎版本",
            Self::Session => "应用会话",
            Self::ParseUiRequest => "解析到预览（前端时钟）",
            Self::ParseIpcDispatch => "解析命令分派",
            Self::ParseValidate => "解析参数校验",
            Self::ParseEngineSpawn => "解析进程启动",
            Self::ParseMetadataExtract => "元数据提取（含引擎及网络）",
            Self::ParseResponse => "解析结果封装",
            Self::ParseTotal => "首次解析（后端）",
            Self::EnqueueUiRequest => "加入队列响应（前端时钟）",
            Self::EnqueueValidate => "入队本地校验",
            Self::EnqueueMetadataRecheck => "二次安全复核（含引擎及网络）",
            Self::EnqueuePersist => "入队持久化（含DB排队）",
            Self::EnqueueDbQueue => "SQLite写入队列等待",
            Self::EnqueueDbWrite => "SQLite事务执行（含锁等待）",
            Self::EnqueueToProgress => "入队到首次有效进度",
            Self::QueueWait => "串行队列等待",
            Self::DownloadEngineSpawn => "下载进程启动",
            Self::DownloadFirstOutput => "等待首次引擎输出（非网络首字节）",
            Self::DownloadBeforeDl => "启动至下载前标记",
            Self::DownloadBeforeDlToProgress => "下载前标记至首次有效进度",
            Self::DownloadProcessExit => "下载进程运行至回收",
            Self::PreviewCacheDecision => "可信预览缓存决策",
            Self::EnqueueTrustedValidate => "后端可信预览安全校验",
            Self::DownloadFirstProgress => "进程启动到首次有效进度",
            Self::DownloadActive => "下载阶段",
            Self::DownloadPostprocess => "后处理",
            Self::DownloadTerminal => "终态持久化",
            Self::PersistenceIssue => "持久化异常（时点）",
            Self::DownloadProcessIssue => "下载进程异常（时点）",
            Self::DownloadTotal => "下载尝试总用时",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Started,
    Ok,
    Failed,
    Cancelled,
    Unobserved,
    NotApplicable,
    Interrupted,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Info,
    Warn,
    Error,
    Debug,
    Trace,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheOutcome {
    Hit,
    Missing,
    Expired,
    ConfigChanged,
    SourceMismatch,
    ModeMismatch,
    FormatMismatch,
    Disabled,
    Duplicate,
    TooLarge,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Pipe {
    Stdout,
    Stderr,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Fields {
    pub environment_id: Option<String>,
    pub environment: Option<super::Environment>,
    pub cache_outcome: Option<CacheOutcome>,
    pub output_stream: Option<Pipe>,
    pub related_trace_id: Option<String>,
    pub attempt_id: Option<String>,
    pub error_code: Option<ErrorCode>,
    pub error_kind: Option<crate::errors::ErrorKind>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Event {
    pub schema_version: u8,
    pub event_id: String,
    pub session_id: String,
    pub trace_id: String,
    pub span_id: String,
    pub parent_span_id: Option<String>,
    pub task_id: Option<String>,
    pub timestamp_utc: String,
    pub elapsed_since_session_start_ms: f64,
    pub level: Level,
    pub source: Source,
    pub event: EventName,
    pub phase: Stage,
    pub status: Status,
    pub duration_ms: Option<f64>,
    pub fields: Fields,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Source {
    #[serde(rename = "rust")]
    Rust,
    #[serde(rename = "frontend")]
    Frontend,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventName {
    StageStarted,
    StageFinished,
    FaultObserved,
}
pub fn valid_id(value: &str) -> bool {
    value.len() == 36 && uuid::Uuid::parse_str(value).is_ok_and(|id| id.to_string() == value)
}
impl Event {
    pub fn valid(&self) -> bool {
        self.schema_version == 1
            && self
                .fields
                .environment_id
                .as_ref()
                .is_none_or(|id| valid_id(id))
            && self.fields.environment.as_ref().is_none_or(|e| e.valid())
            && [
                &self.event_id,
                &self.session_id,
                &self.trace_id,
                &self.span_id,
            ]
            .iter()
            .all(|id| valid_id(id))
            && [
                self.task_id.as_ref(),
                self.parent_span_id.as_ref(),
                self.fields.related_trace_id.as_ref(),
                self.fields.attempt_id.as_ref(),
            ]
            .iter()
            .flatten()
            .all(|id| valid_id(id))
            && chrono::DateTime::parse_from_rfc3339(&self.timestamp_utc).is_ok()
            && self.elapsed_since_session_start_ms.is_finite()
            && self.elapsed_since_session_start_ms >= 0.0
            && self
                .duration_ms
                .is_none_or(|v| v.is_finite() && (0.0..=604_800_000.0).contains(&v))
    }
}
#[derive(Clone, Default, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct Filter {
    pub session_id: Option<String>,
    pub trace_id: Option<String>,
    pub task_id: Option<String>,
    pub level: Option<Level>,
    pub source: Option<Source>,
    pub phase: Option<Stage>,
    pub search: String,
    pub from_utc: Option<String>,
    pub to_utc: Option<String>,
    pub cursor: usize,
    pub limit: usize,
}
impl Filter {
    pub fn validate(&self) -> Result<(), String> {
        if self.limit > 200
            || self.cursor > 100_000
            || self.search.len() > 120
            || [
                self.session_id.as_ref(),
                self.trace_id.as_ref(),
                self.task_id.as_ref(),
            ]
            .iter()
            .flatten()
            .any(|id| !valid_id(id))
            || [self.from_utc.as_ref(), self.to_utc.as_ref()]
                .iter()
                .flatten()
                .any(|s| chrono::DateTime::parse_from_rfc3339(s).is_err())
        {
            return Err("诊断筛选参数无效；每页最多200条。".into());
        }
        if let (Some(a), Some(b)) = (&self.from_utc, &self.to_utc) {
            if chrono::DateTime::parse_from_rfc3339(a).unwrap()
                > chrono::DateTime::parse_from_rfc3339(b).unwrap()
            {
                return Err("诊断时间范围的开始晚于结束。".into());
            }
        }
        Ok(())
    }
    pub fn matches(&self, e: &Event) -> bool {
        let t = chrono::DateTime::parse_from_rfc3339(&e.timestamp_utc).ok();
        self.session_id
            .as_ref()
            .is_none_or(|id| id == &e.session_id)
            && self.trace_id.as_ref().is_none_or(|id| id == &e.trace_id)
            && self
                .task_id
                .as_ref()
                .is_none_or(|id| e.task_id.as_ref() == Some(id))
            && self.level.is_none_or(|v| v == e.level)
            && self.source.is_none_or(|v| v == e.source)
            && self.phase.is_none_or(|v| v == e.phase)
            && self
                .from_utc
                .as_ref()
                .is_none_or(|v| t >= chrono::DateTime::parse_from_rfc3339(v).ok())
            && self
                .to_utc
                .as_ref()
                .is_none_or(|v| t <= chrono::DateTime::parse_from_rfc3339(v).ok())
            && (self.search.is_empty()
                || format!(
                    "{} {:?} {:?} {} {} {} {:?} {:?}",
                    serde_json::to_string(&e.phase).unwrap_or_default(),
                    e.fields.error_code,
                    e.fields.error_kind,
                    e.phase.label(),
                    e.trace_id,
                    e.task_id.as_deref().unwrap_or(""),
                    e.event,
                    e.source
                )
                .to_lowercase()
                .contains(&self.search.to_lowercase()))
    }
}
