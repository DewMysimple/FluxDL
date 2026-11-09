#[cfg(windows)]
use crate::process_job::ProcessJob;
use crate::{
    engine::{self, ProgressEvent},
    models::*,
    persistence::Persistence,
};
use futures_util::{stream, StreamExt};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tauri::Emitter;
use tokio::{
    io::AsyncReadExt,
    process::Command,
    sync::{mpsc, Mutex, Notify},
};

const LOG_LIMIT: usize = 100;
struct Inner {
    settings: Settings,
    engines: EngineDiagnostics,
    tasks: Vec<DownloadTask>,
    store: Persistence,
}
impl Inner {
    fn save(&mut self) -> Result<(), String> {
        // MVP: short synchronous transactions on a backend runtime thread. See
        // docs/persistence.md before increasing history volume or write frequency.
        self.store.save(&self.settings, &self.tasks)
    }
    fn warn(&mut self, warning: String) {
        if !self.engines.warnings.contains(&warning) {
            self.engines.warnings.push(warning);
            if self.engines.warnings.len() > 20 {
                self.engines.warnings.remove(0);
            }
        }
    }
}
struct Control {
    id: String,
    cancelled: AtomicBool,
    wake: Notify,
}

pub struct Manager {
    inner: Mutex<Inner>,
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
    ) -> Result<Arc<Self>, String> {
        let (store, loaded) = tokio::task::spawn_blocking(move || Persistence::open(&data_dir))
            .await
            .map_err(|e| e.to_string())??;
        let mut engines = engine::detect(&loaded.settings, resource_dir.as_deref()).await;
        if let Some(warning) = loaded.recovery_warning {
            engines.warnings.push(warning);
        }
        let manager = Arc::new(Self {
            inner: Mutex::new(Inner {
                settings: loaded.settings,
                engines,
                tasks: loaded.tasks,
                store,
            }),
            active: Mutex::new(None),
            wake: Notify::new(),
            shutting_down: AtomicBool::new(false),
            worker_stopped: Notify::new(),
            worker_finished: AtomicBool::new(false),
            resource_dir,
            app,
        });
        let worker = manager.clone();
        tauri::async_runtime::spawn(async move {
            worker.worker().await;
        });
        Ok(manager)
    }

    pub async fn snapshot(&self) -> AppSnapshot {
        let (settings, engines, tasks) = {
            let inner = self.inner.lock().await;
            (
                inner.settings.clone(),
                inner.engines.clone(),
                inner.tasks.clone(),
            )
        };
        // File existence is refreshed without blocking a runtime thread or holding the queue lock.
        let tasks = stream::iter(tasks)
            .map(|mut task| async move {
                task.file_exists = match task.final_path.as_deref() {
                    Some(path) => tokio::fs::metadata(path).await.is_ok_and(|m| m.is_file()),
                    None => false,
                };
                task
            })
            .buffered(16)
            .collect()
            .await;
        AppSnapshot {
            settings,
            engines,
            tasks,
        }
    }
    async fn emit(&self) {
        if let Some(app) = &self.app {
            let _ = app.emit("app-state-updated", self.snapshot().await);
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
    pub async fn task(&self, id: &str) -> Result<DownloadTask, String> {
        self.snapshot()
            .await
            .tasks
            .into_iter()
            .find(|t| t.id == id)
            .ok_or_else(|| "任务记录不存在。".into())
    }
    pub async fn detect(&self) -> EngineDiagnostics {
        let settings = self.inner.lock().await.settings.clone();
        let diagnostics = engine::detect(&settings, self.resource_dir.as_deref()).await;
        let mut inner = self.inner.lock().await;
        if inner.settings.engine_path != settings.engine_path
            || inner.settings.ffmpeg_path != settings.ffmpeg_path
        {
            return inner.engines.clone();
        }
        inner.engines = diagnostics.clone();
        drop(inner);
        self.emit().await;
        diagnostics
    }
    pub async fn probe(&self, url: &str) -> Result<MediaInfo, String> {
        let (settings, engines) = {
            let inner = self.inner.lock().await;
            (inner.settings.clone(), inner.engines.clone())
        };
        engine::probe(&settings, &engines, url).await
    }
    pub async fn enqueue(&self, input: EnqueueInput) -> Result<DownloadTask, String> {
        if self.shutting_down.load(Ordering::SeqCst) {
            return Err("应用正在退出，请稍后重试。".into());
        }
        validate_options(&input.mode, &input.quality)?;
        // Metadata sent over IPC is not trusted. Resolve it again with the configured engine.
        let media = self.probe(&input.media.url).await?;
        if media.is_playlist {
            return Err("当前 MVP 不执行整列表批量下载，请提供单个视频的链接。".into());
        }
        if media.formats.is_empty() {
            return Err("没有可下载的媒体格式，请尝试其他链接或检查引擎诊断。".into());
        }
        if let Some(id) = &input.format_id {
            if !media.formats.iter().any(|f| &f.format_id == id) {
                return Err("所选格式已不可用，请重新解析链接。".into());
            }
        }
        let settings = self.inner.lock().await.settings.clone();
        let requested_dir = input
            .output_dir
            .unwrap_or_else(|| settings.download_dir.clone());
        let output_dir = writable_directory(&requested_dir)?;
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
        {
            let mut inner = self.inner.lock().await;
            engine::build_download_args(&task, &inner.settings, &inner.engines)?;
            if self.shutting_down.load(Ordering::SeqCst) {
                return Err("应用正在退出，请稍后重试。".into());
            }
            inner.tasks.push(task.clone());
            if let Err(error) = inner.save() {
                inner.tasks.retain(|t| t.id != task.id);
                return Err(error);
            }
        }
        self.emit().await;
        self.wake.notify_one();
        Ok(task)
    }
    pub async fn cancel(&self, id: &str) -> Result<(), String> {
        {
            let mut inner = self.inner.lock().await;
            let task = inner
                .tasks
                .iter_mut()
                .find(|t| t.id == id)
                .ok_or("任务记录不存在。")?;
            if !task.status.active() {
                return Ok(());
            }
            task.status = if task.status == TaskStatus::Queued {
                TaskStatus::Cancelled
            } else {
                TaskStatus::Cancelling
            };
            task.updated_at = chrono::Utc::now().to_rfc3339();
            if let Err(error) = inner.save() {
                inner.warn(error);
            }
        }
        if let Some(control) = self
            .active
            .lock()
            .await
            .as_ref()
            .filter(|control| control.id == id)
        {
            control.cancelled.store(true, Ordering::SeqCst);
            control.wake.notify_one();
        }
        self.emit().await;
        self.wake.notify_one();
        Ok(())
    }
    pub async fn retry(&self, id: &str) -> Result<(), String> {
        if self.shutting_down.load(Ordering::SeqCst) {
            return Err("应用正在退出。".into());
        }
        {
            let mut inner = self.inner.lock().await;
            let index = inner
                .tasks
                .iter()
                .position(|t| t.id == id)
                .ok_or("任务记录不存在。")?;
            if !inner.tasks[index].status.retryable() {
                return Err("只有失败、取消或中断的任务可以重试。".into());
            }
            writable_directory(&inner.tasks[index].output_dir)?;
            engine::build_download_args(&inner.tasks[index], &inner.settings, &inner.engines)?;
            let previous = inner.tasks[index].clone();
            let task = &mut inner.tasks[index];
            task.status = TaskStatus::Queued;
            task.progress = None;
            task.speed = None;
            task.eta = None;
            task.downloaded_bytes = 0;
            task.total_bytes = None;
            task.final_path = None;
            task.error = None;
            task.logs.clear();
            task.file_exists = false;
            task.updated_at = chrono::Utc::now().to_rfc3339();
            // Retried work joins the tail rather than jumping ahead of queued work.
            let retried = inner.tasks.remove(index);
            inner.tasks.push(retried);
            if let Err(error) = inner.save() {
                inner.tasks.pop();
                inner.tasks.insert(index, previous);
                return Err(error);
            }
        }
        self.emit().await;
        self.wake.notify_one();
        Ok(())
    }
    pub async fn remove_history(&self, id: &str) -> Result<(), String> {
        {
            let mut inner = self.inner.lock().await;
            let index = inner
                .tasks
                .iter()
                .position(|t| t.id == id)
                .ok_or("任务记录不存在。")?;
            if inner.tasks[index].status != TaskStatus::Completed {
                return Err("只能删除已完成的历史记录。".into());
            }
            let task = inner.tasks.remove(index);
            if let Err(error) = inner.save() {
                inner.tasks.insert(index, task);
                return Err(error);
            }
        }
        self.emit().await;
        Ok(())
    }
    pub async fn update_settings(&self, settings: Settings) -> Result<Settings, String> {
        validate_settings(&settings)?;
        writable_directory(&settings.download_dir)?;
        {
            let mut inner = self.inner.lock().await;
            let previous = std::mem::replace(&mut inner.settings, settings.clone());
            if let Err(error) = inner.save() {
                inner.settings = previous;
                return Err(error);
            }
        }
        self.detect().await;
        Ok(settings)
    }
    pub async fn shutdown(&self) -> Result<(), String> {
        self.shutting_down.store(true, Ordering::SeqCst);
        let ids: Vec<String> = self
            .inner
            .lock()
            .await
            .tasks
            .iter()
            .filter(|t| t.status.active())
            .map(|t| t.id.clone())
            .collect();
        for id in ids {
            self.cancel(&id).await?;
        }
        self.wake.notify_one();
        let wait = async {
            while !self.worker_finished.load(Ordering::SeqCst) {
                let notified = self.worker_stopped.notified();
                if self.worker_finished.load(Ordering::SeqCst) {
                    break;
                }
                notified.await;
            }
        };
        tokio::time::timeout(Duration::from_secs(12), wait)
            .await
            .map_err(|_| "停止下载进程超时，请稍后再次退出。".to_string())?;
        self.inner.lock().await.save()
    }

    async fn worker(self: Arc<Self>) {
        loop {
            if self.shutting_down.load(Ordering::SeqCst) {
                break;
            }
            let mut claim_failed = false;
            let selected = {
                // The control is installed while holding the state lock, before cancellation can see a running task.
                let mut inner = self.inner.lock().await;
                if let Some(index) = inner
                    .tasks
                    .iter()
                    .position(|t| t.status == TaskStatus::Queued)
                {
                    let task = &mut inner.tasks[index];
                    task.status = TaskStatus::Resolving;
                    task.updated_at = chrono::Utc::now().to_rfc3339();
                    let task = task.clone();
                    let control = Arc::new(Control {
                        id: task.id.clone(),
                        cancelled: AtomicBool::new(false),
                        wake: Notify::new(),
                    });
                    *self.active.lock().await = Some(control.clone());
                    if let Err(error) = inner.save() {
                        let task = &mut inner.tasks[index];
                        task.status = TaskStatus::Failed;
                        task.error = Some(error);
                        claim_failed = true;
                        None
                    } else {
                        Some((task, inner.settings.clone(), inner.engines.clone(), control))
                    }
                } else {
                    None
                }
            };
            if claim_failed {
                *self.active.lock().await = None;
                self.emit().await;
                continue;
            }
            if let Some((task, settings, engines, control)) = selected {
                self.emit().await;
                let result = self.run_process(&task, &settings, &engines, &control).await;
                {
                    let mut inner = self.inner.lock().await;
                    if let Some(task) = inner.tasks.iter_mut().find(|t| t.id == task.id) {
                        if task.status.active() {
                            task.updated_at = chrono::Utc::now().to_rfc3339();
                            task.speed = None;
                            task.eta = None;
                            if control.cancelled.load(Ordering::SeqCst)
                                || task.status == TaskStatus::Cancelling
                            {
                                if let Err(error) = result {
                                    task.status = TaskStatus::Failed;
                                    task.error = Some(format!("停止任务时发生错误：{error}"));
                                } else {
                                    task.status = TaskStatus::Cancelled;
                                    task.error = None;
                                }
                            } else if let Err(error) = result {
                                task.status = TaskStatus::Failed;
                                task.error = Some(error);
                            } else {
                                task.status = TaskStatus::Completed;
                                task.progress = None;
                                task.file_exists = true;
                            }
                        }
                    }
                    if let Err(error) = inner.save() {
                        inner.warn(error);
                    }
                }
                *self.active.lock().await = None;
                self.emit().await;
                if settings.completion_action == "folder" {
                    let task = self.task(&task.id).await;
                    if let Ok(task) = task {
                        if task.status == TaskStatus::Completed {
                            let _ = open_folder(&task.output_dir).await;
                        }
                    }
                }
                continue;
            }
            if self.shutting_down.load(Ordering::SeqCst) {
                break;
            }
            self.wake.notified().await;
        }
        self.worker_finished.store(true, Ordering::SeqCst);
        self.worker_stopped.notify_waiters();
    }

    async fn run_process(
        &self,
        task: &DownloadTask,
        settings: &Settings,
        engines: &EngineDiagnostics,
        control: &Arc<Control>,
    ) -> Result<(), String> {
        if control.cancelled.load(Ordering::SeqCst) {
            return Ok(());
        }
        let args = engine::build_download_args(task, settings, engines)?;
        let executable = engines
            .ytdlp
            .path
            .as_ref()
            .ok_or("未找到 yt-dlp，请在设置中选择引擎。")?;
        let mut command = Command::new(executable);
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .current_dir(&task.output_dir)
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x08000000);
        let mut child = command
            .spawn()
            .map_err(|e| format!("无法启动 yt-dlp：{e}。请重新检查引擎路径。"))?;
        let pid = child.id();
        #[cfg(windows)]
        let mut job = match ProcessJob::assign(&child) {
            Ok(job) => Some(job),
            Err(error) => {
                if let Some(pid) = pid {
                    let _ = kill_process_tree(pid).await;
                }
                let _ = child.kill().await;
                return Err(format!(
                    "无法建立安全的下载进程组：{error}。请检查系统进程权限后重试。"
                ));
            }
        };
        let (sender, mut receiver) = mpsc::channel::<String>(128);
        let mut readers = Vec::new();
        if let Some(stdout) = child.stdout.take() {
            readers.push(tokio::spawn(read_lines_bounded(stdout, sender.clone())));
        }
        if let Some(stderr) = child.stderr.take() {
            readers.push(tokio::spawn(read_lines_bounded(stderr, sender.clone())));
        }
        drop(sender);
        let mut tick = tokio::time::interval(Duration::from_millis(250));
        let mut last_save = Instant::now();
        let mut dirty = false;
        let mut exit = None;
        let mut streams_closed = false;
        let mut killed = false;
        let mut exited_at = None;
        while exit.is_none() || !streams_closed {
            tokio::select! {
                status=child.wait(), if exit.is_none()=>{
                    exit=Some(status.map_err(|e|format!("无法读取下载进程退出状态：{e}"))?);exited_at=Some(Instant::now());
                    // Closing the job also cleans up descendants if the parent exited unexpectedly.
                    #[cfg(windows)] drop(job.take());
                }
                _=control.wake.notified(), if !killed&&exit.is_none()=>{
                    if control.cancelled.load(Ordering::SeqCst){
                        #[cfg(windows)] if let Some(job)=job.as_ref(){if job.terminate().is_err(){if let Some(pid)=pid{kill_process_tree(pid).await?;}}}
                        #[cfg(not(windows))] if let Some(pid)=pid {kill_process_tree(pid).await?;}
                        let _=child.kill().await;killed=true;
                    }
                }
                line=receiver.recv(), if !streams_closed=>{
                    if let Some(line)=line {
                        let event=engine::parse_progress(&line);
                        let mut inner=self.inner.lock().await;
                        if let Some(task)=inner.tasks.iter_mut().find(|t|t.id==task.id) {
                            if task.status.active() {
                                match event {
                                    Some(ProgressEvent::Download{downloaded_bytes,total_bytes,speed,eta,progress})=>{if task.status!=TaskStatus::Cancelling {task.status=TaskStatus::Downloading;task.downloaded_bytes=downloaded_bytes;task.total_bytes=total_bytes;task.speed=speed;task.eta=eta;task.progress=progress;}},
                                    Some(ProgressEvent::Postprocessing)=>{if task.status!=TaskStatus::Cancelling{task.status=TaskStatus::Postprocessing;task.progress=None;task.speed=None;task.eta=None;}},
                                    Some(ProgressEvent::File(path))=>{task.final_path=Some(path);},
                                    None=>{let line=engine::redact_log(&line);if !line.trim().is_empty(){push_log(task,line);}},
                                }
                                task.updated_at=chrono::Utc::now().to_rfc3339();dirty=true;
                            }
                        }
                    }else{streams_closed=true;}
                }
                _=tick.tick()=>{
                    if exited_at.is_some_and(|at:Instant|at.elapsed()>Duration::from_secs(3))&&!streams_closed {break;}
                    if dirty {
                        if last_save.elapsed()>=Duration::from_secs(2){let mut inner=self.inner.lock().await;if let Err(error)=inner.save(){inner.warn(error);}last_save=Instant::now();}
                        self.emit().await;dirty=false;
                    }
                }
            }
        }
        for reader in readers {
            reader.abort();
        }
        if control.cancelled.load(Ordering::SeqCst) {
            return Ok(());
        }
        let inner = self.inner.lock().await;
        let current = inner
            .tasks
            .iter()
            .find(|t| t.id == task.id)
            .ok_or("任务记录不存在。")?;
        if !exit.is_some_and(|status| status.success()) {
            let technical = current
                .logs
                .iter()
                .rev()
                .take(12)
                .cloned()
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
                .join("\n");
            return Err(engine::friendly_error(&technical));
        }
        let path = current
            .final_path
            .as_deref()
            .ok_or("引擎已退出，但没有报告最终文件路径。请检查任务日志与 yt-dlp 版本。")?;
        validate_final_path(path, &current.output_dir)?;
        Ok(())
    }
}

