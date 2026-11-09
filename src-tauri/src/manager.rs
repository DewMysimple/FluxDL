#[cfg(test)]
use crate::events::AppEvent;
mod files;
use crate::engine::trusted_preview::{Fingerprint, Selection, TrustedPreviews};
use crate::{
    diagnostics::{Diagnostics, Stage, Status, Trace},
    engine::{self, DownloadEvent},
    errors::{AppError, AppResult, ErrorCode, ErrorPhase},
    events::{AppChange, EventJournal, FileAvailability, APP_STATE_EVENT},
    models::*,
    persistence::{ApplyOutcome, Mutation, PersistenceWriter, WriteFailure},
    process::Cancellation,
    transitions::{self, RuntimeChange, RuntimeTracker},
};
use files::{open_folder, prepare_directory, validate_output};
pub use files::{validate_final_path, writable_directory};
use futures_util::{stream, StreamExt};
use std::collections::{HashMap, HashSet};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, OnceLock, Weak,
    },
    time::{Duration, Instant},
};
use tauri::Emitter;
use tokio::sync::{Mutex, Notify};

const LOG_LIMIT: usize = 100;
#[derive(Clone)]
struct PendingTerminal {
    seq: i64,
    task: DownloadTask,
    attempt: u64,
}
struct Inner {
    settings: Settings,
    engines: EngineDiagnostics,
    tasks: Vec<DownloadTask>,
    next_seq: i64,
    file_refresh: HashMap<String, uuid::Uuid>,
    versions: HashMap<String, i64>,
    attempts: HashMap<String, u64>,
    pending_tasks: HashSet<String>,
    pending_terminal: HashMap<String, PendingTerminal>,
    cancel_requested: HashSet<String>,
    dirty: HashSet<String>,
    persistence_error: Option<String>,
    process_error: Option<String>,
    storage_diagnostics: Vec<String>,
    events: EventJournal,
}
impl Inner {
    fn next_seq(&mut self) -> AppResult<i64> {
        self.next_seq = self.next_seq.checked_add(1).ok_or_else(|| {
            AppError::new(
                ErrorCode::PersistenceRejected,
                ErrorPhase::Persist,
                "持久化版本号已达到安全上限。",
            )
        })?;
        Ok(self.next_seq)
    }
    fn apply_availability(
        &mut self,
        id: &str,
        path: &Option<String>,
        version: i64,
        token: uuid::Uuid,
        exists: bool,
    ) -> bool {
        if self.versions.get(id).copied().unwrap_or(0) != version
            || self.file_refresh.get(id) != Some(&token)
        {
            return false;
        }
        if let Some(task) = self
            .tasks
            .iter_mut()
            .find(|task| task.id == id && &task.final_path == path)
        {
            let changed = task.file_exists != exists;
            task.file_exists = exists;
            changed
        } else {
            false
        }
    }
    fn diagnostics(&self) -> EngineDiagnostics {
        let mut engines = self.engines.clone();
        for warning in &self.storage_diagnostics {
            if !engines.warnings.contains(warning) {
                engines.warnings.push(warning.clone());
            }
        }
        if let Some(error) = &self.persistence_error {
            engines.warnings.push(error.clone());
        }
        if let Some(error) = &self.process_error {
            engines.warnings.push(error.clone());
        }
        if let Some(error) = &self.events.error {
            engines.warnings.push(error.clone());
        }
        engines
    }
    fn flush_events(&mut self) {
        self.events.flush_progress(&self.tasks);
    }
    fn capture_task(&mut self, id: &str) {
        if let Some(index) = self.tasks.iter().position(|task| task.id == id) {
            self.events.upsert(self.tasks[index].clone(), index);
        }
    }
    fn set_persistence_error(&mut self, error: Option<String>) {
        let previous = self.diagnostics();
        self.persistence_error = error;
        self.capture_diagnostics(previous);
    }
    fn set_process_error(&mut self, error: Option<String>) {
        let previous = self.diagnostics();
        self.process_error = error;
        self.capture_diagnostics(previous);
    }
    fn capture_diagnostics(&mut self, previous: EngineDiagnostics) {
        let current = self.diagnostics();
        if current != previous {
            self.events
                .capture(AppChange::DiagnosticsUpdated { engines: current });
        }
    }
    fn stamped_snapshot(&self) -> AppSnapshot {
        AppSnapshot {
            session_id: self.events.session_id.clone(),
            revision: self.events.revision,
            settings: self.settings.clone(),
            engines: self.diagnostics(),
            tasks: self.tasks.clone(),
        }
    }
    fn accepts_progress(&self, id: &str, attempt: u64) -> bool {
        self.tasks
            .iter()
            .find(|task| task.id == id)
            .is_some_and(|task| {
                transitions::admits_runtime(
                    self.attempts.get(id).copied(),
                    attempt,
                    &task.status,
                    self.pending_terminal.contains_key(id),
                    self.cancel_requested.contains(id),
                )
            })
            && !self.pending_tasks.contains(id)
    }
}

struct Control {
    id: String,
    attempt: u64,
    cancel: Cancellation,
}

pub struct Manager {
    pub diagnostics: Arc<Diagnostics>,
    previews: std::sync::Mutex<TrustedPreviews>,
    preview_generation: std::sync::atomic::AtomicU64,
    preview_cache_enabled: AtomicBool,
    self_ref: OnceLock<Weak<Manager>>,
    inner: Mutex<Inner>,
    writer: PersistenceWriter,
    critical: Mutex<()>,
    publisher: Mutex<()>,
    #[cfg(test)]
    test_events: Mutex<Vec<AppEvent>>,
    shutdown_gate: Arc<Mutex<()>>,
    stop_worker: AtomicBool,
    progress_finished: AtomicBool,
    progress_stopped: Notify,
    progress_wake: Notify,
    event_finished: AtomicBool,
    event_stopped: Notify,
    event_wake: Notify,
    state_changed: Notify,
    active: Mutex<Option<Arc<Control>>>,
    wake: Notify,
    shutting_down: AtomicBool,
    worker_stopped: Notify,
    worker_finished: AtomicBool,
    resource_dir: Option<PathBuf>,
    app: Option<tauri::AppHandle>,
}

impl Manager {
    pub async fn init(
        data_dir: PathBuf,
        resource_dir: Option<PathBuf>,
        app: Option<tauri::AppHandle>,
    ) -> AppResult<Arc<Self>> {
        let diagnostics = Diagnostics::for_app(app.as_ref());
        Self::init_with_diagnostics(data_dir, resource_dir, app, diagnostics).await
    }
    /// Native integration/benchmark embedding; never exposed over IPC.
    pub async fn init_with_diagnostics(
        data_dir: PathBuf,
        resource_dir: Option<PathBuf>,
        app: Option<tauri::AppHandle>,
        diagnostics: Arc<Diagnostics>,
    ) -> AppResult<Arc<Self>> {
        let (writer, loaded) = PersistenceWriter::open(data_dir).await?;
        let engines = engine::detect(&loaded.settings, resource_dir.as_deref()).await;
        diagnostics.set_engine_versions(&engines);
        let mut storage_diagnostics = loaded.diagnostics;
        if let Some(warning) = loaded.recovery_warning {
            storage_diagnostics.push(warning);
        }
        let manager = Arc::new(Self {
            diagnostics,
            previews: std::sync::Mutex::new(TrustedPreviews::default()),
            preview_generation: std::sync::atomic::AtomicU64::new(0),
            preview_cache_enabled: AtomicBool::new(true),
            self_ref: OnceLock::new(),
            inner: Mutex::new(Inner {
                settings: loaded.settings,
                engines,
                tasks: loaded.tasks,
                next_seq: loaded.max_mutation_seq,
                file_refresh: HashMap::new(),
                versions: HashMap::new(),
                attempts: HashMap::new(),
                pending_tasks: HashSet::new(),
                pending_terminal: HashMap::new(),
                cancel_requested: HashSet::new(),
                dirty: HashSet::new(),
                persistence_error: None,
                process_error: None,
                storage_diagnostics,
                events: EventJournal::new(),
            }),
            writer,
            critical: Mutex::new(()),
            publisher: Mutex::new(()),
            #[cfg(test)]
            test_events: Mutex::new(Vec::new()),
            shutdown_gate: Arc::new(Mutex::new(())),
            stop_worker: AtomicBool::new(false),
            progress_finished: AtomicBool::new(true),
            progress_stopped: Notify::new(),
            progress_wake: Notify::new(),
            event_finished: AtomicBool::new(true),
            event_stopped: Notify::new(),
            event_wake: Notify::new(),
            state_changed: Notify::new(),
            active: Mutex::new(None),
            wake: Notify::new(),
            shutting_down: AtomicBool::new(false),
            worker_stopped: Notify::new(),
            worker_finished: AtomicBool::new(true),
            resource_dir,
            app,
        });
        let _ = manager.self_ref.set(Arc::downgrade(&manager));
        manager.ensure_workers();
        Ok(manager)
    }

