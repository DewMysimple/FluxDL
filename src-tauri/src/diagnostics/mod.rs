//! One local, typed source for diagnostics. No media data or raw process output.
mod environment;
mod event;
pub use environment::{BuildIdentity, Environment};
mod export;
mod redact;
mod report;
mod writer;
use crate::errors::AppError;
pub use event::*;
pub use redact::{engine_output_summary, redact_log, report_error, strip_ansi, LogRedactor};
pub use report::{Overview, Statistic};
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc, Mutex, Weak,
    },
    time::{Duration, Instant},
};
use tauri::Emitter;
#[cfg(not(debug_assertions))]
use tauri::Manager as _;

pub const MEMORY_LIMIT: usize = 2000;
pub const READ_LIMIT: usize = 20_000;
#[derive(Clone)]
pub struct Policy {
    pub max_bytes: u64,
    pub max_files: usize,
    pub max_age: Duration,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            max_bytes: 10 * 1024 * 1024,
            max_files: 5,
            max_age: Duration::from_secs(7 * 86400),
        }
    }
}
struct Shared {
    environment: Mutex<(String, Environment)>,
    versions: Mutex<std::collections::BTreeMap<String, String>>,
    #[cfg(test)]
    paused: AtomicBool,
    events: Mutex<VecDeque<Event>>,
    warning: AtomicBool,
    dropped: AtomicU64,
    critical_dropped: AtomicU64,
    generation: AtomicU64,
}
pub struct Diagnostics {
    cache: Mutex<Option<ReadCache>>,
    root: Option<PathBuf>,
    mode: &'static str,
    session: String,
    start: Instant,
    shared: Arc<Shared>,
    low: mpsc::SyncSender<Event>,
    high: mpsc::SyncSender<Event>,
    control: mpsc::Sender<mpsc::Sender<Result<(), String>>>,
    tasks: Mutex<HashMap<String, Trace>>,
    last_export: Mutex<Option<PathBuf>>,
    requests: Mutex<VecDeque<String>>,
    frontend_seen: Mutex<VecDeque<(String, Stage)>>,
    app: Option<tauri::AppHandle>,
}
#[derive(Clone)]
pub struct Trace {
    service: Weak<Diagnostics>,
    id: String,
    related: Arc<Mutex<Option<String>>>,
    task: Option<String>,
    attempt: Option<String>,
    state: Arc<Mutex<TraceState>>,
    failure: Arc<Mutex<Option<FailureClassification>>>,
    context: Arc<Mutex<TraceContext>>,
}
type TraceContext = (Option<CacheOutcome>, Option<Pipe>, String);
type FailureClassification = (crate::errors::ErrorCode, Option<crate::errors::ErrorKind>);
#[derive(Default)]
struct TraceState {
    open: HashMap<Stage, (Instant, String)>,
    done: HashSet<Stage>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub events: Vec<Event>,
    pub total: usize,
    pub next_cursor: Option<usize>,
    pub unreadable_lines: usize,
    pub truncated: bool,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Health {
    pub mode: &'static str,
    pub session_id: String,
    pub directory: Option<String>,
    pub write_restricted: bool,
    pub dropped: u64,
    pub critical_dropped: u64,
    pub generation: u64,
    pub level: &'static str,
    pub build: BuildIdentity,
    pub elapsed_since_session_start_ms: f64,
}
#[derive(Clone)]
pub struct ReadResult {
    pub events: Vec<Event>,
    pub unreadable: usize,
    pub truncated: bool,
}
type LogStamp = Vec<(PathBuf, u64, std::time::SystemTime)>;
struct ReadCache {
    generation: u64,
    stamp: LogStamp,
    created: Instant,
    data: Arc<ReadResult>,
}
impl Diagnostics {
    pub fn for_app(app: Option<&tauri::AppHandle>) -> Arc<Self> {
        #[cfg(debug_assertions)]
        let root = Some(
            std::env::var_os("FLUXDL_DIAGNOSTICS_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    Path::new(env!("CARGO_MANIFEST_DIR"))
                        .parent()
                        .expect("source root")
                        .join("dev-logs")
                }),
        );
        #[cfg(not(debug_assertions))]
        let root = app.and_then(|a| a.path().app_log_dir().ok());
        Self::open(
            root,
            if cfg!(debug_assertions) {
                "development"
            } else {
                "production"
            },
            Policy::default(),
            app.cloned(),
        )
    }
    pub fn disabled() -> Arc<Self> {
        Self::open(None, "test-disabled", Policy::default(), None)
    }
    pub fn open(
        root: Option<PathBuf>,
        mode: &'static str,
        policy: Policy,
        app: Option<tauri::AppHandle>,
    ) -> Arc<Self> {
        let (low, low_rx) = mpsc::sync_channel(1024);
        let (high, high_rx) = mpsc::sync_channel(256);
        let (control, controls) = mpsc::channel();
        let shared = Arc::new(Shared {
            environment: Mutex::new((
                uuid::Uuid::new_v4().to_string(),
                Environment {
                    build: BuildIdentity::compiled(),
                    engines: Default::default(),
                },
            )),
            versions: Mutex::new(std::collections::BTreeMap::new()),
            #[cfg(test)]
            paused: AtomicBool::new(false),
            events: Mutex::new(VecDeque::new()),
            warning: AtomicBool::new(root.is_none() && mode != "test-disabled"),
            dropped: AtomicU64::new(0),
            critical_dropped: AtomicU64::new(0),
            generation: AtomicU64::new(0),
        });
        let session = uuid::Uuid::new_v4().to_string();
        let service = Arc::new(Self {
            cache: Mutex::new(None),
            root: root.clone(),
            mode,
            session: session.clone(),
            start: Instant::now(),
            shared: shared.clone(),
            low,
            high,
            control,
            tasks: Mutex::new(HashMap::new()),
            last_export: Mutex::new(None),
            requests: Mutex::new(VecDeque::new()),
            frontend_seen: Mutex::new(VecDeque::new()),
            app,
        });
        let worker_shared = shared.clone();
        if std::thread::Builder::new()
            .name("fluxdl-diagnostics".into())
            .spawn(move || {
                writer::run(
                    root,
                    mode,
                    session,
                    policy,
                    worker_shared,
                    low_rx,
                    high_rx,
                    controls,
                )
            })
            .is_err()
        {
            shared.warning.store(true, Ordering::Relaxed);
        }
        service
            .trace(None, None, None)
            .missing(Stage::SessionEnvironment, Status::Ok);
        let t = service.trace(None, None, None);
        t.begin(Stage::Session);
        t.finish(Stage::Session, Status::Ok, None);
        service
    }
    pub fn trace(
        self: &Arc<Self>,
        id: Option<&str>,
        related: Option<&str>,
        task: Option<&str>,
    ) -> Trace {
        Trace {
            service: Arc::downgrade(self),
            id: id
                .filter(|id| valid_id(id))
                .map(str::to_owned)
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            related: Arc::new(Mutex::new(
                related.filter(|id| valid_id(id)).map(str::to_owned),
            )),
            task: task.filter(|id| valid_id(id)).map(str::to_owned),
            attempt: task.map(|_| uuid::Uuid::new_v4().to_string()),
            state: Arc::new(Mutex::new(TraceState::default())),
            failure: Arc::new(Mutex::new(None)),
            context: Arc::new(Mutex::new((
                None,
                None,
                self.shared
                    .environment
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .0
                    .clone(),
            ))),
        }
    }
    pub fn attach(&self, task: &str, trace: Trace) {
        let mut tasks = self.tasks.lock().unwrap_or_else(|e| e.into_inner());
        if tasks.len() >= MEMORY_LIMIT && !tasks.contains_key(task) {
            // Runtime correlation is bounded; files remain authoritative.
            if let Some(key) = tasks.keys().next().cloned() {
                tasks.remove(&key);
            }
            self.shared.dropped.fetch_add(1, Ordering::Relaxed);
        }
        tasks.insert(task.to_owned(), trace);
    }
    pub fn task_trace(&self, task: &str) -> Option<Trace> {
        self.tasks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(task)
            .cloned()
    }
    pub fn set_engine_versions(self: &Arc<Self>, engines: &crate::models::EngineDiagnostics) {
        let versions = export::safe_versions(&[
            (
                "yt-dlp".into(),
                engines.ytdlp.version.clone().unwrap_or_default(),
            ),
            (
                "ffmpeg".into(),
                engines.ffmpeg.version.clone().unwrap_or_default(),
            ),
            (
                "ffprobe".into(),
                engines.ffprobe.version.clone().unwrap_or_default(),
            ),
        ]);
        let mut current = self
            .shared
            .versions
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if *current != versions {
            *current = versions.clone();
            *self
                .shared
                .environment
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = (
                uuid::Uuid::new_v4().to_string(),
                Environment {
                    build: BuildIdentity::compiled(),
                    engines: versions,
                },
            );
            drop(current);
            self.trace(None, None, None)
                .missing(Stage::SessionEnvironment, Status::Ok);
            self.shared.generation.fetch_add(1, Ordering::Relaxed);
        }
    }
    pub fn request_trace(
        self: &Arc<Self>,
        id: Option<&str>,
        related: Option<&str>,
    ) -> Result<Trace, String> {
        if id.is_some_and(|v| !valid_id(v)) || related.is_some_and(|v| !valid_id(v)) {
            return Err("Trace ID必须为规范UUID。".into());
        }
        let trace = self.trace(id, related, None);
        let mut requests = self.requests.lock().unwrap_or_else(|e| e.into_inner());
        if requests.iter().any(|v| v == trace.id()) {
            return Err("该操作ID已使用，请重新发起操作。".into());
        }
        if requests.len() == MEMORY_LIMIT {
            requests.pop_front();
        }
        requests.push_back(trace.id().to_owned());
        Ok(trace)
    }
    pub fn record_frontend(
        self: &Arc<Self>,
        id: &str,
        stage: Stage,
        duration_ms: f64,
        success: bool,
    ) -> Result<(), String> {
        if !valid_id(id)
            || !matches!(stage, Stage::ParseUiRequest | Stage::EnqueueUiRequest)
            || !duration_ms.is_finite()
            || !(0.0..=600_000.0).contains(&duration_ms)
        {
            return Err("前端诊断参数无效。".into());
        }
        let events = self.shared.events.lock().unwrap_or_else(|e| e.into_inner());
        let expected = if stage == Stage::ParseUiRequest {
            Stage::ParseTotal
        } else {
            Stage::EnqueueToProgress
        };
        if !events
            .iter()
            .any(|e| e.trace_id == id && e.source == Source::Rust && e.phase == expected)
        {
            return Err("前端测量与后端操作类型不匹配，或操作已超出内存关联窗口。".into());
        }
        if events.iter().any(|e| e.trace_id == id && e.phase == stage) {
            return Ok(());
        }
        let event = events
            .iter()
            .rev()
            .find(|e| e.trace_id == id && e.source == Source::Rust)
            .cloned()
            .ok_or("找不到本会话的后端操作。")?;
        drop(events);
        let mut seen = self.frontend_seen.lock().unwrap_or_else(|e| e.into_inner());
        if seen
            .iter()
            .any(|(trace, phase)| trace == id && *phase == stage)
        {
            return Ok(());
        }
        if seen.len() == MEMORY_LIMIT {
            seen.pop_front();
        }
        seen.push_back((id.to_owned(), stage));
        let mut trace = self.trace(
            Some(id),
            event.fields.related_trace_id.as_deref(),
            event.task_id.as_deref(),
        );
        trace.attempt = event.fields.attempt_id;
        if let Some(id) = event.fields.environment_id {
            trace.context.lock().unwrap_or_else(|e| e.into_inner()).2 = id;
        }
        trace.frontend(stage, duration_ms, success);
        Ok(())
    }
    pub fn health(&self) -> Health {
        Health {
            mode: self.mode,
            session_id: self.session.clone(),
            directory: self.root.as_ref().map(|p| p.to_string_lossy().into_owned()),
            write_restricted: self.shared.warning.load(Ordering::Relaxed),
            dropped: self.shared.dropped.load(Ordering::Relaxed),
            critical_dropped: self.shared.critical_dropped.load(Ordering::Relaxed),
            generation: self.shared.generation.load(Ordering::Relaxed),
            level: "info / warn / error（固定白名单阶段）",
            build: BuildIdentity::compiled(),
            elapsed_since_session_start_ms: self.start.elapsed().as_secs_f64() * 1000.0,
        }
    }
    fn emit(&self, event: Event) {
        if !event.valid() {
            self.shared.dropped.fetch_add(1, Ordering::Relaxed);
            return;
        }
        {
            let mut events = self.shared.events.lock().unwrap_or_else(|e| e.into_inner());
            if events.len() == MEMORY_LIMIT {
                events.pop_front();
            }
            events.push_back(event.clone());
        }
        self.shared.generation.fetch_add(1, Ordering::Relaxed);
        let important = event.level == Level::Error || event.phase == Stage::DownloadTerminal;
        let result = if important {
            self.high.try_send(event)
        } else {
            self.low.try_send(event)
        };
        if result.is_err() {
            if important {
                self.shared.critical_dropped.fetch_add(1, Ordering::Relaxed);
            } else {
                self.shared.dropped.fetch_add(1, Ordering::Relaxed);
            }
            self.shared.warning.store(true, Ordering::Relaxed);
        }
    }
    pub fn flush(&self) -> Result<(), String> {
        let (tx, rx) = mpsc::channel();
        self.control.send(tx).map_err(|_| "日志线程已停止。")?;
        rx.recv_timeout(Duration::from_secs(8))
            .map_err(|_| "日志刷新超时；下载继续运行。")?
    }
    pub fn read(&self) -> ReadResult {
        self.read_snapshot().as_ref().clone()
    }
    fn read_snapshot(&self) -> Arc<ReadResult> {
        let generation = self.shared.generation.load(Ordering::Relaxed);
        let stamp = self
            .root
            .as_ref()
            .map(|root| writer::stamp(root).ok())
            .unwrap_or_else(|| Some(vec![]));
        // Only diagnostic readers acquire this lock. No download, producer or
        // SQLite state waits for it. Validate directory boundaries on every hit.
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        if let (Some(cached), Some(stamp)) = (cache.as_ref(), stamp.as_ref()) {
            if cached.generation == generation
                && cached.stamp == *stamp
                && cached.created.elapsed() < Duration::from_secs(5)
            {
                return cached.data.clone();
            }
        }
        let mut result = self
            .root
            .as_ref()
            .map(|root| writer::read(root))
            .unwrap_or(ReadResult {
                events: vec![],
                unreadable: 0,
                truncated: false,
            });
        let mut ids: HashSet<_> = result.events.iter().map(|e| e.event_id.clone()).collect();
        for event in self
            .shared
            .events
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
        {
            if ids.insert(event.event_id.clone()) {
                result.events.push(event.clone());
            }
        }
        sort_events(&mut result.events);
        if result.events.len() > READ_LIMIT {
            result.events.drain(..result.events.len() - READ_LIMIT);
            result.truncated = true;
        }
        let result = Arc::new(result);
        if let Some(stamp) = stamp {
            *cache = Some(ReadCache {
                generation,
                stamp,
                created: Instant::now(),
                data: result.clone(),
            });
        } else {
            *cache = None;
        }
        result
    }
    pub fn query(&self, filter: &Filter) -> Result<Page, String> {
        filter.validate()?;
        let read = self.read_snapshot();
        let scope = ScopedFilter::new(filter, &read.events);
        let events: Vec<_> = read
            .events
            .iter()
            .rev()
            .filter(|e| scope.matches(e))
            .collect();
        let total = events.len();
        let limit = if filter.limit == 0 { 100 } else { filter.limit };
        let events = events
            .into_iter()
            .skip(filter.cursor)
            .take(limit)
            .cloned()
            .collect::<Vec<_>>();
        Ok(Page {
            next_cursor: (filter.cursor + events.len() < total)
                .then_some(filter.cursor + events.len()),
            total,
            events,
            unreadable_lines: read.unreadable,
            truncated: read.truncated,
        })
    }
    pub fn overview(&self, filter: &Filter) -> Result<Overview, String> {
        filter.validate()?;
        let read = self.read_snapshot();
        let scope = ScopedFilter::new(filter, &read.events);
        Ok(report::overview_with_context(
            read.events
                .iter()
                .filter(|e| scope.matches(e))
                .cloned()
                .collect(),
            &self.session,
            read.unreadable,
            read.truncated,
            &read.events,
        ))
    }
    pub fn directory(&self) -> Result<PathBuf, String> {
        let root = self.root.as_ref().ok_or("日志目录不可用。")?;
        writer::check_directory(root).map_err(|_| "日志目录不可用或包含链接；未打开。".into())
    }
    pub fn export(
        &self,
        destination: &Path,
        filter: &Filter,
        versions: &[(String, String)],
    ) -> Result<(), String> {
        filter.validate()?;
        // Explicit export always re-decodes current files; it never trusts a hit.
        *self.cache.lock().unwrap_or_else(|e| e.into_inner()) = None;
        let read = self.read();
        let scope = ScopedFilter::new(filter, &read.events);
        let mut events: Vec<_> = read
            .events
            .iter()
            .filter(|e| scope.matches(e))
            .cloned()
            .collect();
        let environments: HashSet<_> = events
            .iter()
            .filter_map(|e| e.fields.environment_id.clone())
            .collect();
        let ids: HashSet<_> = events.iter().map(|e| e.event_id.clone()).collect();
        events.extend(
            read.events
                .iter()
                .filter(|e| {
                    e.fields.environment.is_some()
                        && e.fields
                            .environment_id
                            .as_ref()
                            .is_some_and(|id| environments.contains(id))
                        && !ids.contains(&e.event_id)
                })
                .cloned(),
        );
        sort_events(&mut events);
        export::bundle(
            destination,
            &events,
            &self.health(),
            versions,
            read.unreadable,
            read.truncated,
        )?;
        *self.last_export.lock().unwrap_or_else(|e| e.into_inner()) =
            Some(destination.to_path_buf());
        Ok(())
    }
    pub fn last_export(&self) -> Option<PathBuf> {
        self.last_export
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
    pub fn notify(&self) {
        if let Some(app) = &self.app {
            let _ = app.emit("diagnostics-changed", self.health().generation);
        }
    }
}
impl Trace {
    pub fn fault(&self, stage: Stage, error: &AppError) {
        if !self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .done
            .insert(stage)
        {
            return;
        }
        self.record(
            stage,
            Status::Failed,
            None,
            Some(error),
            uuid::Uuid::new_v4().to_string(),
            Source::Rust,
        );
    }
    pub fn is_open(&self, stage: Stage) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .open
            .contains_key(&stage)
    }
    pub fn failure(&self, error: &AppError) {
        *self.failure.lock().unwrap_or_else(|e| e.into_inner()) = Some((error.code, error.kind));
    }
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn with_task(&self, task: &str) -> Self {
        let mut t = self.clone();
        t.task = valid_id(task).then(|| task.to_owned());
        t.attempt = Some(uuid::Uuid::new_v4().to_string());
        t
    }
    pub fn bind_related(&self, id: &str) {
        if valid_id(id) {
            *self.related.lock().unwrap_or_else(|e| e.into_inner()) = Some(id.to_owned());
        }
    }
    pub fn refresh_environment(&self) {
        if let Some(service) = self.service.upgrade() {
            self.context.lock().unwrap_or_else(|e| e.into_inner()).2 = service
                .shared
                .environment
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .0
                .clone();
        }
    }
    pub fn first_output(&self, pipe: Pipe) {
        let mut context = self.context.lock().unwrap_or_else(|e| e.into_inner());
        if context.1.is_some() {
            return;
        }
        context.1 = Some(pipe);
        drop(context);
        self.finish(Stage::DownloadFirstOutput, Status::Ok, None);
    }
    pub fn cache_outcome(&self, outcome: CacheOutcome) {
        self.context.lock().unwrap_or_else(|e| e.into_inner()).0 = Some(outcome);
        self.missing(Stage::PreviewCacheDecision, Status::Ok);
    }
    pub fn begin(&self, stage: Stage) {
        self.begin_at(stage, Instant::now());
    }
    pub fn begin_at(&self, stage: Stage, at: Instant) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.done.contains(&stage) || state.open.contains_key(&stage) {
            return;
        }
        let span = uuid::Uuid::new_v4().to_string();
        state.open.insert(stage, (at, span.clone()));
        self.record(stage, Status::Started, None, None, span, Source::Rust);
    }
    pub fn finish(&self, stage: Stage, status: Status, error: Option<&AppError>) {
        self.finish_at(stage, status, error, Instant::now());
    }
    pub fn finish_at(&self, stage: Stage, status: Status, error: Option<&AppError>, at: Instant) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.done.contains(&stage) {
            return;
        }
        if let Some((start, span)) = state.open.remove(&stage) {
            state.done.insert(stage);
            self.record(
                stage,
                status,
                at.checked_duration_since(start)
                    .map(|d| d.as_secs_f64() * 1000.0),
                error,
                span,
                Source::Rust,
            );
        }
    }
    pub fn result<T>(&self, stage: Stage, result: &Result<T, AppError>) {
        self.finish(
            stage,
            if result.is_ok() {
                Status::Ok
            } else {
                Status::Failed
            },
            result.as_ref().err(),
        );
    }
    pub fn missing(&self, stage: Stage, status: Status) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.done.contains(&stage) {
            return;
        }
        let span = state
            .open
            .remove(&stage)
            .map(|(_, s)| s)
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        state.done.insert(stage);
        self.record(stage, status, None, None, span, Source::Rust);
    }
    pub fn frontend(&self, stage: Stage, ms: f64, success: bool) {
        if !matches!(stage, Stage::ParseUiRequest | Stage::EnqueueUiRequest)
            || !ms.is_finite()
            || !(0.0..=600_000.0).contains(&ms)
        {
            return;
        }
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if !state.done.insert(stage) {
            return;
        }
        self.record(
            stage,
            if success { Status::Ok } else { Status::Failed },
            Some(ms),
            None,
            uuid::Uuid::new_v4().to_string(),
            Source::Frontend,
        );
    }
    fn record(
        &self,
        phase: Stage,
        status: Status,
        duration_ms: Option<f64>,
        error: Option<&AppError>,
        span_id: String,
        source: Source,
    ) {
        let Some(service) = self.service.upgrade() else {
            return;
        };
        let error_code = error.map(|e| e.code).or_else(|| {
            if status == Status::Failed {
                self.failure
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .map(|(code, _)| code)
            } else {
                None
            }
        });
        let error_kind = error.and_then(|e| e.kind).or_else(|| {
            if status == Status::Failed {
                self.failure
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .and_then(|(_, kind)| kind)
            } else {
                None
            }
        });
        let context = self
            .context
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        service.emit(Event {
            schema_version: 1,
            event_id: uuid::Uuid::new_v4().to_string(),
            session_id: service.session.clone(),
            trace_id: self.id.clone(),
            span_id,
            parent_span_id: None,
            task_id: self.task.clone(),
            timestamp_utc: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            elapsed_since_session_start_ms: service.start.elapsed().as_secs_f64() * 1000.0,
            level: if status == Status::Failed {
                Level::Error
            } else if matches!(status, Status::Interrupted | Status::Unobserved) {
                Level::Warn
            } else {
                Level::Info
            },
            source,
            event: if matches!(phase, Stage::PersistenceIssue | Stage::DownloadProcessIssue) {
                EventName::FaultObserved
            } else if status == Status::Started {
                EventName::StageStarted
            } else {
                EventName::StageFinished
            },
            phase,
            status,
            duration_ms,
            fields: Fields {
                environment_id: Some(context.2.clone()),
                environment: if phase == Stage::SessionEnvironment {
                    Some(
                        service
                            .shared
                            .environment
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .1
                            .clone(),
                    )
                } else {
                    None
                },
                cache_outcome: if phase == Stage::PreviewCacheDecision {
                    context.0
                } else {
                    None
                },
                output_stream: if phase == Stage::DownloadFirstOutput {
                    context.1
                } else {
                    None
                },
                related_trace_id: self
                    .related
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone(),
                attempt_id: self.attempt.clone(),
                error_code,
                error_kind,
            },
        });
    }
}