fn push_log(task: &mut DownloadTask, line: String) {
    task.logs.push(line.chars().take(1600).collect());
    if task.logs.len() > LOG_LIMIT {
        task.logs.drain(..task.logs.len() - LOG_LIMIT);
    }
}

async fn read_lines_bounded<R: tokio::io::AsyncRead + Unpin>(
    mut reader: R,
    sender: mpsc::Sender<String>,
) {
    let mut chunk = [0u8; 4096];
    let mut line = Vec::with_capacity(8192);
    loop {
        match reader.read(&mut chunk).await {
            Ok(0) => break,
            Ok(count) => {
                for byte in &chunk[..count] {
                    if *byte == b'\n' || *byte == b'\r' {
                        if !line.is_empty() {
                            if sender
                                .send(String::from_utf8_lossy(&line).into_owned())
                                .await
                                .is_err()
                            {
                                return;
                            }
                            line.clear();
                        }
                    } else if line.len() < 8192 {
                        line.push(*byte);
                    }
                }
            }
            Err(_) => break,
        }
    }
    if !line.is_empty() {
        let _ = sender
            .send(String::from_utf8_lossy(&line).into_owned())
            .await;
    }
}

async fn kill_process_tree(pid: u32) -> Result<(), String> {
    #[cfg(windows)]
    {
        let mut command = Command::new("taskkill.exe");
        command
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(0x08000000);
        // A nonzero code can mean the process exited immediately before cancellation.
        tokio::time::timeout(Duration::from_secs(6), command.status())
            .await
            .map_err(|_| "终止下载子进程树超时。".to_string())?
            .map_err(|e| format!("无法终止下载子进程树：{e}"))?;
    }
    #[cfg(not(windows))]
    {
        let _ = pid;
    }
    Ok(())
}