    async fn run_owned<T, F, Future>(&self, operation: F) -> AppResult<T>
    where
        T: Send + 'static,
        F: FnOnce(Arc<Self>) -> Future + Send + 'static,
        Future: std::future::Future<Output = AppResult<T>> + Send + 'static,
    {
        let manager = self
            .self_ref
            .get()
            .and_then(Weak::upgrade)
            .ok_or_else(|| state_error("后端实例已停止。"))?;
        tauri::async_runtime::spawn(operation(manager))
            .await
            .map_err(|error| state_error("后端操作未完成。").with_details(error.to_string()))?
    }
    fn ensure_workers(self: &Arc<Self>) {
        if self
            .worker_finished
            .compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            let manager = self.clone();
            tauri::async_runtime::spawn(async move {
                manager.worker().await;
            });
        }
        if self
            .progress_finished
            .compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            let manager = self.clone();
            tauri::async_runtime::spawn(async move {
                manager.progress_worker().await;
            });
        }
        if self
            .event_finished
            .compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            let manager = self.clone();
            tauri::async_runtime::spawn(async move {
                manager.event_worker(Duration::from_secs(6)).await;
            });
        }
        self.wake.notify_one();
        self.progress_wake.notify_one();
        self.event_wake.notify_one();
    }
    #[cfg(test)]
    async fn cached_snapshot(&self) -> AppSnapshot {
        self.inner.lock().await.stamped_snapshot()
    }
    pub async fn snapshot(&self) -> AppSnapshot {
        let (checks, token) = {
            let mut inner = self.inner.lock().await;
            let token = uuid::Uuid::new_v4();
            let checks = inner
                .tasks
                .iter()
                .map(|task| {
                    (
                        task.id.clone(),
                        task.final_path.clone(),
                        *inner.versions.get(&task.id).unwrap_or(&0),
                    )
                })
                .collect::<Vec<_>>();
            for (id, _, _) in &checks {
                inner.file_refresh.insert(id.clone(), token);
            }
            (checks, token)
        };
        let results = stream::iter(checks)
            .map(|(id, path, version)| async move {
                let exists = match &path {
                    Some(path) => tokio::fs::metadata(path).await.is_ok_and(|m| m.is_file()),
                    None => false,
                };
                (id, path, version, exists)
            })
            .buffered(16)
            .collect::<Vec<_>>()
            .await;
        let snapshot = {
            let mut inner = self.inner.lock().await;
            let mut files = Vec::new();
            for (id, path, version, exists) in results {
                if inner.apply_availability(&id, &path, version, token, exists) {
                    files.push(FileAvailability {
                        task_id: id,
                        file_exists: exists,
                    });
                }
            }
            if !files.is_empty() {
                inner
                    .events
                    .capture(AppChange::FileAvailabilityUpdated { files });
            }
            inner.flush_events();
            inner.stamped_snapshot()
        };
        self.emit().await;
        snapshot
    }
    async fn emit(&self) {
        let _publisher = self.publisher.lock().await;
        loop {
            let pending = self.inner.lock().await.events.take_pending();
            if pending.is_empty() {
                break;
            }
            #[cfg(test)]
            {
                let mut events = self.test_events.lock().await;
                events.extend(pending.clone());
                if events.len() > 4096 {
                    let count = events.len() - 4096;
                    events.drain(..count);
                }
            }
            if let Some(app) = &self.app {
                for event in pending {
                    if let Err(error) = app.emit(APP_STATE_EVENT, event) {
                        crate::diagnostics::report_error(
                            &AppError::new(
                                ErrorCode::WindowAction,
                                ErrorPhase::Window,
                                "任务同步事件发送失败。",
                            )
                            .with_details(error.to_string()),
                        );
                        let mut inner = self.inner.lock().await;
                        let previous = inner.diagnostics();
                        inner.events.error =
                            Some("任务同步事件发送失败，请重新聚焦窗口恢复状态。".into());
                        inner.capture_diagnostics(previous);
                    }
                }
            }
        }
    }
    async fn publish_progress(&self) {
        self.inner.lock().await.flush_events();
        self.emit().await;
    }
    async fn event_worker(self: Arc<Self>, checkpoint_interval: Duration) {
        let mut tick = tokio::time::interval(Duration::from_millis(250));
        let mut checkpoint = Instant::now();
        loop {
            if self.stop_worker.load(Ordering::SeqCst) {
                break;
            }
            tokio::select! {_=tick.tick()=>{},_=self.event_wake.notified()=>{}}
            if self.stop_worker.load(Ordering::SeqCst) {
                break;
            }
            if checkpoint.elapsed() >= checkpoint_interval {
                self.inner
                    .lock()
                    .await
                    .events
                    .capture(AppChange::Checkpoint);
                checkpoint = Instant::now();
            }
            self.publish_progress().await;
        }
        self.event_finished.store(true, Ordering::SeqCst);
        self.event_stopped.notify_waiters();
        if !self.stop_worker.load(Ordering::SeqCst) && !self.shutting_down.load(Ordering::SeqCst) {
            self.ensure_workers();
        }
    }
    async fn apply_critical(&self, changes: Vec<Mutation>) -> AppResult<()> {
        let context = changes.first().and_then(|m| match m {
            Mutation::UpsertTask { task, .. } => self.diagnostics.task_trace(&task.id),
            _ => None,
        });
        let trace = changes.first().and_then(|m| match m {
            Mutation::UpsertTask { task, .. } => self
                .diagnostics
                .task_trace(&task.id)
                .filter(|t| t.is_open(Stage::EnqueuePersist)),
            _ => None,
        });
        let mut result = self
            .writer
            .apply_checked_traced(changes.clone(), trace)
            .await;
        if let (
            Some(trace),
            Err(WriteFailure::Rejected(error) | WriteFailure::Unconfirmed(error)),
        ) = (&context, &result)
        {
            trace.fault(Stage::PersistenceIssue, error);
        }
        while let Err(WriteFailure::Unconfirmed(error)) = &result {
            self.inner
                .lock()
                .await
                .set_persistence_error(Some(format!("持久化结果尚未确认，候选仍保留：{error}")));
            self.emit().await;
            tokio::time::sleep(Duration::from_millis(300)).await;
            result = self.writer.reconcile(changes.clone()).await;
        }
        let result = result.map_err(AppError::from).and_then(|outcomes| {
            if outcomes.contains(&ApplyOutcome::Stale) {
                Err(AppError::new(
                    ErrorCode::StateConflict,
                    ErrorPhase::Persist,
                    "状态版本已过期，未确认本次变更。",
                ))
            } else {
                Ok(())
            }
        });
        if let Err(error) = &result {
            self.inner
                .lock()
                .await
                .set_persistence_error(Some(format!("持久化未完成：{error}")));
            self.emit().await;
        }
        result
    }
    async fn save_candidate(
        &self,
        task: DownloadTask,
        seq: i64,
        move_to_tail: bool,
        claim_attempt: Option<u64>,
    ) -> AppResult<()> {
        let id = task.id.clone();
        let result = self
            .apply_critical(vec![Mutation::UpsertTask {
                seq,
                task: task.clone(),
            }])
            .await;
        let mut inner = self.inner.lock().await;
        inner.pending_tasks.remove(&id);
        if result.is_ok() {
            if let Some(attempt) = claim_attempt {
                inner.attempts.insert(id.clone(), attempt);
            }
            if let Some(index) = inner.tasks.iter().position(|current| current.id == id) {
                if move_to_tail {
                    inner.tasks.remove(index);
                    inner.tasks.push(task);
                } else {
                    inner.tasks[index] = task;
                }
            } else {
                inner.tasks.push(task);
            }
            inner.versions.insert(id.clone(), seq);
            inner.dirty.remove(&id);
            inner.capture_task(&id);
            if inner.pending_terminal.is_empty() {
                inner.set_persistence_error(None);
            }
        }
        drop(inner);
        self.state_changed.notify_waiters();
        result
    }
    async fn flush_progress(&self) -> AppResult<()> {
        let changes = {
            let inner = self.inner.lock().await;
            inner
                .dirty
                .iter()
                .filter(|id| {
                    !inner.pending_tasks.contains(*id) && !inner.pending_terminal.contains_key(*id)
                })
                .filter_map(|id| {
                    inner
                        .tasks
                        .iter()
                        .find(|task| &task.id == id)
                        .map(|task| Mutation::Progress {
                            seq: *inner.versions.get(id).unwrap_or(&0),
                            task: task.clone(),
                        })
                })
                .collect::<Vec<_>>()
        };
        if changes.is_empty() {
            return Ok(());
        }
        let result = self.writer.apply(changes.clone()).await;
        let mut inner = self.inner.lock().await;
        match result {
            Ok(_) => {
                for change in changes {
                    if let Mutation::Progress { seq, task } = change {
                        if inner.versions.get(&task.id) == Some(&seq) {
                            inner.dirty.remove(&task.id);
                        }
                    }
                }
                if inner.pending_terminal.is_empty() {
                    inner.set_persistence_error(None);
                }
                Ok(())
            }
            Err(error) => {
                inner.set_persistence_error(Some(format!(
                    "进度保存失败，最新状态仍保留在内存：{error}"
                )));
                Err(error)
            }
        }
    }
    async fn progress_worker(self: Arc<Self>) {
        let mut tick = tokio::time::interval(Duration::from_secs(2));
        loop {
            if self.stop_worker.load(Ordering::SeqCst) {
                break;
            }
            tokio::select! {_=tick.tick()=>{},_=self.progress_wake.notified()=>{}}
            if self.stop_worker.load(Ordering::SeqCst) {
                break;
            }
            let _ = self.flush_progress().await;
            self.emit().await;
        }
        self.progress_finished.store(true, Ordering::SeqCst);
        self.progress_stopped.notify_waiters();
        if !self.stop_worker.load(Ordering::SeqCst) && !self.shutting_down.load(Ordering::SeqCst) {
            self.ensure_workers();
        }
    }
    pub async fn has_active(&self) -> bool {
        self.inner
            .lock()
            .await
            .tasks
            .iter()
            .any(|t| t.status.active())
    }
    pub async fn task(&self, id: &str) -> AppResult<DownloadTask> {
        let (task, version, token) = {
            let mut inner = self.inner.lock().await;
            let task = inner
                .tasks
                .iter()
                .find(|task| task.id == id)
                .cloned()
                .ok_or_else(task_not_found)?;
            let token = uuid::Uuid::new_v4();
            inner.file_refresh.insert(id.to_string(), token);
            (task, *inner.versions.get(id).unwrap_or(&0), token)
        };
        let exists = match &task.final_path {
            Some(path) => tokio::fs::metadata(path).await.is_ok_and(|m| m.is_file()),
            None => false,
        };
        let current = {
            let mut inner = self.inner.lock().await;
            if inner.apply_availability(id, &task.final_path, version, token, exists) {
                inner.events.capture(AppChange::FileAvailabilityUpdated {
                    files: vec![FileAvailability {
                        task_id: id.into(),
                        file_exists: exists,
                    }],
                });
            }
            inner
                .tasks
                .iter()
                .find(|task| task.id == id)
                .cloned()
                .ok_or_else(task_not_found)?
        };
        self.emit().await;
        Ok(current)
    }
    pub async fn details(&self, id: &str) -> AppResult<TaskDetails> {
        self.task(id).await?;
        let details = {
            let mut inner = self.inner.lock().await;
            inner.flush_events();
            let task = inner
                .tasks
                .iter()
                .find(|task| task.id == id)
                .cloned()
                .ok_or_else(task_not_found)?;
            TaskDetails {
                session_id: inner.events.session_id.clone(),
                revision: inner.events.revision,
                task,
            }
        };
        self.emit().await;
        Ok(details)
    }
    pub async fn detect(&self) -> EngineDiagnostics {
        let settings = self.inner.lock().await.settings.clone();
        let diagnostics = engine::detect(&settings, self.resource_dir.as_deref()).await;
        let mut inner = self.inner.lock().await;
        if inner.settings.engine_path != settings.engine_path
            || inner.settings.ffmpeg_path != settings.ffmpeg_path
        {
            return inner.diagnostics();
        }
        let previous = inner.diagnostics();
        if inner.engines != diagnostics {
            self.preview_generation.fetch_add(1, Ordering::SeqCst);
        }
        inner.engines = diagnostics.clone();
        self.diagnostics.set_engine_versions(&diagnostics);
        inner.capture_diagnostics(previous);
        drop(inner);
        self.emit().await;
        self.inner.lock().await.diagnostics()
    }
    pub async fn probe(&self, url: &str) -> AppResult<MediaInfo> {
        let trace = self.diagnostics.trace(None, None, None);
        self.probe_with_trace(url, trace).await
    }
    pub async fn probe_with_trace(&self, url: &str, trace: Trace) -> AppResult<MediaInfo> {
        trace.begin(Stage::ParseTotal);
        trace.begin(Stage::ParseIpcDispatch);
        let (settings, engines, generation) = {
            let inner = self.inner.lock().await;
            (
                inner.settings.clone(),
                inner.engines.clone(),
                self.preview_generation.load(Ordering::SeqCst),
            )
        };
        trace.finish(Stage::ParseIpcDispatch, Status::Ok, None);
        let fingerprint = Fingerprint::read(&settings, &engines).await;
        let mut result = engine::probe_traced(&settings, &engines, url, Some(&trace)).await;
        if let (Ok(media), Some(fingerprint)) = (&mut result, fingerprint) {
            if generation == self.preview_generation.load(Ordering::SeqCst) {
                media.trusted_preview_ref = self
                    .previews
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .store(
                        media,
                        settings.default_mode.clone(),
                        generation,
                        fingerprint,
                    );
            }
        }
        if let Ok(media) = &result {
            if let Some(reference) = &media.trusted_preview_ref {
                self.previews
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .link_parse(reference, trace.id());
            }
        }
        trace.result(Stage::ParseTotal, &result);
        self.diagnostics.notify();
        result
    }
    /// A/B policy changes are only compiled for debug/test, never a release safety switch.
    #[cfg(debug_assertions)]
    pub fn set_preview_cache_enabled(&self, enabled: bool) {
        self.preview_cache_enabled.store(enabled, Ordering::SeqCst);
    }
    pub async fn enqueue(&self, input: EnqueueInput) -> AppResult<DownloadTask> {
        let trace = self.diagnostics.trace(None, None, None);
        self.enqueue_with_trace(input, trace).await
    }
    pub async fn enqueue_with_trace(
        &self,
        input: EnqueueInput,
        trace: Trace,
    ) -> AppResult<DownloadTask> {
        self.run_owned(move |manager| async move {
            trace.begin(Stage::EnqueueToProgress);
            trace.begin(Stage::EnqueueValidate);
            let result = manager.enqueue_owned(input, &trace).await;
            trace.result(Stage::EnqueueValidate, &result);
            trace.result(Stage::EnqueueMetadataRecheck, &result);
            trace.result(Stage::EnqueuePersist, &result);
            if result.is_err() {
                trace.result(Stage::EnqueueToProgress, &result);
            }
            manager.diagnostics.notify();
            result
        })
        .await
    }
    async fn enqueue_owned(&self, input: EnqueueInput, trace: &Trace) -> AppResult<DownloadTask> {
        if self.shutting_down.load(Ordering::SeqCst) {
            return Err(closing_error("应用正在退出，请稍后重试。"));
        }
        validate_options(&input.mode, &input.quality)?;
        trace.finish(Stage::EnqueueValidate, Status::Ok, None);
        let reference = input.media.trusted_preview_ref.clone();
        let gate = self
            .previews
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .gate(reference.as_deref());
        let _preview_admission = match gate {
            Some(gate) => Some(gate.lock_owned().await),
            None => None,
        };
        let (settings, engines, generation) = {
            let inner = self.inner.lock().await;
            (
                inner.settings.clone(),
                inner.engines.clone(),
                self.preview_generation.load(Ordering::SeqCst),
            )
        };
        let url = engine::validate_url(&input.media.url)?;
        let selection = Selection {
            mode: input.mode.clone(),
            quality: input.quality.clone(),
            format: input.format_id.clone(),
            directory: input
                .output_dir
                .clone()
                .unwrap_or_else(|| settings.download_dir.clone()),
        };
        trace.begin(Stage::EnqueueTrustedValidate);
        let fingerprint = Fingerprint::read(&settings, &engines).await;
        let lookup = self
            .previews
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .lookup(
                reference.as_deref(),
                &url,
                &selection,
                generation,
                fingerprint.as_ref(),
                self.preview_cache_enabled.load(Ordering::SeqCst),
            );
        if let Some(parse) = &lookup.parse_trace {
            trace.bind_related(parse);
        }
        trace.cache_outcome(lookup.outcome);
        trace.finish(Stage::EnqueueTrustedValidate, Status::Ok, None);
        if let Some(id) = lookup.duplicate {
            trace.missing(Stage::EnqueueToProgress, Status::NotApplicable);
            return self.task(&id).await;
        }
        let reused = lookup.media.is_some();
        let mut media = if let Some(media) = lookup.media {
            media
        } else {
            // Neither the frontend DTO nor its reference authorizes a format.
            trace.begin(Stage::EnqueueMetadataRecheck);
            let result = engine::probe(&settings, &engines, &url).await;
            trace.result(Stage::EnqueueMetadataRecheck, &result);
            result?
        };
        media.trusted_preview_ref = None;
        if media.is_playlist {
            return Err(invalid_error(
                "当前 MVP 不执行整列表批量下载，请提供单个视频的链接。",
            ));
        }
        if media.formats.is_empty() {
            return Err(invalid_error(
                "没有可下载的媒体格式，请尝试其他链接或检查引擎诊断。",
            ));
        }
        if let Some(id) = &input.format_id {
            if !media.formats.iter().any(|f| &f.format_id == id) {
                return Err(invalid_error("所选格式已不可用，请重新解析链接。"));
            }
        }
        let settings = self.inner.lock().await.settings.clone();
        let requested_dir = input
            .output_dir
            .unwrap_or_else(|| settings.download_dir.clone());
        let output_dir = prepare_directory(requested_dir).await?;
        let now = chrono::Utc::now().to_rfc3339();
        let task = DownloadTask {
            id: uuid::Uuid::new_v4().to_string(),
            media,
            mode: input.mode,
            quality: input.quality,
            format_id: input.format_id,
            output_dir: output_dir.to_string_lossy().into_owned(),
            status: TaskStatus::Queued,
            progress: None,
            speed: None,
            eta: None,
            downloaded_bytes: 0,
            total_bytes: None,
            final_path: None,
            error: None,
            logs: vec![],
            created_at: now.clone(),
            updated_at: now,
            file_exists: false,
        };
        let task_trace = trace.with_task(&task.id);
        self.diagnostics.attach(&task.id, task_trace.clone());
        task_trace.begin(Stage::EnqueuePersist);
        {
            let _gate = self.critical.lock().await;
            let seq = {
                let mut inner = self.inner.lock().await;
                if generation != self.preview_generation.load(Ordering::SeqCst) {
                    return Err(invalid_error(
                        "引擎配置在校验期间已变化，请重新解析后加入队列。",
                    ));
                }
                engine::build_download_args(&task, &inner.settings, &inner.engines)?;
                if self.shutting_down.load(Ordering::SeqCst) {
                    return Err(closing_error("应用正在退出，请稍后重试。"));
                }
                let seq = inner.next_seq()?;
                inner.pending_tasks.insert(task.id.clone());
                seq
            };
            self.save_candidate(task.clone(), seq, false, None).await?;
            task_trace.finish(Stage::EnqueuePersist, Status::Ok, None);
            task_trace.begin(Stage::QueueWait);
        }
        if reused {
            if let Some(reference) = reference {
                self.previews
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .accept(&reference, selection, task.id.clone());
            }
        }
        self.emit().await;
        self.wake.notify_one();
        Ok(task)
    }
    pub async fn cancel(&self, id: &str) -> AppResult<()> {
        let id = id.to_string();
        self.run_owned(move |manager| async move { manager.cancel_owned(&id, false).await })
            .await
    }
    async fn cancel_owned(&self, id: &str, allow_closing: bool) -> AppResult<()> {
        if self.shutting_down.load(Ordering::SeqCst) && !allow_closing {
            return Err(closing_error("应用正在退出，未接受新的取消写入。"));
        }
        // Accept cancellation before any persistence wait. A decided finalization wins late cancel.
        let should_signal = {
            let mut inner = self.inner.lock().await;
            let task = inner
                .tasks
                .iter()
                .find(|task| task.id == id)
                .ok_or_else(task_not_found)?;
            let signal = task.status.active() && !inner.pending_terminal.contains_key(id);
            if signal {
                inner.cancel_requested.insert(id.to_string());
            }
            signal
        };
        if should_signal {
            if let Some(control) = self
                .active
                .lock()
                .await
                .as_ref()
                .filter(|control| control.id == id)
            {
                control.cancel.cancel();
            }
        }
        {
            let _gate = self.critical.lock().await;
            if self.shutting_down.load(Ordering::SeqCst) && !allow_closing {
                return Err(closing_error("应用正在退出，取消状态由退出协调器保存。"));
            }
            let candidate = {
                let mut inner = self.inner.lock().await;
                if inner.pending_terminal.contains_key(id) {
                    return Err(state_error("进程已结束，最终状态仍在等待数据库确认。"));
                }
                let mut task = inner
                    .tasks
                    .iter()
                    .find(|task| task.id == id)
                    .cloned()
                    .ok_or_else(task_not_found)?;
                if !task.status.active() {
                    return Ok(());
                }
                if !transitions::request_cancel(&mut task) {
                    return Ok(());
                }
                task.updated_at = chrono::Utc::now().to_rfc3339();
                let seq = inner.next_seq()?;
                inner.pending_tasks.insert(id.to_string());
                (task, seq)
            };
            // Close the claim/publication race: Control may have appeared after the fast path.
            if let Some(control) = self
                .active
                .lock()
                .await
                .as_ref()
                .filter(|control| control.id == id)
            {
                control.cancel.cancel();
            }
            let queued_cancel = candidate.0.status == TaskStatus::Cancelled;
            if queued_cancel {
                if let Some(t) = self.diagnostics.task_trace(id) {
                    t.begin(Stage::DownloadTerminal);
                }
            }
            self.save_candidate(candidate.0, candidate.1, false, None)
                .await?;
            if queued_cancel {
                if let Some(t) = self.diagnostics.task_trace(id) {
                    t.finish(Stage::QueueWait, Status::Cancelled, None);
                    t.finish(Stage::EnqueueToProgress, Status::Cancelled, None);
                    t.finish(Stage::DownloadTerminal, Status::Cancelled, None);
                    t.missing(Stage::DownloadEngineSpawn, Status::NotApplicable);
                    t.missing(Stage::DownloadFirstProgress, Status::Unobserved);
                    t.missing(Stage::DownloadPostprocess, Status::NotApplicable);
                }
            }
        }
        self.emit().await;
        self.wake.notify_one();
        Ok(())
    }
    pub async fn retry(&self, id: &str) -> AppResult<()> {
        let id = id.to_string();
        self.run_owned(move |manager| async move { manager.retry_owned(&id).await })
            .await
    }
    async fn retry_owned(&self, id: &str) -> AppResult<()> {
        if self.shutting_down.load(Ordering::SeqCst) {
            return Err(closing_error("应用正在退出。"));
        }
        let original = self.task(id).await?;
        let directory = original.output_dir.clone();
        prepare_directory(directory).await?;
        {
            let _gate = self.critical.lock().await;
            let (task, seq) = {
                let mut inner = self.inner.lock().await;
                if self.shutting_down.load(Ordering::SeqCst) {
                    return Err(closing_error("应用正在退出。"));
                }
                let mut task = inner
                    .tasks
                    .iter()
                    .find(|task| task.id == id)
                    .cloned()
                    .ok_or_else(task_not_found)?;
                engine::build_download_args(&task, &inner.settings, &inner.engines)?;
                transitions::retry(&mut task)?;
                task.updated_at = chrono::Utc::now().to_rfc3339();
                let seq = inner.next_seq()?;
                inner.pending_tasks.insert(id.to_string());
                (task, seq)
            };
            let previous = self.diagnostics.task_trace(id);
            let trace = self
                .diagnostics
                .trace(None, previous.as_ref().map(|t| t.id()), Some(id));
            trace.begin(Stage::EnqueueToProgress);
            trace.begin(Stage::EnqueuePersist);
            let result = self.save_candidate(task, seq, true, None).await;
            trace.result(Stage::EnqueuePersist, &result);
            if result.is_err() {
                trace.result(Stage::EnqueueToProgress, &result);
            }
            result?;
            trace.begin(Stage::QueueWait);
            self.diagnostics.attach(id, trace);
            self.inner.lock().await.cancel_requested.remove(id);
        }
        self.emit().await;
        self.wake.notify_one();
        Ok(())
    }
    pub async fn remove_history(&self, id: &str) -> AppResult<()> {
        let id = id.to_string();
        self.run_owned(move |manager| async move { manager.remove_history_owned(&id).await })
            .await
    }
    async fn remove_history_owned(&self, id: &str) -> AppResult<()> {
        {
            let _gate = self.critical.lock().await;
            if self.shutting_down.load(Ordering::SeqCst) {
                return Err(closing_error("应用正在退出，未接受新的写入。"));
            }
            let seq = {
                let mut inner = self.inner.lock().await;
                let task = inner
                    .tasks
                    .iter()
                    .find(|task| task.id == id)
                    .ok_or_else(task_not_found)?;
                if task.status != TaskStatus::Completed {
                    return Err(state_error("只能删除已完成的历史记录。"));
                }
                let seq = inner.next_seq()?;
                inner.pending_tasks.insert(id.to_string());
                seq
            };
            let result = self
                .apply_critical(vec![Mutation::DeleteTask {
                    seq,
                    task_id: id.to_string(),
                }])
                .await;
            let mut inner = self.inner.lock().await;
            inner.pending_tasks.remove(id);
            if result.is_ok() {
                inner.tasks.retain(|task| task.id != id);
                inner.events.removed(id);
                inner.versions.insert(id.to_string(), seq);
                inner.dirty.remove(id);
                if inner.pending_terminal.is_empty() {
                    inner.set_persistence_error(None);
                }
            }
            result?;
        }
        self.emit().await;
        Ok(())
    }
    pub async fn update_settings(&self, settings: Settings) -> AppResult<Settings> {
        self.run_owned(move |manager| async move { manager.update_settings_owned(settings).await })
            .await
    }
    async fn update_settings_owned(&self, settings: Settings) -> AppResult<Settings> {
        if self.shutting_down.load(Ordering::SeqCst) {
            return Err(closing_error("应用正在退出，未接受新的设置。"));
        }
        validate_settings(&settings)?;
        let directory = settings.download_dir.clone();
        prepare_directory(directory).await?;
        {
            let _gate = self.critical.lock().await;
            if self.shutting_down.load(Ordering::SeqCst) {
                return Err(closing_error("应用正在退出，未接受新的写入。"));
            }
            let seq = self.inner.lock().await.next_seq()?;
            self.apply_critical(vec![Mutation::UpsertSettings {
                seq,
                settings: settings.clone(),
            }])
            .await?;
            let mut inner = self.inner.lock().await;
            if inner.settings != settings {
                if inner.settings.engine_path != settings.engine_path
                    || inner.settings.ffmpeg_path != settings.ffmpeg_path
                    || inner.settings.default_mode != settings.default_mode
                {
                    self.preview_generation.fetch_add(1, Ordering::SeqCst);
                }
                inner.settings = settings.clone();
                inner.events.capture(AppChange::SettingsUpdated {
                    settings: settings.clone(),
                });
            }
            if inner.pending_terminal.is_empty() {
                inner.set_persistence_error(None);
            }
        }
        self.detect().await;
        Ok(settings)
    }
    pub async fn shutdown(self: &Arc<Self>) -> AppResult<()> {
        self.shutdown_with_timeout(Duration::from_secs(12)).await
    }
    async fn shutdown_with_timeout(self: &Arc<Self>, timeout: Duration) -> AppResult<()> {
        let deadline = tokio::time::Instant::now() + timeout;
        let gate = tokio::time::timeout_at(deadline, self.shutdown_gate.clone().lock_owned())
            .await
            .map_err(|_| shutdown_timeout("前一次退出仍在等待数据库确认，请稍后重试。"))?;
        self.shutting_down.store(true, Ordering::SeqCst);
        let aborted = Arc::new(AtomicBool::new(false));
        let cancellation = aborted.clone();
        let manager = self.clone();
        // Do not cancel an already-submitted COMMIT if the outer deadline expires.
        let mut operation = tauri::async_runtime::spawn(async move {
            let result = manager.shutdown_steps(cancellation).await;
            (gate, result)
        });
        let result = match tokio::time::timeout_at(deadline, &mut operation).await {
            Ok(Ok((_gate, result))) => {
                if result.is_ok() {
                    self.stop_worker.store(true, Ordering::SeqCst);
                    self.wake.notify_one();
                    self.progress_wake.notify_one();
                    self.event_wake.notify_one();
                    tokio::time::timeout_at(deadline, async {
                        loop {
                            let queue = self.worker_stopped.notified();
                            let progress = self.progress_stopped.notified();
                            let events = self.event_stopped.notified();
                            if self.worker_finished.load(Ordering::SeqCst)
                                && self.progress_finished.load(Ordering::SeqCst)
                                && self.event_finished.load(Ordering::SeqCst)
                            {
                                break;
                            }
                            tokio::select! {_=queue=>{},_=progress=>{},_=events=>{}}
                        }
                    })
                    .await
                    .map_err(|_| shutdown_timeout("停止后台执行器超时，请重试退出。"))
                } else {
                    result
                }
            }
            Ok(Err(error)) => Err(state_error("退出协调失败。")
                .with_phase(ErrorPhase::Shutdown)
                .with_details(error.to_string())),
            Err(_) => Err(shutdown_timeout(
                "退出超时，正在保全未确认写入；应用将保持可用，请稍后重试。",
            )),
        };
        if result.is_err() {
            aborted.store(true, Ordering::SeqCst);
            self.stop_worker.store(false, Ordering::SeqCst);
            self.shutting_down.store(false, Ordering::SeqCst);
            self.ensure_workers();
        }
        result
    }
    async fn shutdown_steps(&self, aborted: Arc<AtomicBool>) -> AppResult<()> {
        let running = {
            let inner = self.inner.lock().await;
            inner
                .tasks
                .iter()
                .filter(|task| task.status.running())
                .map(|task| task.id.clone())
                .collect::<Vec<_>>()
        };
        for id in running {
            if aborted.load(Ordering::SeqCst) {
                return Err(closing_error("退出已取消，数据库写入仍会收敛。"));
            }
            let pending = self.inner.lock().await.pending_terminal.contains_key(&id);
            if !pending {
                self.cancel_owned(&id, true).await?;
            }
        }
        {
            let _gate = self.critical.lock().await;
            if aborted.load(Ordering::SeqCst) {
                return Err(closing_error("退出已取消。"));
            }
            let changes = {
                let mut inner = self.inner.lock().await;
                let tasks = inner
                    .tasks
                    .iter()
                    .filter(|task| task.status == TaskStatus::Queued)
                    .cloned()
                    .collect::<Vec<_>>();
                let mut changes = Vec::new();
                for mut task in tasks {
                    transitions::request_cancel(&mut task);
                    task.updated_at = chrono::Utc::now().to_rfc3339();
                    let seq = inner.next_seq()?;
                    changes.push(Mutation::UpsertTask { seq, task });
                }
                changes
            };
            if !changes.is_empty() {
                self.apply_critical(changes.clone()).await?;
                let mut inner = self.inner.lock().await;
                for change in changes {
                    if let Mutation::UpsertTask { seq, task } = change {
                        if let Some(current) =
                            inner.tasks.iter_mut().find(|current| current.id == task.id)
                        {
                            *current = task.clone();
                        }
                        inner.versions.insert(task.id.clone(), seq);
                        inner.dirty.remove(&task.id);
                        inner.capture_task(&task.id);
                    }
                }
            }
        }
        self.emit().await;
        self.wake.notify_one();
        loop {
            let changed = self.state_changed.notified();
            if aborted.load(Ordering::SeqCst) {
                return Err(closing_error("退出已取消。"));
            }
            let pending = !self.inner.lock().await.pending_terminal.is_empty();
            if self.active.lock().await.is_none() && !pending {
                break;
            }
            changed.await;
        }
        self.flush_progress().await?;
        self.writer.flush().await
    }
    async fn claim_task(
        &self,
    ) -> AppResult<Option<(DownloadTask, Settings, EngineDiagnostics, Arc<Control>)>> {
        let _gate = self.critical.lock().await;
        if self.shutting_down.load(Ordering::SeqCst) {
            return Ok(None);
        }
        if self.active.lock().await.is_some() {
            return Ok(None);
        }
        let selected = {
            let mut inner = self.inner.lock().await;
            if let Some(mut task) = inner
                .tasks
                .iter()
                .find(|task| task.status == TaskStatus::Queued)
                .cloned()
            {
                if !transitions::claim(&mut task) {
                    return Ok(None);
                }
                let attempt = inner
                    .attempts
                    .get(&task.id)
                    .copied()
                    .unwrap_or(0)
                    .checked_add(1)
                    .ok_or_else(|| state_error("任务执行序号已达到上限。"))?;
                task.updated_at = chrono::Utc::now().to_rfc3339();
                let seq = inner.next_seq()?;
                inner.pending_tasks.insert(task.id.clone());
                Some((task, seq, attempt))
            } else {
                None
            }
        };
        let Some((task, seq, attempt)) = selected else {
            return Ok(None);
        };
        self.save_candidate(task.clone(), seq, false, Some(attempt))
            .await?;
        if let Some(trace) = self.diagnostics.task_trace(&task.id) {
            trace.finish(Stage::QueueWait, Status::Ok, None);
            trace.refresh_environment();
            trace.begin(Stage::DownloadTotal);
        }
        let mut inner = self.inner.lock().await;
        if self.shutting_down.load(Ordering::SeqCst) {
            inner.cancel_requested.insert(task.id.clone());
        }
        let control = Arc::new(Control {
            id: task.id.clone(),
            attempt,
            cancel: Cancellation::default(),
        });
        if inner.cancel_requested.contains(&task.id) {
            control.cancel.cancel();
        }
        let settings = inner.settings.clone();
        let engines = inner.engines.clone();
        drop(inner);
        *self.active.lock().await = Some(control.clone());
        if self.inner.lock().await.cancel_requested.contains(&task.id)
            || self.shutting_down.load(Ordering::SeqCst)
        {
            control.cancel.cancel();
        }
        Ok(Some((task, settings, engines, control)))
    }
    async fn finish_task(
        &self,
        id: &str,
        control: &Control,
        result: AppResult<()>,
    ) -> AppResult<()> {
        {
            let _gate = self.critical.lock().await;
            let mut inner = self.inner.lock().await;
            if inner.attempts.get(id) != Some(&control.attempt) {
                return Ok(());
            }
            if !inner.pending_terminal.contains_key(id) {
                let mut task = inner
                    .tasks
                    .iter()
                    .find(|task| task.id == id)
                    .cloned()
                    .ok_or_else(task_not_found)?;
                let cancelled =
                    control.cancel.is_cancelled() || inner.cancel_requested.contains(id);
                if !transitions::settle(&mut task, cancelled, result) {
                    return Ok(());
                }
                task.updated_at = chrono::Utc::now().to_rfc3339();
                let seq = inner.next_seq()?;
                inner.pending_tasks.insert(id.to_string());
                inner.dirty.remove(id);
                inner.pending_terminal.insert(
                    id.to_string(),
                    PendingTerminal {
                        seq,
                        task,
                        attempt: control.attempt,
                    },
                );
            }
        }
        self.commit_terminal(id).await
    }
    async fn commit_terminal(&self, id: &str) -> AppResult<()> {
        let _gate = self.critical.lock().await;
        let pending = { self.inner.lock().await.pending_terminal.get(id).cloned() };
        let Some(pending) = pending else {
            return Ok(());
        };
        let result = self
            .apply_critical(vec![Mutation::UpsertTask {
                seq: pending.seq,
                task: pending.task.clone(),
            }])
            .await;
        let mut inner = self.inner.lock().await;
        if result.is_ok() {
            if inner.attempts.get(id) == Some(&pending.attempt) {
                if let Some(trace) = self.diagnostics.task_trace(id) {
                    let status = match pending.task.status {
                        TaskStatus::Completed => Status::Ok,
                        TaskStatus::Cancelled => Status::Cancelled,
                        _ => Status::Failed,
                    };
                    trace.finish(Stage::DownloadTerminal, status, None);
                    trace.finish(Stage::DownloadTotal, status, None);
                    for stage in [
                        Stage::DownloadFirstOutput,
                        Stage::DownloadBeforeDl,
                        Stage::DownloadBeforeDlToProgress,
                        Stage::DownloadProcessExit,
                    ] {
                        trace.missing(stage, Status::Unobserved);
                    }
                    trace.missing(Stage::DownloadFirstProgress, Status::Unobserved);
                    trace.missing(Stage::DownloadEngineSpawn, Status::Unobserved);
                    trace.missing(Stage::DownloadActive, Status::Unobserved);
                    trace.missing(Stage::EnqueueToProgress, Status::Unobserved);
                    trace.missing(Stage::DownloadPostprocess, Status::NotApplicable);
                }
            }
            if inner.attempts.get(id) == Some(&pending.attempt) {
                if let Some(task) = inner.tasks.iter_mut().find(|task| task.id == id) {
                    *task = pending.task;
                }
                inner.versions.insert(id.to_string(), pending.seq);
            }
            inner.pending_terminal.remove(id);
            inner.pending_tasks.remove(id);
            inner.cancel_requested.remove(id);
            inner.dirty.remove(id);
            inner.capture_task(id);
            inner.set_process_error(None);
            if inner.pending_terminal.is_empty() {
                inner.set_persistence_error(None);
            }
        } else {
            let error = Some(
                "进程已结束，最终状态保存失败；文件及待确认结果已保留，请检查数据库诊断。"
                    .to_string(),
            );
            let changed = if let Some(task) = inner.tasks.iter_mut().find(|task| task.id == id) {
                let changed = task.error != error;
                task.error = error;
                changed
            } else {
                false
            };
            if changed {
                inner.capture_task(id);
            }
        }
        drop(inner);
        self.state_changed.notify_waiters();
        result
    }

    async fn worker(self: Arc<Self>) {
        loop {
            if self.stop_worker.load(Ordering::SeqCst) {
                break;
            }
            let selected = match self.claim_task().await {
                Ok(value) => value,
                Err(error) => {
                    self.inner
                        .lock()
                        .await
                        .set_persistence_error(Some(error.safe_text()));
                    self.emit().await;
                    tokio::select! {_=tokio::time::sleep(Duration::from_millis(500))=>{},_=self.wake.notified()=>{}}
                    continue;
                }
            };
            if let Some((task, settings, engines, control)) = selected {
                self.emit().await;
                let result = self.run_process(&task, &settings, &engines, &control).await;
                self.settle_download(&task.id, &control, result).await;
                if settings.completion_action == "folder" {
                    let action = match self.task(&task.id).await {
                        Ok(current) if current.status == TaskStatus::Completed => {
                            open_folder(&current.output_dir).await
                        }
                        Ok(_) => Ok(()),
                        // A user may remove the completed row before this optional action.
                        Err(error) if error.code == ErrorCode::TaskNotFound => Ok(()),
                        Err(error) => Err(error),
                    };
                    if let Err(error) = action {
                        crate::diagnostics::report_error(&error);
                        self.inner
                            .lock()
                            .await
                            .set_process_error(Some(error.safe_text()));
                        self.emit().await;
                    }
                }
                continue;
            }
            if self.stop_worker.load(Ordering::SeqCst) {
                break;
            }
            self.wake.notified().await;
        }
        self.worker_finished.store(true, Ordering::SeqCst);
        self.worker_stopped.notify_waiters();
        if !self.stop_worker.load(Ordering::SeqCst) && !self.shutting_down.load(Ordering::SeqCst) {
            self.ensure_workers();
        }
    }

    async fn settle_download(&self, id: &str, control: &Control, result: AppResult<()>) {
        let current_attempt = self.inner.lock().await.attempts.get(id) == Some(&control.attempt);
        if current_attempt {
            if let Some(trace) = self.diagnostics.task_trace(id) {
                if let Err(error) = &result {
                    trace.failure(error);
                }
                let status = if result.is_err() {
                    Status::Failed
                } else if control.cancel.is_cancelled() {
                    Status::Cancelled
                } else {
                    Status::Ok
                };
                trace.finish(Stage::DownloadActive, status, result.as_ref().err());
                trace.finish(Stage::DownloadPostprocess, status, result.as_ref().err());
                trace.begin(Stage::DownloadTerminal);
            }
        }
        loop {
            match self.finish_task(id, control, result.clone()).await {
                Ok(()) => break,
                Err(error) => {
                    let mut inner = self.inner.lock().await;
                    if error.phase == ErrorPhase::Persist {
                        inner.set_persistence_error(Some(error.safe_text()));
                    } else {
                        inner.set_process_error(Some(error.safe_text()));
                    }
                    drop(inner);
                    self.emit().await;
                    tokio::time::sleep(Duration::from_millis(300)).await;
                }
            }
        }
        {
            let mut active = self.active.lock().await;
            if active.as_ref().is_some_and(|current| {
                current.id == control.id && current.attempt == control.attempt
            }) {
                *active = None;
            }
        }
        self.state_changed.notify_waiters();
        self.wake.notify_one();
        self.emit().await;
    }

    async fn apply_download_event(
        &self,
        id: &str,
        attempt: u64,
        tracker: &mut RuntimeTracker,
        event: DownloadEvent,
    ) -> AppResult<()> {
        let mut inner = self.inner.lock().await;
        if let DownloadEvent::Diagnostic(error) = &event {
            let current = inner.attempts.get(id) == Some(&attempt)
                && inner
                    .tasks
                    .iter()
                    .any(|task| task.id == id && task.status.running())
                && !inner.pending_terminal.contains_key(id);
            if current {
                if let Some(trace) = self.diagnostics.task_trace(id) {
                    trace.fault(Stage::DownloadProcessIssue, error);
                }
                // Cleanup failures must stay visible even after cancellation
                // has closed ordinary progress/log admission.
                inner.set_process_error(Some(error.safe_text()));
            }
        }
        if !inner.accepts_progress(id, attempt) {
            return Ok(());
        }
        if let DownloadEvent::BeforeDownload {
            stream,
            sequence,
            observed_at,
        } = &event
        {
            if *stream != crate::process::OutputStream::Stdout {
                return Ok(());
            }
            if tracker.admit_line(*stream, *sequence) {
                if let Some(trace) = self.diagnostics.task_trace(id) {
                    trace.finish_at(Stage::DownloadBeforeDl, Status::Ok, None, *observed_at);
                    trace.begin_at(Stage::DownloadBeforeDlToProgress, *observed_at);
                }
            }
            return Ok(());
        }
        let progress_kind = match &event {
            DownloadEvent::Progress { fact, .. } => match &fact.event {
                engine::ProgressEvent::Download { .. } => 1,
                engine::ProgressEvent::Postprocessing => 2,
                _ => 0,
            },
            _ => 0,
        };
        let mut appended = None;
        let change = {
            let task = inner
                .tasks
                .iter_mut()
                .find(|task| task.id == id)
                .ok_or_else(task_not_found)?;
            let change = match event {
                DownloadEvent::Progress {
                    stream,
                    sequence,
                    fact,
                } => transitions::apply_progress(task, tracker, stream, sequence, &fact),
                DownloadEvent::Log {
                    stream,
                    sequence,
                    line,
                } => {
                    if !tracker.admit_line(stream, sequence) {
                        return Ok(());
                    }
                    appended = Some(push_log(task, line));
                    RuntimeChange::Progress
                }
                DownloadEvent::BeforeDownload { .. } => {
                    unreachable!("handled after attempt admission")
                }
                DownloadEvent::Diagnostic(error) => {
                    appended = Some(push_log(task, error.safe_text()));
                    RuntimeChange::Progress
                }
            };
            if let RuntimeChange::Diagnostic(message) = change {
                appended = Some(push_log(task, message.to_string()));
            }
            if change != RuntimeChange::Unchanged {
                task.updated_at = chrono::Utc::now().to_rfc3339();
            }
            change
        };
        if change != RuntimeChange::Unchanged {
            if matches!(change, RuntimeChange::Progress | RuntimeChange::FullUpsert) {
                if let Some(trace) = self.diagnostics.task_trace(id) {
                    if progress_kind == 1 {
                        trace.finish(Stage::DownloadFirstProgress, Status::Ok, None);
                        trace.finish(Stage::DownloadBeforeDlToProgress, Status::Ok, None);
                        trace.finish(Stage::EnqueueToProgress, Status::Ok, None);
                        trace.begin(Stage::DownloadActive);
                    } else if progress_kind == 2 {
                        trace.finish(Stage::DownloadActive, Status::Ok, None);
                        trace.begin(Stage::DownloadPostprocess);
                    }
                }
            }
            let seq = inner.next_seq()?;
            inner.versions.insert(id.to_string(), seq);
            inner.dirty.insert(id.to_string());
            inner.events.mark_progress(id);
            if let Some(line) = appended {
                inner.events.append_log(id, &line);
            }
            if change == RuntimeChange::FullUpsert {
                inner.capture_task(id);
            }
        }
        Ok(())
    }

    async fn run_process(
        &self,
        task: &DownloadTask,
        settings: &Settings,
        engines: &EngineDiagnostics,
        control: &Arc<Control>,
    ) -> AppResult<()> {
        if control.cancel.is_cancelled() {
            return Ok(());
        }
        let download = engine::start_download_traced(
            task,
            settings,
            engines,
            control.cancel.clone(),
            self.diagnostics.task_trace(&task.id),
        )?;
        self.drive_download(task, control, download).await
    }

    async fn drive_download(
        &self,
        task: &DownloadTask,
        control: &Control,
        mut download: engine::EngineDownload,
    ) -> AppResult<()> {
        let mut tracker = RuntimeTracker::default();
        let mut state_failure = None;
        while let Some(event) = download.recv().await {
            if state_failure.is_none() || matches!(&event, DownloadEvent::Diagnostic(_)) {
                if let Err(error) = self
                    .apply_download_event(&task.id, control.attempt, &mut tracker, event)
                    .await
                {
                    if state_failure.is_none() {
                        state_failure = Some(error);
                    }
                    control.cancel.cancel();
                }
            }
        }
        // Never settle/start the next task until the concrete owner has reaped
        // the child and every reader, including when applying a fact failed.
        let outcome = download.finish().await?;
        if let Some(error) = state_failure {
            return Err(error);
        }
        if outcome.cancelled || control.cancel.is_cancelled() {
            return Ok(());
        }
        let (path, directory) = {
            let inner = self.inner.lock().await;
            let current = inner
                .tasks
                .iter()
                .find(|current| current.id == task.id)
                .ok_or_else(task_not_found)?;
            let path = current.final_path.clone().ok_or_else(|| {
                AppError::new(
                    ErrorCode::OutputValidation,
                    ErrorPhase::Download,
                    "引擎已退出，但没有报告最终文件路径。请检查任务日志与 yt-dlp 版本。",
                )
            })?;
            (path, current.output_dir.clone())
        };
        validate_output(path, directory).await?;
        Ok(())
    }
}