#[cfg(test)]
mod tests;

fn sort_events(events: &mut [Event]) {
    let mut first = HashMap::<String, String>::new();
    for event in events.iter() {
        first
            .entry(event.session_id.clone())
            .and_modify(|t| {
                if *t > event.timestamp_utc {
                    *t = event.timestamp_utc.clone();
                }
            })
            .or_insert_with(|| event.timestamp_utc.clone());
    }
    events.sort_by(|a, b| {
        first[&a.session_id]
            .cmp(&first[&b.session_id])
            .then(a.session_id.cmp(&b.session_id))
            .then(
                a.elapsed_since_session_start_ms
                    .total_cmp(&b.elapsed_since_session_start_ms),
            )
            .then(a.event_id.cmp(&b.event_id))
    });
}

struct ScopedFilter {
    filter: Filter,
    traces: Option<HashSet<String>>,
}
impl ScopedFilter {
    fn new(filter: &Filter, events: &[Event]) -> Self {
        let traces = filter.task_id.as_ref().map(|task| {
            events
                .iter()
                .filter(|e| e.task_id.as_ref() == Some(task))
                .map(|e| e.trace_id.clone())
                .collect()
        });
        let mut filter = filter.clone();
        filter.task_id = None;
        Self { filter, traces }
    }
    fn matches(&self, event: &Event) -> bool {
        self.traces
            .as_ref()
            .is_none_or(|t| t.contains(&event.trace_id))
            && self.filter.matches(event)
    }
}