async fn open_folder(directory: &str) -> Result<(), String> {
    #[cfg(windows)]
    let executable = "explorer.exe";
    #[cfg(target_os = "macos")]
    let executable = "open";
    #[cfg(all(not(windows), not(target_os = "macos")))]
    let executable = "xdg-open";
    let mut command = Command::new(executable);
    command
        .arg(directory)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    command.spawn().map_err(|e| e.to_string())?;
    Ok(())
}

pub fn writable_directory(raw: &str) -> Result<PathBuf, String> {
    if raw.len() > 32700 || !Path::new(raw).is_absolute() {
        return Err("请选择有效的绝对保存目录路径。".into());
    }
    std::fs::create_dir_all(raw)
        .map_err(|e| format!("无法创建保存目录：{e}。请检查路径、权限及磁盘空间。"))?;
    let directory = std::fs::canonicalize(raw).map_err(|e| format!("无法访问保存目录：{e}"))?;
    let test = directory.join(format!(".fluxdl-write-test-{}", uuid::Uuid::new_v4()));
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&test)
        .map_err(|e| format!("保存目录不可写：{e}。请选择其他目录。"))?;
    std::fs::remove_file(test).map_err(|e| format!("无法清理目录权限检查文件：{e}"))?;
    Ok(directory)
}