fn push_log(task: &mut DownloadTask, line: String) -> String {
    let line = line.chars().take(1600).collect::<String>();
    task.logs.push(line.clone());
    if task.logs.len() > LOG_LIMIT {
        task.logs.drain(..task.logs.len() - LOG_LIMIT);
    }
    line
}

fn validate_options(mode: &str, quality: &str) -> AppResult<()> {
    if !["video", "audio"].contains(&mode)
        || !["best", "2160", "1440", "1080", "720", "480"].contains(&quality)
    {
        return Err(invalid_error("下载模式或质量预设无效。"));
    }
    Ok(())
}
fn validate_settings(settings: &Settings) -> AppResult<()> {
    validate_options(&settings.default_mode, &settings.quality)?;
    if !["dark", "light", "system"].contains(&settings.theme.as_str())
        || !["comfortable", "compact"].contains(&settings.density.as_str())
        || !["tray", "exit"].contains(&settings.close_behavior.as_str())
        || !["none", "folder"].contains(&settings.completion_action.as_str())
    {
        return Err(invalid_error("设置选项无效。"));
    }
    for path in [&settings.engine_path, &settings.ffmpeg_path] {
        if !path.is_empty() && (!Path::new(path).is_absolute() || path.len() > 32700) {
            return Err(invalid_error("引擎路径必须是有效的绝对路径。"));
        }
    }
    Ok(())
}

fn invalid_error(message: &str) -> AppError {
    AppError::new(ErrorCode::InvalidInput, ErrorPhase::Validate, message)
}
fn state_error(message: &str) -> AppError {
    AppError::new(ErrorCode::StateConflict, ErrorPhase::Download, message)
}
fn task_not_found() -> AppError {
    AppError::new(
        ErrorCode::TaskNotFound,
        ErrorPhase::Download,
        "任务记录不存在。",
    )
}
fn closing_error(message: &str) -> AppError {
    AppError::new(ErrorCode::ShuttingDown, ErrorPhase::Shutdown, message)
}
fn shutdown_timeout(message: &str) -> AppError {
    AppError::new(ErrorCode::ProcessTimeout, ErrorPhase::Shutdown, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::Persistence;
    fn fixture_task(id: &str, status: TaskStatus, directory: &Path) -> DownloadTask {
        DownloadTask {
            id: id.into(),
            media: MediaInfo {
                trusted_preview_ref: None,
                url: "https://example.com/fixture.mp4".into(),
                title: "Fixture only".into(),
                media_id: "fixture".into(),
                source: "fixture".into(),
                duration: None,
                thumbnail: None,
                is_playlist: false,
                formats: vec![FormatOption {
                    format_id: "direct".into(),
                    ext: "mp4".into(),
                    label: "direct".into(),
                    height: Some(360),
                    has_video: true,
                    has_audio: true,
                    filesize: None,
                    requires_ffmpeg: false,
                }],
                warnings: vec![],
            },
            mode: "video".into(),
            quality: "best".into(),
            format_id: None,
            output_dir: directory.to_string_lossy().into_owned(),
            status,
            progress: None,
            speed: None,
            eta: None,
            downloaded_bytes: 0,
            total_bytes: None,
            final_path: None,
            error: None,
            logs: vec![],
            created_at: "2026-01-01".into(),
            updated_at: "2026-01-01".into(),
            file_exists: false,
        }
    }
    async fn isolated_manager(directory: &Path, tasks: Vec<DownloadTask>) -> Arc<Manager> {
        let (writer, loaded) = PersistenceWriter::open(directory.to_path_buf())
            .await
            .unwrap();
        let mut engines = EngineDiagnostics::default();
        engines.ytdlp.available = true;
        let manager = Arc::new(Manager {
            previews: std::sync::Mutex::new(TrustedPreviews::default()),
            preview_generation: std::sync::atomic::AtomicU64::new(0),
            preview_cache_enabled: AtomicBool::new(true),
            diagnostics: Diagnostics::disabled(),
            self_ref: OnceLock::new(),
            inner: Mutex::new(Inner {
                settings: Settings::default(),
                engines,
                tasks,
                next_seq: loaded.max_mutation_seq,
                file_refresh: HashMap::new(),
                versions: HashMap::new(),
                attempts: HashMap::new(),
                pending_tasks: HashSet::new(),
                pending_terminal: HashMap::new(),
                cancel_requested: HashSet::new(),
                dirty: HashSet::new(),
                persistence_error: None,
                process_error: None,
                storage_diagnostics: Vec::new(),
                events: EventJournal::new(),
            }),
            writer,
            critical: Mutex::new(()),
            publisher: Mutex::new(()),
            #[cfg(test)]
            test_events: Mutex::new(Vec::new()),
            shutdown_gate: Arc::new(Mutex::new(())),
            stop_worker: AtomicBool::new(false),
            progress_finished: AtomicBool::new(true),
            progress_stopped: Notify::new(),
            progress_wake: Notify::new(),
            event_finished: AtomicBool::new(true),
            event_stopped: Notify::new(),
            event_wake: Notify::new(),
            state_changed: Notify::new(),
            active: Mutex::new(None),
            wake: Notify::new(),
            shutting_down: AtomicBool::new(false),
            worker_stopped: Notify::new(),
            worker_finished: AtomicBool::new(true),
            resource_dir: None,
            app: None,
        });
        let seed = {
            let mut inner = manager.inner.lock().await;
            let tasks = inner.tasks.clone();
            tasks
                .into_iter()
                .map(|task| Mutation::UpsertTask {
                    seq: inner.next_seq().unwrap(),
                    task,
                })
                .collect()
        };
        manager.writer.apply(seed).await.unwrap();
        let _ = manager.self_ref.set(Arc::downgrade(&manager));
        manager
    }
    #[tokio::test]
    async fn queued_cancel_is_persisted_retry_is_guarded_and_delete_never_removes_files() {
        let directory =
            std::env::temp_dir().join(format!("fluxdl-state-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let file = directory.join("existing.mp4");
        std::fs::write(&file, b"fixture").unwrap();
        let mut completed = fixture_task("completed", TaskStatus::Completed, &directory);
        completed.final_path = Some(file.to_string_lossy().into_owned());
        let manager = isolated_manager(
            &directory,
            vec![
                fixture_task("queued", TaskStatus::Queued, &directory),
                completed,
            ],
        )
        .await;
        manager.cancel("queued").await.unwrap();
        manager.cancel("queued").await.unwrap();
        assert_eq!(
            manager.task("queued").await.unwrap().status,
            TaskStatus::Cancelled
        );
        let (first, second) = tokio::join!(manager.retry("queued"), manager.retry("queued"));
        assert_ne!(
            first.is_ok(),
            second.is_ok(),
            "exactly one simultaneous retry must succeed"
        );
        assert_eq!(
            manager.task("queued").await.unwrap().status,
            TaskStatus::Queued
        );
        assert!(manager.remove_history("queued").await.is_err());
        assert!(manager.retry("completed").await.is_err());
        manager.remove_history("completed").await.unwrap();
        assert!(file.is_file());
        drop(manager);
        let (store, loaded) = Persistence::open(&directory).unwrap();
        assert_eq!(loaded.tasks.len(), 1);
        assert_eq!(loaded.tasks[0].status, TaskStatus::Interrupted);
        drop(store);
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[tokio::test]
    async fn terminal_ack_wait_never_holds_state_lock_or_exposes_completed() {
        let directory =
            std::env::temp_dir().join(format!("fluxdl-terminal-ack-{}", uuid::Uuid::new_v4()));
        let manager = isolated_manager(
            &directory,
            vec![fixture_task("running", TaskStatus::Downloading, &directory)],
        )
        .await;
        manager
            .inner
            .lock()
            .await
            .attempts
            .insert("running".into(), 1);
        let control = Arc::new(Control {
            id: "running".into(),
            attempt: 1,
            cancel: Cancellation::default(),
        });
        let connection = rusqlite::Connection::open(directory.join("fluxdl.sqlite3")).unwrap();
        connection.execute_batch("BEGIN IMMEDIATE;").unwrap();
        let candidate_manager = manager.clone();
        let candidate_control = control.clone();
        let job = tokio::spawn(async move {
            candidate_manager
                .finish_task("running", &candidate_control, Ok(()))
                .await
        });
        tokio::time::timeout(Duration::from_secs(2), async {
            while !manager
                .inner
                .lock()
                .await
                .pending_terminal
                .contains_key("running")
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_ne!(
            manager.snapshot().await.tasks[0].status,
            TaskStatus::Completed
        );
        assert_ne!(
            manager.cached_snapshot().await.tasks[0].status,
            TaskStatus::Completed
        );
        let guard = tokio::time::timeout(Duration::from_millis(100), manager.inner.lock())
            .await
            .expect("database wait held main state lock");
        drop(guard);
        connection.execute_batch("ROLLBACK;").unwrap();
        drop(connection);
        job.await.unwrap().unwrap();
        assert_eq!(
            manager.cached_snapshot().await.tasks[0].status,
            TaskStatus::Completed
        );
        drop(manager);
        let (repo, loaded) = Persistence::open(&directory).unwrap();
        assert_eq!(loaded.tasks[0].status, TaskStatus::Completed);
        drop(repo);
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[tokio::test]
    async fn terminal_write_failure_retains_candidate_and_recovers_without_false_success() {
        let directory =
            std::env::temp_dir().join(format!("fluxdl-terminal-failure-{}", uuid::Uuid::new_v4()));
        let manager = isolated_manager(
            &directory,
            vec![fixture_task("running", TaskStatus::Downloading, &directory)],
        )
        .await;
        let diagnostic_trace = manager.diagnostics.trace(None, None, None);
        manager
            .diagnostics
            .attach("running", diagnostic_trace.clone());
        manager
            .inner
            .lock()
            .await
            .attempts
            .insert("running".into(), 1);
        let control = Control {
            id: "running".into(),
            attempt: 1,
            cancel: Cancellation::default(),
        };
        let connection = rusqlite::Connection::open(directory.join("fluxdl.sqlite3")).unwrap();
        connection.execute_batch("CREATE TRIGGER reject_terminal BEFORE INSERT ON tasks WHEN json_extract(NEW.document,'$.status')='completed' BEGIN SELECT RAISE(ABORT,'fixture terminal failure'); END;").unwrap();
        assert!(manager
            .finish_task("running", &control, Ok(()))
            .await
            .is_err());
        let snapshot = manager.snapshot().await;
        assert_ne!(snapshot.tasks[0].status, TaskStatus::Completed);
        assert!(snapshot.tasks[0].error.is_some());
        assert!(!snapshot.engines.warnings.is_empty());
        assert_eq!(manager.inner.lock().await.pending_terminal.len(), 1);
        let diagnostic_events = manager.diagnostics.read().events;
        assert!(diagnostic_events
            .iter()
            .any(|e| e.trace_id == diagnostic_trace.id()
                && e.phase == Stage::PersistenceIssue
                && e.fields.error_code == Some(ErrorCode::PersistenceRejected)
                && e.duration_ms.is_none()));
        connection
            .execute_batch("DROP TRIGGER reject_terminal;")
            .unwrap();
        drop(connection);
        manager.commit_terminal("running").await.unwrap();
        assert_eq!(
            manager.cached_snapshot().await.tasks[0].status,
            TaskStatus::Completed
        );
        assert!(manager.inner.lock().await.pending_terminal.is_empty());
        drop(manager);
        let (repo, loaded) = Persistence::open(&directory).unwrap();
        assert_eq!(loaded.tasks[0].status, TaskStatus::Completed);
        drop(repo);
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[tokio::test]
    async fn cancel_intent_is_signalled_before_database_ack_and_wins_completion() {
        let directory =
            std::env::temp_dir().join(format!("fluxdl-cancel-ack-{}", uuid::Uuid::new_v4()));
        let manager = isolated_manager(
            &directory,
            vec![fixture_task("running", TaskStatus::Downloading, &directory)],
        )
        .await;
        manager
            .inner
            .lock()
            .await
            .attempts
            .insert("running".into(), 1);
        let control = Arc::new(Control {
            id: "running".into(),
            attempt: 1,
            cancel: Cancellation::default(),
        });
        *manager.active.lock().await = Some(control.clone());
        let connection = rusqlite::Connection::open(directory.join("fluxdl.sqlite3")).unwrap();
        connection.execute_batch("BEGIN IMMEDIATE;").unwrap();
        let candidate = manager.clone();
        let job = tokio::spawn(async move { candidate.cancel("running").await });
        tokio::time::timeout(Duration::from_secs(1), async {
            while !control.cancel.is_cancelled() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(!job.is_finished());
        assert!(
            tokio::time::timeout(Duration::from_millis(100), manager.cached_snapshot())
                .await
                .is_ok()
        );
        connection.execute_batch("ROLLBACK;").unwrap();
        drop(connection);
        job.await.unwrap().unwrap();
        manager
            .finish_task("running", &control, Ok(()))
            .await
            .unwrap();
        assert_eq!(
            manager.cached_snapshot().await.tasks[0].status,
            TaskStatus::Cancelled
        );
        *manager.active.lock().await = None;
        drop(manager);
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[tokio::test]
    async fn explicit_file_refresh_updates_cache_and_progress_snapshot_does_not_resurrect_file() {
        let directory =
            std::env::temp_dir().join(format!("fluxdl-file-cache-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("media.mp4");
        std::fs::write(&path, b"fixture").unwrap();
        let mut task = fixture_task("completed", TaskStatus::Completed, &directory);
        task.final_path = Some(path.to_string_lossy().into_owned());
        task.file_exists = true;
        let manager = isolated_manager(&directory, vec![task]).await;
        assert!(manager.snapshot().await.tasks[0].file_exists);
        std::fs::remove_file(path).unwrap();
        assert!(!manager.snapshot().await.tasks[0].file_exists);
        assert!(!manager.cached_snapshot().await.tasks[0].file_exists);
        drop(manager);
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[tokio::test]
    async fn aborting_public_waiter_does_not_abort_accepted_settings_commit() {
        let directory =
            std::env::temp_dir().join(format!("fluxdl-owned-operation-{}", uuid::Uuid::new_v4()));
        let manager = isolated_manager(&directory, vec![]).await;
        let connection = rusqlite::Connection::open(directory.join("fluxdl.sqlite3")).unwrap();
        connection.execute_batch("BEGIN EXCLUSIVE;").unwrap();
        let settings = Settings {
            theme: "light".into(),
            download_dir: directory.join("media").to_string_lossy().into_owned(),
            ..Settings::default()
        };
        let candidate = manager.clone();
        let waiter = tokio::spawn(async move { candidate.update_settings(settings).await });
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if manager.critical.try_lock().is_err() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        waiter.abort();
        assert!(waiter.await.unwrap_err().is_cancelled());
        connection.execute_batch("ROLLBACK;").unwrap();
        drop(connection);
        tokio::time::timeout(Duration::from_secs(3), async {
            while manager.cached_snapshot().await.settings.theme != "light" {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        manager.writer.flush().await.unwrap();
        manager.writer.close().await.unwrap();
        let (repo, loaded) = Persistence::open(&directory).unwrap();
        assert_eq!(loaded.settings.theme, "light");
        drop(repo);
        drop(manager);
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[tokio::test]
    async fn late_file_check_cannot_overwrite_newer_refresh_or_retried_path() {
        let directory =
            std::env::temp_dir().join(format!("fluxdl-refresh-order-{}", uuid::Uuid::new_v4()));
        let mut task = fixture_task("completed", TaskStatus::Completed, &directory);
        task.final_path = Some("C:/fixture/old.mp4".into());
        task.file_exists = true;
        let manager = isolated_manager(&directory, vec![task]).await;
        let old = uuid::Uuid::new_v4();
        let new = uuid::Uuid::new_v4();
        let path = Some("C:/fixture/old.mp4".to_string());
        let mut inner = manager.inner.lock().await;
        inner.file_refresh.insert("completed".into(), old); // A has captured true, but has not applied it.
        inner.file_refresh.insert("completed".into(), new);
        assert!(inner.apply_availability("completed", &path, 0, new, false));
        assert!(!inner.apply_availability("completed", &path, 0, old, true));
        assert!(!inner.tasks[0].file_exists);
        inner.tasks[0].final_path = None;
        inner.versions.insert("completed".into(), 1);
        assert!(!inner.apply_availability("completed", &path, 0, new, true));
        drop(inner);
        drop(manager);
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[tokio::test]
    async fn closing_admission_rejects_settings_delete_retry_and_public_cancel() {
        let directory =
            std::env::temp_dir().join(format!("fluxdl-close-admission-{}", uuid::Uuid::new_v4()));
        let manager = isolated_manager(
            &directory,
            vec![
                fixture_task("completed", TaskStatus::Completed, &directory),
                fixture_task("failed", TaskStatus::Failed, &directory),
            ],
        )
        .await;
        manager.shutdown().await.unwrap();
        let error = manager
            .update_settings(Settings {
                download_dir: directory.to_string_lossy().into_owned(),
                ..Settings::default()
            })
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::ShuttingDown);
        assert_eq!(error.phase, ErrorPhase::Shutdown);
        assert!(manager.remove_history("completed").await.is_err());
        assert!(manager.retry("failed").await.is_err());
        assert!(manager.cancel("failed").await.is_err());
        assert_eq!(manager.cached_snapshot().await.tasks.len(), 2);
        drop(manager);
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[tokio::test]
    async fn committed_change_with_lost_ack_is_verified_before_publication() {
        let directory =
            std::env::temp_dir().join(format!("fluxdl-lost-ack-{}", uuid::Uuid::new_v4()));
        let manager = isolated_manager(
            &directory,
            vec![fixture_task("failed", TaskStatus::Failed, &directory)],
        )
        .await;
        manager.writer.lose_next_ack();
        manager.retry("failed").await.unwrap();
        assert_eq!(
            manager.cached_snapshot().await.tasks[0].status,
            TaskStatus::Queued
        );
        assert!(manager.inner.lock().await.pending_tasks.is_empty());
        let connection = rusqlite::Connection::open(directory.join("fluxdl.sqlite3")).unwrap();
        let raw: String = connection
            .query_row("SELECT document FROM tasks WHERE id='failed'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(
            serde_json::from_str::<DownloadTask>(&raw).unwrap().status,
            TaskStatus::Queued
        );
        drop(connection);
        drop(manager);
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[tokio::test]
    async fn opaque_storage_warning_survives_engine_detection_and_setting_changes() {
        let directory = std::env::temp_dir().join(format!(
            "fluxdl-storage-diagnostics-{}",
            uuid::Uuid::new_v4()
        ));
        let (mut repo, _) = Persistence::open(&directory).unwrap();
        let settings = Settings {
            download_dir: directory.join("media").to_string_lossy().into_owned(),
            ..Settings::default()
        };
        repo.save(&settings, &[]).unwrap();
        drop(repo);
        let connection = rusqlite::Connection::open(directory.join("fluxdl.sqlite3")).unwrap();
        connection
            .execute("INSERT INTO tasks VALUES('opaque','keep original raw')", [])
            .unwrap();
        drop(connection);
        let manager =
            Manager::init_with_diagnostics(directory.clone(), None, None, Diagnostics::disabled())
                .await
                .unwrap();
        assert!(manager
            .detect()
            .await
            .warnings
            .iter()
            .any(|warning| warning.contains("原始文档已保留")));
        let mut changed = settings;
        changed.density = "compact".into();
        manager.update_settings(changed).await.unwrap();
        assert!(manager
            .snapshot()
            .await
            .engines
            .warnings
            .iter()
            .any(|warning| warning.contains("原始文档已保留")));
        manager.shutdown().await.unwrap();
        drop(manager);
        let connection = rusqlite::Connection::open(directory.join("fluxdl.sqlite3")).unwrap();
        assert_eq!(
            connection
                .query_row("SELECT document FROM tasks WHERE id='opaque'", [], |r| {
                    r.get::<_, String>(0)
                })
                .unwrap(),
            "keep original raw"
        );
        drop(connection);
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[tokio::test]
    async fn shutdown_deadline_covers_blocked_writes_and_late_ack_does_not_stop_resumed_worker() {
        let directory =
            std::env::temp_dir().join(format!("fluxdl-shutdown-deadline-{}", uuid::Uuid::new_v4()));
        let manager = isolated_manager(
            &directory,
            vec![fixture_task("queued", TaskStatus::Queued, &directory)],
        )
        .await;
        let connection = rusqlite::Connection::open(directory.join("fluxdl.sqlite3")).unwrap();
        connection.execute_batch("BEGIN EXCLUSIVE;").unwrap();
        let start = Instant::now();
        assert!(manager
            .shutdown_with_timeout(Duration::from_millis(80))
            .await
            .is_err());
        assert!(start.elapsed() < Duration::from_secs(2));
        assert!(!manager.shutting_down.load(Ordering::SeqCst));
        assert!(!manager.stop_worker.load(Ordering::SeqCst));
        connection.execute_batch("ROLLBACK;").unwrap();
        drop(connection);
        let settings = Settings {
            theme: "light".into(),
            download_dir: directory.join("media").to_string_lossy().into_owned(),
            ..Settings::default()
        };
        tokio::time::timeout(Duration::from_secs(3), manager.update_settings(settings))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(manager.cached_snapshot().await.settings.theme, "light");
        assert!(!manager.worker_finished.load(Ordering::SeqCst));
        assert!(!manager.progress_finished.load(Ordering::SeqCst));
        manager.shutdown().await.unwrap();
        drop(manager);
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[tokio::test]
    async fn snapshot_flush_covers_pending_logs_and_only_new_lines_get_new_revisions() {
        let directory =
            std::env::temp_dir().join(format!("fluxdl-event-snapshot-{}", uuid::Uuid::new_v4()));
        let manager = isolated_manager(
            &directory,
            vec![fixture_task("running", TaskStatus::Downloading, &directory)],
        )
        .await;
        {
            let mut inner = manager.inner.lock().await;
            for line in ["A", "B", "C", "D"] {
                let line = push_log(&mut inner.tasks[0], line.into());
                inner.events.append_log("running", &line);
            }
        }
        let snapshot = manager.snapshot().await;
        assert_eq!(snapshot.tasks[0].logs, ["A", "B", "C", "D"]);
        let emitted = std::mem::take(&mut *manager.test_events.lock().await);
        assert_eq!(emitted.len(), 1);
        assert!(emitted
            .iter()
            .all(|event| event.revision <= snapshot.revision
                && event.session_id == snapshot.session_id));
        match &emitted[0].change {
            AppChange::TaskProgress { logs_append, .. } => {
                assert_eq!(logs_append, &["A", "B", "C", "D"])
            }
            other => panic!("unexpected {other:?}"),
        }
        {
            let mut inner = manager.inner.lock().await;
            let line = push_log(&mut inner.tasks[0], "E".into());
            inner.events.append_log("running", &line);
        }
        manager.publish_progress().await;
        let emitted = std::mem::take(&mut *manager.test_events.lock().await);
        assert_eq!(emitted.len(), 1);
        assert!(emitted[0].revision > snapshot.revision);
        match &emitted[0].change {
            AppChange::TaskProgress { logs_append, .. } => assert_eq!(logs_append, &["E"]),
            other => panic!("unexpected {other:?}"),
        }
        drop(manager);
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[tokio::test]
    async fn terminal_then_immediate_retry_keeps_both_semantic_events_and_tail_index() {
        let directory =
            std::env::temp_dir().join(format!("fluxdl-event-retry-{}", uuid::Uuid::new_v4()));
        let manager = isolated_manager(
            &directory,
            vec![
                fixture_task("running", TaskStatus::Downloading, &directory),
                fixture_task("other", TaskStatus::Completed, &directory),
            ],
        )
        .await;
        {
            let mut inner = manager.inner.lock().await;
            inner.attempts.insert("running".into(), 1);
            let line = push_log(&mut inner.tasks[0], "duplicate text".into());
            inner.events.append_log("running", &line);
        }
        let control = Control {
            id: "running".into(),
            attempt: 1,
            cancel: Cancellation::default(),
        };
        manager
            .finish_task(
                "running",
                &control,
                Err(AppError::new(
                    ErrorCode::ProcessExit,
                    ErrorPhase::Download,
                    "fixture failure",
                )),
            )
            .await
            .unwrap();
        manager.retry("running").await.unwrap();
        manager.publish_progress().await;
        let emitted = std::mem::take(&mut *manager.test_events.lock().await);
        let upserts = emitted
            .iter()
            .filter_map(|event| match &event.change {
                AppChange::TaskUpsert { task, index } => {
                    Some((event.revision, task.status, *index, task.logs.clone()))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(upserts.len(), 2);
        assert_eq!((upserts[0].1, upserts[0].2), (TaskStatus::Failed, 0));
        assert_eq!((upserts[1].1, upserts[1].2), (TaskStatus::Queued, 1));
        assert!(upserts[0].0 < upserts[1].0);
        assert_eq!(upserts[0].3, ["duplicate text"]);
        assert!(upserts[1].3.is_empty());
        assert!(!emitted
            .iter()
            .any(|event| matches!(event.change, AppChange::TaskProgress { .. })));
        drop(manager);
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[tokio::test]
    async fn one_dirty_progress_serializes_only_runtime_and_rejects_old_attempt_admission() {
        let directory =
            std::env::temp_dir().join(format!("fluxdl-event-runtime-{}", uuid::Uuid::new_v4()));
        let tasks = (0..100)
            .map(|index| {
                fixture_task(
                    &format!("task-{index}"),
                    if index == 99 {
                        TaskStatus::Downloading
                    } else {
                        TaskStatus::Completed
                    },
                    &directory,
                )
            })
            .collect();
        let manager = isolated_manager(&directory, tasks).await;
        {
            let mut inner = manager.inner.lock().await;
            inner.attempts.insert("task-99".into(), 2);
            assert!(!inner.accepts_progress("task-99", 1));
            assert!(inner.accepts_progress("task-99", 2));
            inner.tasks[99].downloaded_bytes = 42;
            inner.events.mark_progress("task-99");
        }
        manager.publish_progress().await;
        let emitted = std::mem::take(&mut *manager.test_events.lock().await);
        assert_eq!(emitted.len(), 1);
        let value = serde_json::to_value(&emitted[0]).unwrap();
        assert_eq!(value["change"]["taskId"], "task-99");
        assert_eq!(value["change"]["downloadedBytes"], 42);
        for forbidden in ["tasks", "task", "media", "formats", "title", "logs"] {
            assert!(value["change"].get(forbidden).is_none());
        }
        let full = serde_json::to_vec(&manager.cached_snapshot().await)
            .unwrap()
            .len();
        assert!(serde_json::to_vec(&emitted[0]).unwrap().len() * 10 < full);
        drop(manager);
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[tokio::test]
    async fn details_refreshes_only_requested_file_and_flushes_global_pending_progress() {
        let directory =
            std::env::temp_dir().join(format!("fluxdl-event-detail-{}", uuid::Uuid::new_v4()));
        let mut requested = fixture_task("requested", TaskStatus::Completed, &directory);
        requested.final_path = Some(directory.join("missing.mp4").to_string_lossy().into_owned());
        requested.file_exists = true;
        let mut other = fixture_task("other", TaskStatus::Completed, &directory);
        other.final_path = Some(
            directory
                .join("also-missing.mp4")
                .to_string_lossy()
                .into_owned(),
        );
        other.file_exists = true;
        let manager = isolated_manager(
            &directory,
            vec![
                requested,
                other,
                fixture_task("running", TaskStatus::Downloading, &directory),
            ],
        )
        .await;
        {
            let mut inner = manager.inner.lock().await;
            let line = push_log(&mut inner.tasks[2], "C".into());
            inner.events.append_log("running", &line);
        }
        let detail = manager.details("requested").await.unwrap();
        assert!(!detail.task.file_exists);
        let cached = manager.cached_snapshot().await;
        assert!(
            cached.tasks[1].file_exists,
            "detail unexpectedly checked unrelated historical file"
        );
        assert_eq!(detail.revision, cached.revision);
        assert_eq!(detail.session_id, cached.session_id);
        let value = serde_json::to_value(&detail).unwrap();
        assert!(value.get("sessionId").is_some());
        assert!(value.get("revision").is_some());
        assert!(value.get("task").is_some());
        assert!(value.get("tasks").is_none());
        assert!(manager.details("deleted").await.is_err());
        let emitted = manager.test_events.lock().await.clone();
        assert!(emitted
            .iter()
            .all(|event| event.revision <= detail.revision));
        assert!(emitted.iter().any(|event|matches!(&event.change,AppChange::TaskProgress{task_id,logs_append,..}if task_id=="running"&&logs_append==&["C"])));
        drop(manager);
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[tokio::test]
    async fn full_file_refresh_publishes_one_compact_batch_with_snapshot_revision() {
        let directory =
            std::env::temp_dir().join(format!("fluxdl-event-files-{}", uuid::Uuid::new_v4()));
        let tasks = (0..100)
            .map(|index| {
                let mut task =
                    fixture_task(&format!("task-{index}"), TaskStatus::Completed, &directory);
                task.final_path = Some(
                    directory
                        .join(format!("missing-{index}.mp4"))
                        .to_string_lossy()
                        .into_owned(),
                );
                task.file_exists = true;
                task
            })
            .collect();
        let manager = isolated_manager(&directory, tasks).await;
        let snapshot = manager.snapshot().await;
        assert!(snapshot.tasks.iter().all(|task| !task.file_exists));
        let emitted = manager.test_events.lock().await.clone();
        assert_eq!(emitted.len(), 1);
        assert_eq!(emitted[0].revision, snapshot.revision);
        match &emitted[0].change {
            AppChange::FileAvailabilityUpdated { files } => {
                assert_eq!(files.len(), 100);
                assert!(files.iter().all(|file| !file.file_exists));
            }
            other => panic!("unexpected {other:?}"),
        };
        drop(manager);
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[tokio::test]
    async fn rejected_setting_emits_diagnostics_but_never_unconfirmed_settings() {
        let directory =
            std::env::temp_dir().join(format!("fluxdl-event-error-{}", uuid::Uuid::new_v4()));
        let manager = isolated_manager(&directory, vec![]).await;
        let connection = rusqlite::Connection::open(directory.join("fluxdl.sqlite3")).unwrap();
        connection.execute_batch("CREATE TRIGGER reject_settings BEFORE INSERT ON app_settings BEGIN SELECT RAISE(ABORT,'fixture failure'); END;").unwrap();
        let error = manager
            .update_settings(Settings {
                theme: "light".into(),
                download_dir: directory.join("media").to_string_lossy().into_owned(),
                ..Settings::default()
            })
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::PersistenceRejected);
        assert_eq!(error.phase, ErrorPhase::Persist);
        let emitted = manager.test_events.lock().await.clone();
        assert!(emitted.iter().any(|event|matches!(&event.change,AppChange::DiagnosticsUpdated{engines} if engines.warnings.iter().any(|warning|warning.contains("持久化")))));
        assert!(!emitted
            .iter()
            .any(|event| matches!(event.change, AppChange::SettingsUpdated { .. })));
        assert_eq!(manager.cached_snapshot().await.settings.theme, "dark");
        drop(connection);
        drop(manager);
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[tokio::test]
    async fn dropped_only_terminal_is_detectable_by_checkpoint_and_full_recovery() {
        let directory =
            std::env::temp_dir().join(format!("fluxdl-event-tail-loss-{}", uuid::Uuid::new_v4()));
        let manager = isolated_manager(
            &directory,
            vec![fixture_task("running", TaskStatus::Downloading, &directory)],
        )
        .await;
        manager
            .inner
            .lock()
            .await
            .attempts
            .insert("running".into(), 1);
        let before = manager.snapshot().await;
        let control = Control {
            id: "running".into(),
            attempt: 1,
            cancel: Cancellation::default(),
        };
        manager
            .finish_task(
                "running",
                &control,
                Err(AppError::new(
                    ErrorCode::ProcessExit,
                    ErrorPhase::Download,
                    "fixture final failure",
                )),
            )
            .await
            .unwrap();
        manager.emit().await;
        let dropped = std::mem::take(&mut *manager.test_events.lock().await);
        assert_eq!(dropped.len(), 1);
        assert!(matches!(dropped[0].change, AppChange::TaskUpsert { .. }));
        assert_eq!(dropped[0].revision, before.revision + 1);
        manager
            .inner
            .lock()
            .await
            .events
            .capture(AppChange::Checkpoint);
        manager.emit().await;
        let checkpoint = std::mem::take(&mut *manager.test_events.lock().await);
        assert_eq!(checkpoint.len(), 1);
        assert!(matches!(checkpoint[0].change, AppChange::Checkpoint));
        assert!(
            checkpoint[0].revision > before.revision + 1,
            "lost terminal must produce a detectable revision gap"
        );
        let recovered = manager.snapshot().await;
        assert_eq!(recovered.revision, checkpoint[0].revision);
        assert_eq!(recovered.tasks[0].status, TaskStatus::Failed);
        assert!(!manager.inner.lock().await.accepts_progress("running", 1));
        drop(manager);
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[tokio::test]
    async fn event_checkpoint_keeps_running_while_database_and_critical_gate_are_busy() {
        let directory =
            std::env::temp_dir().join(format!("fluxdl-event-independent-{}", uuid::Uuid::new_v4()));
        let manager = isolated_manager(&directory, vec![]).await;
        let connection = rusqlite::Connection::open(directory.join("fluxdl.sqlite3")).unwrap();
        connection.execute_batch("BEGIN EXCLUSIVE;").unwrap();
        let updating = manager.clone();
        let download_dir = directory.join("media").to_string_lossy().into_owned();
        let update = tokio::spawn(async move {
            updating
                .update_settings(Settings {
                    download_dir,
                    theme: "light".into(),
                    ..Settings::default()
                })
                .await
        });
        tokio::time::timeout(Duration::from_secs(2), async {
            while manager.critical.try_lock().is_ok() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        manager.event_finished.store(false, Ordering::SeqCst);
        let event_manager = manager.clone();
        let events = tokio::spawn(async move {
            event_manager.event_worker(Duration::from_millis(50)).await;
        });
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if manager
                    .test_events
                    .lock()
                    .await
                    .iter()
                    .any(|event| matches!(event.change, AppChange::Checkpoint))
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("checkpoint waited for database I/O");
        assert!(!update.is_finished());
        connection.execute_batch("ROLLBACK;").unwrap();
        drop(connection);
        update.await.unwrap().unwrap();
        manager.stop_worker.store(true, Ordering::SeqCst);
        manager.event_wake.notify_one();
        events.await.unwrap();
        drop(manager);
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[tokio::test]
    async fn snapshot_and_append_share_identical_bounded_duplicate_log_history() {
        let directory =
            std::env::temp_dir().join(format!("fluxdl-event-log-bounds-{}", uuid::Uuid::new_v4()));
        let manager = isolated_manager(
            &directory,
            vec![fixture_task("running", TaskStatus::Downloading, &directory)],
        )
        .await;
        {
            let mut inner = manager.inner.lock().await;
            for _ in 0..105 {
                let line = push_log(&mut inner.tasks[0], "中".repeat(1800));
                inner.events.append_log("running", &line);
            }
        }
        let snapshot = manager.snapshot().await;
        assert_eq!(snapshot.tasks[0].logs.len(), 100);
        assert!(snapshot.tasks[0]
            .logs
            .iter()
            .all(|line| line.chars().count() == 1600));
        let emitted = manager.test_events.lock().await.clone();
        assert_eq!(emitted.len(), 1);
        match &emitted[0].change {
            AppChange::TaskProgress { logs_append, .. } => {
                assert_eq!(logs_append, &snapshot.tasks[0].logs)
            }
            other => panic!("unexpected {other:?}"),
        };
        drop(manager);
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn typed_options_reject_cli_injection() {
        assert!(validate_options("video", "1080").is_ok());
        assert!(validate_options("--exec", "best").is_err());
        assert!(validate_options("video", "1080;calc").is_err());
        assert_eq!(
            validate_options("--exec", "best").unwrap_err().code,
            ErrorCode::InvalidInput
        );
    }
    #[tokio::test]
    async fn legacy_diagnostics_are_safe_in_snapshot_and_details_without_rewriting_the_row() {
        let directory =
            std::env::temp_dir().join(format!("fluxdl-legacy-secrets-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let mut legacy = fixture_task("legacy", TaskStatus::Completed, &directory);
        legacy.media.url = "https://example.com/source?intentional=user-query#copy-source".into();
        legacy.logs = vec![
            "Authorization: Bearer LEGACY_LOG_SECRET".into(),
            " LEGACY_FOLDED_SECRET".into(),
            "public reason".into(),
            "Cookie: sid=LEGACY_COOKIE_SECRET".into(),
        ];
        legacy.error =
            Some("Proxy-Authorization: Basic LEGACY_ERROR_SECRET\n LEGACY_ERROR_FOLDED".into());
        legacy.media.warnings = vec![
            "Set-Cookie: LEGACY_WARNING_SECRET".into(),
            " LEGACY_WARNING_FOLDED".into(),
            "ordinary warning".into(),
        ];
        let raw = serde_json::to_string(&legacy).unwrap();
        let connection = rusqlite::Connection::open(directory.join("fluxdl.sqlite3")).unwrap();
        connection.execute_batch("CREATE TABLE app_settings(id INTEGER PRIMARY KEY,document TEXT NOT NULL); CREATE TABLE tasks(id TEXT PRIMARY KEY,document TEXT NOT NULL); PRAGMA user_version=1;").unwrap();
        connection
            .execute("INSERT INTO tasks VALUES('legacy',?1)", [&raw])
            .unwrap();
        drop(connection);
        let manager =
            Manager::init_with_diagnostics(directory.clone(), None, None, Diagnostics::disabled())
                .await
                .unwrap();
        let snapshot = manager.snapshot().await;
        let details = manager.details("legacy").await.unwrap();
        for visible in [
            serde_json::to_string(&snapshot).unwrap(),
            serde_json::to_string(&details).unwrap(),
        ] {
            assert!(
                !visible.contains("LEGACY_"),
                "old diagnostic secret entered IPC"
            );
        }
        assert_eq!(
            details.task.media.url, legacy.media.url,
            "source URL remains available for intentional copy/retry"
        );
        assert!(details.task.logs.iter().any(|line| line == "public reason"));
        assert_eq!(
            details.task.media.warnings.last().unwrap(),
            "ordinary warning"
        );
        manager.shutdown().await.unwrap();
        drop(manager);
        let connection = rusqlite::Connection::open(directory.join("fluxdl.sqlite3")).unwrap();
        let retained: String = connection
            .query_row("SELECT document FROM tasks WHERE id='legacy'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(
            retained, raw,
            "read-only display filtering rewrote a legacy row"
        );
        drop(connection);
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[tokio::test]
    async fn download_markers_reject_old_attempts_wrong_pipes_and_repeated_callbacks() {
        use crate::process::OutputStream;
        let directory =
            std::env::temp_dir().join(format!("fluxdl-markers-{}", uuid::Uuid::new_v4()));
        let id = uuid::Uuid::new_v4().to_string();
        let manager = isolated_manager(
            &directory,
            vec![fixture_task(&id, TaskStatus::Resolving, &directory)],
        )
        .await;
        manager.inner.lock().await.attempts.insert(id.clone(), 2);
        let trace = manager.diagnostics.trace(None, None, None).with_task(&id);
        trace.begin(Stage::DownloadBeforeDl);
        trace.begin(Stage::DownloadFirstProgress);
        manager.diagnostics.attach(&id, trace.clone());
        let mut tracker = RuntimeTracker::default();
        for (attempt, stream) in [(1, OutputStream::Stdout), (2, OutputStream::Stderr)] {
            manager
                .apply_download_event(
                    &id,
                    attempt,
                    &mut tracker,
                    DownloadEvent::BeforeDownload {
                        stream,
                        sequence: 999,
                        observed_at: Instant::now(),
                    },
                )
                .await
                .unwrap();
        }
        assert!(!manager
            .diagnostics
            .query(&crate::diagnostics::Filter {
                trace_id: Some(trace.id().into()),
                ..Default::default()
            })
            .unwrap()
            .events
            .iter()
            .any(|e| e.phase == Stage::DownloadBeforeDl && e.status == Status::Ok));
        for _ in 0..2 {
            manager
                .apply_download_event(
                    &id,
                    2,
                    &mut tracker,
                    DownloadEvent::BeforeDownload {
                        stream: OutputStream::Stdout,
                        sequence: 1,
                        observed_at: Instant::now(),
                    },
                )
                .await
                .unwrap();
        }
        manager
            .apply_download_event(
                &id,
                2,
                &mut tracker,
                DownloadEvent::Progress {
                    stream: OutputStream::Stdout,
                    sequence: 2,
                    fact: engine::ParsedProgress {
                        event: engine::ProgressEvent::Download {
                            downloaded_bytes: 1,
                            total_bytes: Some(10),
                            speed: None,
                            eta: None,
                            progress: Some(10.),
                        },
                        stream_id: None,
                        finished: false,
                    },
                },
            )
            .await
            .unwrap();
        let events = manager
            .diagnostics
            .query(&crate::diagnostics::Filter {
                trace_id: Some(trace.id().into()),
                ..Default::default()
            })
            .unwrap()
            .events;
        for phase in [
            Stage::DownloadBeforeDl,
            Stage::DownloadBeforeDlToProgress,
            Stage::DownloadFirstProgress,
        ] {
            assert_eq!(
                events
                    .iter()
                    .filter(|e| e.phase == phase && e.status == Status::Ok)
                    .count(),
                1
            );
        }
        manager.writer.close().await.unwrap();
        drop(manager);
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[tokio::test]
    async fn typed_runtime_new_stream_resets_with_full_upsert_and_rejects_old_attempts_and_exits() {
        use crate::{
            engine::{ParsedProgress, ProgressEvent},
            process::OutputStream,
        };
        let directory =
            std::env::temp_dir().join(format!("fluxdl-stream-events-{}", uuid::Uuid::new_v4()));
        let manager = isolated_manager(
            &directory,
            vec![fixture_task("running", TaskStatus::Resolving, &directory)],
        )
        .await;
        manager
            .inner
            .lock()
            .await
            .attempts
            .insert("running".into(), 1);
        let mut tracker = RuntimeTracker::default();
        let progress = |sequence, name: &str, downloaded_bytes, total: u64, finished| {
            DownloadEvent::Progress {
                stream: OutputStream::Stdout,
                sequence,
                fact: ParsedProgress {
                    event: ProgressEvent::Download {
                        downloaded_bytes,
                        total_bytes: Some(total),
                        speed: None,
                        eta: None,
                        progress: Some(downloaded_bytes as f64 / total as f64 * 100.0),
                    },
                    stream_id: Some(name.into()),
                    finished,
                },
            }
        };
        manager
            .apply_download_event(
                "running",
                1,
                &mut tracker,
                progress(1, "video", 100, 100, true),
            )
            .await
            .unwrap();
        manager
            .apply_download_event(
                "running",
                1,
                &mut tracker,
                DownloadEvent::Log {
                    stream: OutputStream::Stdout,
                    sequence: 2,
                    line: "same log".into(),
                },
            )
            .await
            .unwrap();
        manager
            .apply_download_event(
                "running",
                1,
                &mut tracker,
                DownloadEvent::Progress {
                    stream: OutputStream::Stdout,
                    sequence: 3,
                    fact: ParsedProgress {
                        event: ProgressEvent::Postprocessing,
                        stream_id: None,
                        finished: false,
                    },
                },
            )
            .await
            .unwrap();
        manager
            .apply_download_event(
                "running",
                1,
                &mut tracker,
                progress(4, "audio", 1, 10, false),
            )
            .await
            .unwrap();
        manager.publish_progress().await;
        let published = manager.test_events.lock().await.clone();
        assert!(
            matches!(&published[0].change, AppChange::TaskUpsert { task, .. } if task.status == TaskStatus::Downloading && task.progress == Some(10.0) && task.downloaded_bytes == 1 && task.logs == ["same log"])
        );
        assert_eq!(
            published.len(),
            1,
            "full stream upsert left duplicate pending log appends"
        );
        let before = manager.inner.lock().await.next_seq;
        manager
            .apply_download_event(
                "running",
                1,
                &mut tracker,
                progress(5, "video", 100, 100, true),
            )
            .await
            .unwrap();
        assert_eq!(
            manager.inner.lock().await.next_seq,
            before,
            "retired stream created a state version"
        );
        manager
            .apply_download_event(
                "running",
                1,
                &mut tracker,
                progress(6, "audio", 1, 20, false),
            )
            .await
            .unwrap();
        assert_eq!(
            manager.cached_snapshot().await.tasks[0].progress,
            Some(5.0),
            "an estimate correction was replaced with max progress"
        );
        let control = Control {
            id: "running".into(),
            attempt: 1,
            cancel: Cancellation::default(),
        };
        manager
            .finish_task(
                "running",
                &control,
                Err(AppError::new(
                    ErrorCode::ProcessExit,
                    ErrorPhase::Download,
                    "expected fixture failure",
                )),
            )
            .await
            .unwrap();
        manager.retry("running").await.unwrap();
        let before = manager.inner.lock().await.next_seq;
        manager
            .apply_download_event(
                "running",
                1,
                &mut tracker,
                DownloadEvent::Log {
                    stream: OutputStream::Stdout,
                    sequence: 7,
                    line: "late previous attempt".into(),
                },
            )
            .await
            .unwrap();
        manager
            .finish_task("running", &control, Ok(()))
            .await
            .unwrap();
        assert_eq!(
            manager.inner.lock().await.next_seq,
            before,
            "late callback changed retry-queued task"
        );
        assert_eq!(
            manager.cached_snapshot().await.tasks[0].status,
            TaskStatus::Queued
        );
        let claimed = manager.claim_task().await.unwrap().unwrap();
        assert_eq!(claimed.3.attempt, 2);
        let before = manager.inner.lock().await.next_seq;
        manager
            .apply_download_event("running", 1, &mut tracker, progress(8, "old", 9, 10, false))
            .await
            .unwrap();
        manager.settle_download("running", &control, Ok(())).await;
        assert_eq!(
            manager.active.lock().await.as_ref().unwrap().attempt,
            2,
            "old cleanup cleared the new control"
        );
        assert_eq!(
            manager.inner.lock().await.next_seq,
            before,
            "old attempt changed the newly claimed task"
        );
        assert_eq!(
            manager.cached_snapshot().await.tasks[0].status,
            TaskStatus::Resolving
        );
        assert_eq!(
            manager.task("absent").await.unwrap_err().code,
            ErrorCode::TaskNotFound
        );
        drop(manager);
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[tokio::test]
    async fn claim_failure_keeps_attempt_and_success_commits_attempt_with_resolving() {
        let directory =
            std::env::temp_dir().join(format!("fluxdl-claim-attempt-{}", uuid::Uuid::new_v4()));
        let manager = isolated_manager(
            &directory,
            vec![fixture_task("queued", TaskStatus::Queued, &directory)],
        )
        .await;
        manager
            .inner
            .lock()
            .await
            .attempts
            .insert("queued".into(), 4);
        let connection = rusqlite::Connection::open(directory.join("fluxdl.sqlite3")).unwrap();
        connection.execute_batch("CREATE TRIGGER reject_claim BEFORE INSERT ON tasks WHEN json_extract(NEW.document,'$.status')='resolving' BEGIN SELECT RAISE(ABORT,'fixture claim failure'); END;").unwrap();
        let error = match manager.claim_task().await {
            Err(error) => error,
            Ok(_) => panic!("claim was not rejected"),
        };
        assert_eq!(error.code, ErrorCode::PersistenceRejected);
        {
            let inner = manager.inner.lock().await;
            assert_eq!(inner.tasks[0].status, TaskStatus::Queued);
            assert_eq!(inner.attempts.get("queued"), Some(&4));
            assert!(inner.pending_tasks.is_empty());
        }
        connection
            .execute_batch("DROP TRIGGER reject_claim;")
            .unwrap();
        drop(connection);
        let selected = manager.claim_task().await.unwrap().unwrap();
        assert_eq!(selected.3.attempt, 5);
        {
            let inner = manager.inner.lock().await;
            assert_eq!(inner.tasks[0].status, TaskStatus::Resolving);
            assert_eq!(inner.attempts.get("queued"), Some(&5));
        }
        drop(manager);
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[cfg(windows)]
    #[tokio::test]
    async fn unproven_process_cleanup_remains_active_across_shutdown_timeout_until_real_reap() {
        use crate::process::{self, ProcessSpec};
        use windows_sys::Win32::{
            Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT},
            System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE},
        };
        struct ProcessHandle(HANDLE);
        impl Drop for ProcessHandle {
            fn drop(&mut self) {
                unsafe {
                    CloseHandle(self.0);
                }
            }
        }
        let directory =
            std::env::temp_dir().join(format!("fluxdl-cleanup-proof-{}", uuid::Uuid::new_v4()));
        let task = fixture_task("running", TaskStatus::Resolving, &directory);
        let manager = isolated_manager(
            &directory,
            vec![
                task.clone(),
                fixture_task("waiting", TaskStatus::Queued, &directory),
            ],
        )
        .await;
        manager
            .inner
            .lock()
            .await
            .attempts
            .insert(task.id.clone(), 1);
        let cancel = Cancellation::default();
        let proof = cancel.hold_cleanup();
        let control = Arc::new(Control {
            id: task.id.clone(),
            attempt: 1,
            cancel: cancel.clone(),
        });
        *manager.active.lock().await = Some(control.clone());
        let powershell =
            PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into()))
                .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let process = process::start_download(ProcessSpec {
            executable: powershell,
            args: vec!["-NoProfile".into(), "-NonInteractive".into(), "-Command".into(), r#"Write-Output '__FLUX_PROGRESS__{"downloaded_bytes":1,"total_bytes":100,"filename":"video","status":"downloading"}'; Start-Sleep -Seconds 60"#.into()],
            directory: directory.clone(),
        }, cancel.clone());
        let download = engine::EngineDownload::from_process(process);
        let driving_manager = manager.clone();
        let driving_control = control.clone();
        let driving = tokio::spawn(async move {
            let result = driving_manager
                .drive_download(&task, &driving_control, download)
                .await;
            let code = result.as_ref().err().map(|error| error.code);
            driving_manager
                .settle_download(&task.id, &driving_control, result)
                .await;
            code
        });
        tokio::time::timeout(Duration::from_secs(5), async {
            while manager.cached_snapshot().await.tasks[0].status != TaskStatus::Downloading {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let process_handle = ProcessHandle(unsafe {
            OpenProcess(PROCESS_SYNCHRONIZE, 0, cancel.process_id().unwrap())
        });
        assert!(!process_handle.0.is_null());
        let error = manager
            .shutdown_with_timeout(Duration::from_millis(150))
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::ProcessTimeout);
        tokio::time::timeout(Duration::from_secs(3), async {
            while proof.attempts() == 0 || manager.inner.lock().await.process_error.is_none() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let snapshot = manager.snapshot().await;
        assert_eq!(snapshot.tasks[0].status, TaskStatus::Cancelling);
        // Shutdown may already have durably cancelled queued work. A new,
        // explicit retry after the failed exit must still wait for this owner.
        assert!(matches!(
            snapshot.tasks[1].status,
            TaskStatus::Queued | TaskStatus::Cancelled
        ));
        if snapshot.tasks[1].status == TaskStatus::Cancelled {
            manager.retry("waiting").await.unwrap();
        }
        assert_eq!(
            manager.cached_snapshot().await.tasks[1].status,
            TaskStatus::Queued
        );
        assert!(snapshot
            .engines
            .warnings
            .iter()
            .any(|warning| warning.contains("仍由原 owner 隔离")));
        assert!(manager.active.lock().await.is_some());
        assert!(!manager.shutting_down.load(Ordering::SeqCst));
        assert!(
            !driving.is_finished(),
            "Manager settled an owner with no stop proof"
        );
        assert!(
            manager.claim_task().await.unwrap().is_none(),
            "unproven cleanup allowed another claim"
        );
        assert_eq!(
            unsafe { WaitForSingleObject(process_handle.0, 0) },
            WAIT_TIMEOUT
        );
        manager.cancel("running").await.unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            while proof.attempts() < 2 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert!(!driving.is_finished());
        proof.release();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(8), driving)
                .await
                .unwrap()
                .unwrap(),
            Some(ErrorCode::ProcessCleanup)
        );
        assert_eq!(
            unsafe { WaitForSingleObject(process_handle.0, 0) },
            WAIT_OBJECT_0
        );
        tokio::time::timeout(Duration::from_secs(5), async {
            while manager.cached_snapshot().await.tasks[1].status != TaskStatus::Failed {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let snapshot = manager.snapshot().await;
        assert_eq!(
            snapshot.tasks[0].status,
            TaskStatus::Failed,
            "cleanup failure was presented as a successful cancellation"
        );
        assert!(manager.active.lock().await.is_none());
        assert!(manager.inner.lock().await.process_error.is_none());
        let events = manager.test_events.lock().await.clone();
        assert_eq!(events.iter().filter(|event| matches!(&event.change, AppChange::TaskUpsert { task, .. } if task.id == "waiting" && task.status == TaskStatus::Resolving)).count(), 1, "scheduler did not resume with exactly one claim");
        manager.shutdown().await.unwrap();
        drop(manager);
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[tokio::test]
    async fn failure_before_terminal_candidate_keeps_active_until_actual_ack_after_repair() {
        let directory =
            std::env::temp_dir().join(format!("fluxdl-terminal-budget-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("finished.mp4");
        std::fs::write(&path, b"fixture completed media").unwrap();
        let mut task = fixture_task("running", TaskStatus::Downloading, &directory);
        task.final_path = Some(path.to_string_lossy().into_owned());
        validate_final_path(task.final_path.as_deref().unwrap(), &task.output_dir).unwrap();
        let manager = isolated_manager(&directory, vec![task]).await;
        let control = Arc::new(Control {
            id: "running".into(),
            attempt: 1,
            cancel: Cancellation::default(),
        });
        let previous_seq = {
            let mut inner = manager.inner.lock().await;
            inner.attempts.insert("running".into(), 1);
            let previous = inner.next_seq;
            inner.next_seq = i64::MAX;
            previous
        };
        *manager.active.lock().await = Some(control.clone());
        let settling_manager = manager.clone();
        let settling = tokio::spawn(async move {
            settling_manager
                .settle_download("running", &control, Ok(()))
                .await
        });
        tokio::time::timeout(Duration::from_secs(2), async {
            while manager.inner.lock().await.persistence_error.is_none() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert!(!settling.is_finished());
        assert!(manager.active.lock().await.is_some());
        assert!(manager.inner.lock().await.pending_terminal.is_empty());
        assert_eq!(
            manager.snapshot().await.tasks[0].status,
            TaskStatus::Downloading
        );
        let connection = rusqlite::Connection::open(directory.join("fluxdl.sqlite3")).unwrap();
        let raw: String = connection
            .query_row("SELECT document FROM tasks WHERE id='running'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(
            serde_json::from_str::<DownloadTask>(&raw).unwrap().status,
            TaskStatus::Downloading
        );
        drop(connection);
        // Repair only the injected in-memory limit. The next real transaction
        // must create and confirm a candidate before the owner is released.
        manager.inner.lock().await.next_seq = previous_seq;
        tokio::time::timeout(Duration::from_secs(2), settling)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            manager.snapshot().await.tasks[0].status,
            TaskStatus::Completed
        );
        assert!(manager.active.lock().await.is_none());
        assert!(manager.inner.lock().await.persistence_error.is_none());
        drop(manager);
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn directory_and_final_path_must_be_real_and_contained() {
        let root = std::env::temp_dir().join(format!("fluxdl-path-test-{}", uuid::Uuid::new_v4()));
        let out = root.join("out");
        let dir = writable_directory(out.to_str().unwrap()).unwrap();
        let good = dir.join("中文名.mp4");
        std::fs::write(&good, b"media").unwrap();
        let other = root.join("outside.mp4");
        std::fs::write(&other, b"media").unwrap();
        let executable = dir.join("unsafe.exe");
        std::fs::write(&executable, b"fixture only").unwrap();
        assert!(validate_final_path(good.to_str().unwrap(), dir.to_str().unwrap()).is_ok());
        assert!(validate_final_path(other.to_str().unwrap(), dir.to_str().unwrap()).is_err());
        assert!(validate_final_path(executable.to_str().unwrap(), dir.to_str().unwrap()).is_err());
        assert!(writable_directory("relative-dir").is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn status_retry_and_running_semantics() {
        assert!(TaskStatus::Downloading.running());
        assert!(!TaskStatus::Queued.running());
        assert!(TaskStatus::Interrupted.retryable());
        assert!(!TaskStatus::Completed.retryable());
        assert!(!TaskStatus::Cancelled.active());
    }
    #[tokio::test]
    #[ignore = "scripts/test-queue.ps1 compiles the deterministic fixture engine first"]
    async fn worker_lifecycle_fixture() {
        let engine_path = std::env::var("FLUXDL_TEST_ENGINE").expect("Run scripts/test-queue.ps1");
        let root =
            std::env::temp_dir().join(format!("fluxdl-worker-test-{}", uuid::Uuid::new_v4()));
        let data = root.join("data");
        let output = root.join("downloads");
        let (mut store, _) = Persistence::open(&data).unwrap();
        let settings = Settings {
            engine_path,
            download_dir: output.to_string_lossy().into_owned(),
            theme: "light".into(),
            ..Settings::default()
        };
        store.save(&settings, &[]).unwrap();
        drop(store);
        let manager =
            Manager::init_with_diagnostics(data.clone(), None, None, Diagnostics::disabled())
                .await
                .unwrap();
        async fn add(manager: &Manager, name: &str) -> DownloadTask {
            let media = MediaInfo {
                trusted_preview_ref: None,
                url: format!("https://fixture.invalid/{name}"),
                title: "untrusted frontend placeholder".into(),
                media_id: "ignored".into(),
                source: "ignored".into(),
                duration: None,
                thumbnail: None,
                is_playlist: false,
                formats: vec![],
                warnings: vec![],
            };
            manager
                .enqueue(EnqueueInput {
                    media,
                    mode: "video".into(),
                    quality: "best".into(),
                    format_id: None,
                    output_dir: None,
                })
                .await
                .unwrap()
        }
        async fn wait(manager: &Manager, id: &str, status: TaskStatus) -> DownloadTask {
            tokio::time::timeout(Duration::from_secs(20), async {
                loop {
                    let task = manager.task(id).await.unwrap();
                    if task.status == status {
                        return task;
                    }
                    if !task.status.active() {
                        panic!(
                            "unexpected terminal state {:?}: {:?}",
                            task.status, task.error
                        );
                    }
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
            })
            .await
            .expect("fixture task timed out")
        }
        let slow = add(&manager, "slow").await;
        wait(&manager, &slow.id, TaskStatus::Downloading).await;
        assert_eq!(
            manager.task(&slow.id).await.unwrap().media.title,
            "Fixture slow"
        );
        let cancelled = add(&manager, "waiting-cancel").await;
        let waiting = add(&manager, "waiting-finish").await;
        manager.cancel(&cancelled.id).await.unwrap();
        assert_eq!(
            manager.task(&cancelled.id).await.unwrap().status,
            TaskStatus::Cancelled
        );
        // A busy database must not delay stopping the actual fixture process.
        let connection = rusqlite::Connection::open(data.join("fluxdl.sqlite3")).unwrap();
        connection.execute_batch("BEGIN IMMEDIATE;").unwrap();
        let control = manager.active.lock().await.as_ref().unwrap().clone();
        #[cfg(windows)]
        let process_handle = unsafe {
            windows_sys::Win32::System::Threading::OpenProcess(
                windows_sys::Win32::System::Threading::PROCESS_SYNCHRONIZE,
                0,
                control.cancel.process_id().expect("active fixture process"),
            )
        };
        #[cfg(windows)]
        assert!(!process_handle.is_null());
        let cancelling_manager = manager.clone();
        let cancelling_id = slow.id.clone();
        let cancellation =
            tokio::spawn(async move { cancelling_manager.cancel(&cancelling_id).await });
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                #[cfg(windows)]
                let stopped = unsafe {
                    windows_sys::Win32::System::Threading::WaitForSingleObject(process_handle, 0)
                } == windows_sys::Win32::Foundation::WAIT_OBJECT_0;
                #[cfg(not(windows))]
                let stopped = control.cancel.is_cancelled();
                if stopped {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("real process did not stop while database was busy");
        // SQLite may return BUSY immediately when upgrading a deferred read transaction.
        // An error ack is permitted; successful unconfirmed cancellation is not.

        #[cfg(windows)]
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(process_handle);
        }
        connection.execute_batch("ROLLBACK;").unwrap();
        drop(connection);
        if let Err(error) = cancellation.await.unwrap() {
            assert!(matches!(
                error.code,
                ErrorCode::PersistenceRejected | ErrorCode::PersistenceUnconfirmed
            ));
        }
        wait(&manager, &slow.id, TaskStatus::Cancelled).await;
        wait(&manager, &waiting.id, TaskStatus::Completed).await;
        let first = add(&manager, "serial-a").await;
        let second = add(&manager, "serial-b").await;
        wait(&manager, &first.id, TaskStatus::Completed).await;
        wait(&manager, &second.id, TaskStatus::Completed).await;
        let failed = add(&manager, "fail-first").await;
        let failure = wait(&manager, &failed.id, TaskStatus::Failed).await;
        assert!(failure.error.is_some());
        manager.retry(&failed.id).await.unwrap();
        let finished = wait(&manager, &failed.id, TaskStatus::Completed).await;
        assert!(finished.file_exists);
        let events = std::fs::read_to_string(output.join("events.log")).unwrap();
        assert!(!events.contains("start waiting-cancel"));
        assert!(events.find("finish serial-a").unwrap() < events.find("start serial-b").unwrap());
        manager.shutdown().await.unwrap();
        drop(manager);
        let reopened =
            Manager::init_with_diagnostics(data.clone(), None, None, Diagnostics::disabled())
                .await
                .unwrap();
        let snapshot = reopened.snapshot().await;
        assert_eq!(snapshot.settings.theme, "light");
        assert_eq!(snapshot.tasks.len(), 6);
        assert_eq!(
            snapshot
                .tasks
                .iter()
                .filter(|t| t.status == TaskStatus::Completed)
                .count(),
            4
        );
        std::fs::remove_file(finished.final_path.unwrap()).unwrap();
        assert!(!reopened.task(&failed.id).await.unwrap().file_exists);
        // A rejected shutdown must restore acceptance and leave exactly one functioning worker.
        let recoverable = add(&reopened, "slow").await;
        wait(&reopened, &recoverable.id, TaskStatus::Downloading).await;
        let connection = rusqlite::Connection::open(data.join("fluxdl.sqlite3")).unwrap();
        connection.execute_batch("CREATE TRIGGER reject_cancel BEFORE INSERT ON tasks WHEN json_extract(NEW.document,'$.status')='cancelling' BEGIN SELECT RAISE(ABORT,'fixture shutdown failure'); END;").unwrap();
        assert!(reopened.shutdown().await.is_err());
        assert!(!reopened.shutting_down.load(Ordering::SeqCst));
        assert!(!reopened.stop_worker.load(Ordering::SeqCst));
        connection
            .execute_batch("DROP TRIGGER reject_cancel;")
            .unwrap();
        drop(connection);
        wait(&reopened, &recoverable.id, TaskStatus::Cancelled).await;
        let recovery = add(&reopened, "after-shutdown-failure").await;
        wait(&reopened, &recovery.id, TaskStatus::Completed).await;
        let events = std::fs::read_to_string(output.join("events.log")).unwrap();
        assert_eq!(
            events
                .lines()
                .filter(|line| *line == "start after-shutdown-failure")
                .count(),
            1
        );
        reopened.shutdown().await.unwrap();
        drop(reopened);
        std::fs::remove_dir_all(root).unwrap();
    }
}