pub fn validate_final_path(raw: &str, output_dir: &str) -> Result<PathBuf, String> {
    let path = std::fs::canonicalize(raw)
        .map_err(|_| "本地文件不存在：引擎报告的输出文件没有落地或已被移动。".to_string())?;
    let directory =
        std::fs::canonicalize(output_dir).map_err(|e| format!("无法检查保存目录：{e}"))?;
    if !path.is_file() || !path.starts_with(directory) {
        return Err("引擎报告的文件不在本任务保存目录内，已阻止不安全的文件操作。".into());
    }
    if !path
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(engine::is_media_extension)
    {
        return Err(
            "引擎报告的输出不是受支持的媒体文件类型，已阻止完成和打开此文件。请检查任务日志。"
                .into(),
        );
    }
    Ok(path)
}
fn validate_options(mode: &str, quality: &str) -> Result<(), String> {
    if !["video", "audio"].contains(&mode)
        || !["best", "2160", "1440", "1080", "720", "480"].contains(&quality)
    {
        return Err("下载模式或质量预设无效。".into());
    }
    Ok(())
}
fn validate_settings(settings: &Settings) -> Result<(), String> {
    validate_options(&settings.default_mode, &settings.quality)?;
    if !["dark", "light", "system"].contains(&settings.theme.as_str())
        || !["comfortable", "compact"].contains(&settings.density.as_str())
        || !["tray", "exit"].contains(&settings.close_behavior.as_str())
        || !["none", "folder"].contains(&settings.completion_action.as_str())
    {
        return Err("设置选项无效。".into());
    }
    for path in [&settings.engine_path, &settings.ffmpeg_path] {
        if !path.is_empty() && (!Path::new(path).is_absolute() || path.len() > 32700) {
            return Err("引擎路径必须是有效的绝对路径。".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture_task(id: &str, status: TaskStatus, directory: &Path) -> DownloadTask {
        DownloadTask {
            id: id.into(),
            media: MediaInfo {
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
    #[tokio::test]
    async fn queued_cancel_is_persisted_retry_is_guarded_and_delete_never_removes_files() {
        let directory =
            std::env::temp_dir().join(format!("fluxdl-state-test-{}", uuid::Uuid::new_v4()));
        let (store, _) = Persistence::open(&directory).unwrap();
        let mut engines = EngineDiagnostics::default();
        engines.ytdlp.available = true;
        let file = directory.join("existing.mp4");
        std::fs::write(&file, b"fixture").unwrap();
        let mut completed = fixture_task("completed", TaskStatus::Completed, &directory);
        completed.final_path = Some(file.to_string_lossy().into_owned());
        let manager = Manager {
            inner: Mutex::new(Inner {
                settings: Settings::default(),
                engines,
                tasks: vec![
                    fixture_task("queued", TaskStatus::Queued, &directory),
                    completed,
                ],
                store,
            }),
            active: Mutex::new(None),
            wake: Notify::new(),
            shutting_down: AtomicBool::new(false),
            worker_stopped: Notify::new(),
            worker_finished: AtomicBool::new(false),
            resource_dir: None,
            app: None,
        };
        manager.cancel("queued").await.unwrap();
        manager.cancel("queued").await.unwrap();
        assert_eq!(
            manager.task("queued").await.unwrap().status,
            TaskStatus::Cancelled
        );
        let (first, second) = tokio::join!(manager.retry("queued"), manager.retry("queued"));
        assert!(first.is_ok());
        assert!(second.is_err());
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
    #[test]
    fn typed_options_reject_cli_injection() {
        assert!(validate_options("video", "1080").is_ok());
        assert!(validate_options("--exec", "best").is_err());
        assert!(validate_options("video", "1080;calc").is_err());
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
    #[tokio::test]
    async fn bounded_reader_limits_large_unbroken_output() {
        let (tx, mut rx) = mpsc::channel(2);
        let input = vec![b'x'; 128000];
        let reader = tokio::spawn(async move {
            read_lines_bounded(input.as_slice(), tx).await;
        });
        let line = rx.recv().await.unwrap();
        assert_eq!(line.len(), 8192);
        reader.await.unwrap();
    }
    #[test]
    fn status_retry_and_running_semantics() {
        assert!(TaskStatus::Downloading.running());
        assert!(!TaskStatus::Queued.running());
        assert!(TaskStatus::Interrupted.retryable());
        assert!(!TaskStatus::Completed.retryable());
        assert!(!TaskStatus::Cancelled.active());
    }
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
        let manager = Manager::init(data.clone(), None, None).await.unwrap();
        async fn add(manager: &Manager, name: &str) -> DownloadTask {
            let media = MediaInfo {
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
        manager.cancel(&slow.id).await.unwrap();
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
        let reopened = Manager::init(data, None, None).await.unwrap();
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
        reopened.shutdown().await.unwrap();
        drop(reopened);
        std::fs::remove_dir_all(root).unwrap();
    }
}
