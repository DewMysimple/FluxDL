//! Owned, bounded production Manager / exact-argv CLI comparison. Input is stdin-only.
//! Raw metadata/stderr, URL, title and command line never leave this process.
use fluxdl_lib::{
    diagnostics::{Diagnostics, Policy},
    engine::{self, ProgressEvent},
    errors::{AppError, ErrorCode, ErrorPhase},
    manager::Manager,
    models::{DownloadTask, EngineDiagnostics, EnqueueInput, MediaInfo, Settings, TaskStatus},
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tokio::{
    io::AsyncReadExt,
    process::{Child, Command},
    sync::mpsc,
    task::JoinHandle,
};
// Only the low-level lifetime guard is shared. CLI does not use Manager,
// process::start_download, transitions, or the application's progress consumer.
#[cfg(windows)]
#[path = "../src/process_job.rs"]
mod owned_cli_job;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Input {
    action: String,
    url: String,
    engine_path: String,
    #[serde(default)]
    ffmpeg_path: String,
    run_root: String,
    sample_root: String,
    format_id: Option<String>,
    proxy: Option<String>,
    fragments: Option<u8>,
    max_seconds: f64,
    max_bytes: u64,
}
fn emit(value: Value) {
    println!("{value}");
    let _ = std::io::stdout().flush();
}
fn safe_failure(error: &AppError) -> Value {
    json!({"type":"failure","error_code":error.code,"error_kind":error.kind,"phase":error.phase})
}
fn invalid() -> AppError {
    AppError::new(
        ErrorCode::InvalidInput,
        ErrorPhase::Validate,
        "Invalid bounded test input",
    )
}
fn probe_error(mut error: AppError) -> AppError {
    error.phase = ErrorPhase::Probe;
    error
}
fn cli_error(code: ErrorCode) -> AppError {
    AppError::new(
        code,
        ErrorPhase::Download,
        "Owned CLI comparison did not complete",
    )
}

enum PipeChunk {
    Data { stderr: bool, bytes: Vec<u8> },
    Fault,
}
async fn read_pipe<R: tokio::io::AsyncRead + Unpin>(
    mut pipe: R,
    stderr: bool,
    tx: mpsc::Sender<PipeChunk>,
) {
    let mut buffer = [0u8; 8192];
    loop {
        match pipe.read(&mut buffer).await {
            Ok(0) => break,
            Ok(n) => {
                if tx
                    .send(PipeChunk::Data {
                        stderr,
                        bytes: buffer[..n].to_vec(),
                    })
                    .await
                    .is_err()
                {
                    break;
                }
            }
            Err(_) => {
                let _ = tx.send(PipeChunk::Fault).await;
                break;
            }
        }
    }
}
struct OwnedCli {
    child: Child,
    spawned_at: Instant,
    #[cfg(windows)]
    job: owned_cli_job::ProcessJob,
    readers: [JoinHandle<()>; 2],
    output: mpsc::Receiver<PipeChunk>,
}
impl Drop for OwnedCli {
    fn drop(&mut self) {
        for reader in &self.readers {
            reader.abort();
        }
    }
}
impl OwnedCli {
    async fn spawn(executable: &str, args: &[String], directory: &Path) -> Result<Self, AppError> {
        let mut command = Command::new(executable);
        command
            .args(args)
            .current_dir(directory)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x08000000);
        let mut child = command
            .spawn()
            .map_err(|_| cli_error(ErrorCode::ProcessSpawn))?;
        let spawned_at = Instant::now();
        #[cfg(windows)]
        let job = match owned_cli_job::ProcessJob::assign(&child) {
            Ok(job) => job,
            Err(_) => {
                let _ = child.start_kill();
                let _ = child.wait().await;
                return Err(cli_error(ErrorCode::ProcessCleanup));
            }
        };
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| cli_error(ErrorCode::ProcessRead))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| cli_error(ErrorCode::ProcessRead))?;
        let (tx, output) = mpsc::channel(32);
        let readers = [
            tokio::spawn(read_pipe(stdout, false, tx.clone())),
            tokio::spawn(read_pipe(stderr, true, tx)),
        ];
        Ok(Self {
            child,
            spawned_at,
            #[cfg(windows)]
            job,
            readers,
            output,
        })
    }
    fn stop(&mut self) -> Result<(), AppError> {
        #[cfg(windows)]
        self.job
            .terminate()
            .map_err(|_| cli_error(ErrorCode::ProcessCleanup))?;
        let _ = self.child.start_kill();
        Ok(())
    }
    async fn reap(&mut self) -> Result<std::process::ExitStatus, AppError> {
        match tokio::time::timeout(Duration::from_secs(5), self.child.wait()).await {
            Ok(result) => result.map_err(|_| cli_error(ErrorCode::ProcessCleanup)),
            Err(_) => {
                self.stop()?;
                let _ = tokio::time::timeout(Duration::from_secs(5), self.child.wait()).await;
                Err(cli_error(ErrorCode::ProcessCleanup))
            }
        }
    }
}

fn proxy_valid(proxy: &str) -> bool {
    let explicit_port = proxy
        .split_once("://")
        .and_then(|(_, rest)| rest.split('/').next())
        .and_then(|host| host.rsplit_once(':'))
        .is_some_and(|(_, port)| {
            !port.is_empty() && port.bytes().all(|byte| byte.is_ascii_digit())
        });
    url::Url::parse(proxy).is_ok_and(|p| {
        ["http", "socks5"].contains(&p.scheme())
            && ["127.0.0.1", "localhost", "[::1]"].contains(&p.host_str().unwrap_or_default())
            && explicit_port
            && p.port_or_known_default().is_some_and(|port| port > 0)
            && p.username().is_empty()
            && p.password().is_none()
            && ["", "/"].contains(&p.path())
            && p.query().is_none()
            && p.fragment().is_none()
    })
}
fn check_path(path: &Path) -> Result<(), AppError> {
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(invalid());
    }
    for parent in path.ancestors() {
        let m = std::fs::symlink_metadata(parent).map_err(|_| invalid())?;
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if m.file_attributes() & 0x400 != 0 {
                return Err(invalid());
            }
        }
        if m.file_type().is_symlink() || !m.is_dir() {
            return Err(invalid());
        }
    }
    Ok(())
}
fn validate(input: &Input) -> Result<PathBuf, AppError> {
    if !input.max_seconds.is_finite()
        || !(0.1..=45.).contains(&input.max_seconds)
        || !(1..=40 * 1024 * 1024).contains(&input.max_bytes)
    {
        return Err(invalid());
    }
    engine::validate_url(&input.url)?;
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let allowed = workspace.join("artifacts/network-diagnostics");
    let run = PathBuf::from(&input.run_root);
    let sample = PathBuf::from(&input.sample_root);
    if !run.starts_with(&allowed) || run == allowed || !sample.starts_with(&run) || sample == run {
        return Err(invalid());
    }
    check_path(&run)?;
    check_path(&sample)?;
    let marker = run.join(".network-owner.json");
    let marker_meta = std::fs::symlink_metadata(&marker).map_err(|_| invalid())?;
    if !marker_meta.is_file() || marker_meta.file_type().is_symlink() || marker_meta.len() > 8192 {
        return Err(invalid());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if marker_meta.file_attributes() & 0x400 != 0 {
            return Err(invalid());
        }
    }
    let owner = std::fs::read_to_string(marker).map_err(|_| invalid())?;
    let value: Value = serde_json::from_str(&owner).map_err(|_| invalid())?;
    if value.get("owner").and_then(Value::as_str) != Some("fluxdl-network-diagnostics-v1") {
        return Err(invalid());
    }
    if input.fragments.is_some_and(|n| ![1, 2, 4].contains(&n)) {
        return Err(invalid());
    }
    if let Some(proxy) = &input.proxy {
        if !proxy_valid(proxy) {
            return Err(invalid());
        }
    }
    Ok(sample)
}
fn settings(input: &Input, output: &Path) -> Settings {
    Settings {
        engine_path: input.engine_path.clone(),
        ffmpeg_path: input.ffmpeg_path.clone(),
        download_dir: output.to_string_lossy().into_owned(),
        ..Settings::default()
    }
}
fn task(media: MediaInfo, input: &Input, output: &Path) -> DownloadTask {
    DownloadTask {
        id: uuid::Uuid::new_v4().to_string(),
        media,
        mode: "video".into(),
        quality: "1080".into(),
        format_id: input.format_id.clone(),
        output_dir: output.to_string_lossy().into_owned(),
        status: TaskStatus::Queued,
        progress: None,
        speed: None,
        eta: None,
        downloaded_bytes: 0,
        total_bytes: None,
        final_path: None,
        error: None,
        logs: vec![],
        created_at: chrono::Utc::now().to_rfc3339(),
        updated_at: chrono::Utc::now().to_rfc3339(),
        file_exists: false,
    }
}
fn argument_summary(args: &[String]) -> Value {
    let after = |key: &str| {
        args.iter()
            .position(|v| v == key)
            .and_then(|n| args.get(n + 1))
            .cloned()
    };
    json!({"builder":"production build_download_args", "ignore_config":args.iter().any(|v|v=="--ignore-config"),
        "no_plugins":args.iter().any(|v|v=="--no-plugin-dirs"),"no_remote_components":args.iter().any(|v|v=="--no-remote-components"),
        "format_selector":after("--format"),"socket_timeout_s":after("--socket-timeout"),"retries":after("--retries"),
        "progress_delta_s":after("--progress-delta"),"external_downloader":false,"proxy":if args.iter().any(|v|v=="--proxy"){"explicit_loopback"}else{"inherited_environment_or_TUN; actual route uncertain"},
        "concurrent_fragments":after("--concurrent-fragments").unwrap_or_else(||"1 (engine default)".into()),
        "paths_and_url":"omitted; exact argv remains process-local"})
}
fn versions(diag: &EngineDiagnostics) -> Value {
    let rx = regex::Regex::new(r"\d+(?:\.\d+){1,3}").unwrap();
    let version = |text: &Option<String>| {
        text.as_ref()
            .and_then(|v| rx.find(v))
            .map(|m| m.as_str().to_owned())
    };
    json!({"yt_dlp":version(&diag.ytdlp.version),"ffmpeg":version(&diag.ffmpeg.version),"runtime":version(&diag.runtime.version),"runtime_available":diag.runtime.available})
}
fn protocol(value: &str) -> &'static str {
    match value {
        "m3u8" | "m3u8_native" => "hls",
        "http_dash_segments" => "dash",
        "https" => "https",
        "http" => "http",
        _ => "other_or_unknown",
    }
}
fn safe_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}
fn http_error(text: &str) -> Option<u16> {
    for code in [403, 429] {
        if [
            format!("HTTP Error {code}"),
            format!("HTTP error {code}"),
            format!("status code: {code}"),
            format!("{code} Forbidden"),
            format!("{code} Too Many"),
        ]
        .iter()
        .any(|p| text.contains(p))
        {
            return Some(code);
        }
    }
    None
}
fn directory_bytes(root: &Path) -> u64 {
    fn visit(root: &Path, count: &mut usize) -> u64 {
        let Ok(entries) = std::fs::read_dir(root) else {
            return 0;
        };
        let mut bytes = 0u64;
        for entry in entries.flatten() {
            *count += 1;
            if *count > 512 {
                return u64::MAX;
            }
            let Ok(meta) = std::fs::symlink_metadata(entry.path()) else {
                continue;
            };
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if meta.file_attributes() & 0x400 != 0 {
                    return u64::MAX;
                }
            }
            if meta.file_type().is_symlink() {
                return u64::MAX;
            }
            bytes = bytes.saturating_add(if meta.is_file() {
                meta.len()
            } else if meta.is_dir() {
                visit(&entry.path(), count)
            } else {
                0
            });
        }
        bytes
    }
    visit(root, &mut 0)
}
fn stages(diagnostics: &Diagnostics) -> Vec<Value> {
    diagnostics
        .read()
        .events
        .iter()
        .filter(|e| e.source == fluxdl_lib::diagnostics::Source::Rust && e.duration_ms.is_some())
        .map(|e| json!({"phase":e.phase,"status":e.status,"duration_ms":e.duration_ms}))
        .collect()
}
fn new_stage_records(diagnostics: &Diagnostics, seen: &mut HashSet<String>) -> Vec<Value> {
    diagnostics.read().events.iter().filter(|event| {
        event.source == fluxdl_lib::diagnostics::Source::Rust
            && event.phase != fluxdl_lib::diagnostics::Stage::Session
            && event.duration_ms.is_some()
            && seen.insert(event.event_id.clone())
    }).map(|event|json!({"type":"stage","stage":{"phase":event.phase,"status":event.status,"duration_ms":event.duration_ms}})).collect()
}
fn publish_stages(diagnostics: &Diagnostics, seen: &mut HashSet<String>) {
    for record in new_stage_records(diagnostics, seen) {
        emit(record);
    }
}
async fn watch_manager_pipeline<T>(
    pipeline: impl std::future::Future<Output = Result<T, AppError>>,
    diagnostics: &Diagnostics,
    seen: &mut HashSet<String>,
    limit: Duration,
    mut publish: impl FnMut(Value),
) -> Result<Result<T, AppError>, tokio::time::error::Elapsed> {
    let bounded = tokio::time::timeout(limit, pipeline);
    tokio::pin!(bounded);
    let mut tick = tokio::time::interval(Duration::from_millis(500));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            result=&mut bounded=>{
                for record in new_stage_records(diagnostics,seen) {publish(record);}
                return result;
            },
            _=tick.tick()=>{
                for record in new_stage_records(diagnostics,seen) {publish(record);}
            }
        }
    }
}
fn manager_first_progress_s(diagnostics: &Diagnostics, task_id: &str) -> Option<f64> {
    let trace = diagnostics.task_trace(task_id)?;
    diagnostics
        .read()
        .events
        .iter()
        .find(|event| {
            event.task_id.as_deref() == Some(task_id)
                && event.trace_id == trace.id()
                && event.source == fluxdl_lib::diagnostics::Source::Rust
                && event.phase == fluxdl_lib::diagnostics::Stage::DownloadFirstProgress
                && event.status == fluxdl_lib::diagnostics::Status::Ok
        })
        .and_then(|event| event.duration_ms)
        .map(|ms| ms / 1000.0)
}
fn add_network_arguments(args: &mut Vec<String>, input: &Input) -> Result<(), AppError> {
    let position = args.iter().position(|a| a == "--").ok_or_else(invalid)?;
    let mut extra = vec![];
    if let Some(proxy) = &input.proxy {
        if !proxy_valid(proxy) {
            return Err(invalid());
        }
        extra.extend(["--proxy".into(), proxy.clone()]);
    }
    if let Some(n) = input.fragments {
        if ![1, 2, 4].contains(&n) {
            return Err(invalid());
        }
        extra.extend(["--concurrent-fragments".into(), n.to_string()]);
    }
    args.splice(position..position, extra);
    Ok(())
}
fn metadata_arguments(
    input: &Input,
    settings: &Settings,
    diag: &EngineDiagnostics,
    output: &Path,
) -> Result<Vec<String>, AppError> {
    let media = MediaInfo {
        url: input.url.clone(),
        title: String::new(),
        media_id: String::new(),
        source: String::new(),
        duration: None,
        thumbnail: None,
        is_playlist: false,
        formats: vec![fluxdl_lib::models::FormatOption {
            format_id: "preflight".into(),
            ext: "mp4".into(),
            label: String::new(),
            height: Some(1080),
            has_video: true,
            has_audio: true,
            filesize: None,
            requires_ffmpeg: false,
        }],
        warnings: vec![],
    };
    let mut dummy = task(media, input, output);
    dummy.format_id = None;
    let mut args = engine::build_download_args(&dummy, settings, diag)?;
    let cutoff = args
        .iter()
        .position(|a| a == "--newline")
        .ok_or_else(invalid)?;
    args.truncate(cutoff);
    args.extend(
        [
            "--dump-single-json",
            "--skip-download",
            "--flat-playlist",
            "--playlist-end",
            "1",
            "--",
            &input.url,
        ]
        .map(str::to_owned),
    );
    add_network_arguments(&mut args, input)?;
    Ok(args)
}
async fn metadata_capture(
    input: &Input,
    output: &Path,
    settings: &Settings,
    diag: &EngineDiagnostics,
    limit: Duration,
) -> Result<(MediaInfo, Value), AppError> {
    let args = metadata_arguments(input, settings, diag, output)?;
    let mut child = OwnedCli::spawn(
        diag.ytdlp.path.as_deref().ok_or_else(invalid)?,
        &args,
        output,
    )
    .await?;
    let start = Instant::now();
    let mut stdout = vec![];
    let mut stderr = vec![];
    loop {
        let item =
            match tokio::time::timeout(limit.saturating_sub(start.elapsed()), child.output.recv())
                .await
            {
                Ok(item) => item,
                Err(_) => {
                    child.stop()?;
                    child.reap().await?;
                    return Err(cli_error(ErrorCode::ProcessTimeout));
                }
            };
        match item {
            None => break,
            Some(PipeChunk::Fault) => {
                child.stop()?;
                child.reap().await?;
                return Err(cli_error(ErrorCode::ProcessRead));
            }
            Some(PipeChunk::Data {
                stderr: is_stderr,
                bytes,
            }) => {
                let (target, bound) = if is_stderr {
                    (&mut stderr, 64 * 1024)
                } else {
                    (&mut stdout, 16 * 1024 * 1024)
                };
                if target.len() + bytes.len() > bound {
                    child.stop()?;
                    child.reap().await?;
                    return Err(cli_error(ErrorCode::EngineProtocol));
                }
                target.extend(bytes);
                if is_stderr {
                    if let Some(code) = http_error(&String::from_utf8_lossy(&stderr)) {
                        emit(json!({"type":"http_error","code":code}));
                        child.stop()?;
                        child.reap().await?;
                        return Err(cli_error(ErrorCode::ProcessExit)
                            .with_kind(fluxdl_lib::errors::ErrorKind::AccessRestricted));
                    }
                }
            }
        }
    }
    if !child.reap().await?.success() {
        return Err(cli_error(ErrorCode::ProcessExit));
    }
    let raw: Value =
        serde_json::from_slice(&stdout).map_err(|_| cli_error(ErrorCode::EngineProtocol))?;
    let media = engine::media_from_json(&raw, &input.url, &String::from_utf8_lossy(&stderr))?;
    Ok((media, raw))
}
async fn metadata(input: &Input, output: &Path) -> Result<(), AppError> {
    let settings = settings(input, output);
    let diag = engine::detect(&settings, None).await;
    if !diag.ytdlp.available {
        return Err(AppError::new(
            ErrorCode::DependencyUnavailable,
            ErrorPhase::Detect,
            "Engine unavailable",
        ));
    }
    emit(json!({"type":"versions","versions":versions(&diag)}));
    let (validated, raw) = metadata_capture(
        input,
        output,
        &settings,
        &diag,
        Duration::from_secs_f64(input.max_seconds),
    )
    .await
    .map_err(probe_error)?;
    let formats:Vec<Value>=raw.get("formats").and_then(Value::as_array).into_iter().flatten().filter_map(|f| {
        let id=f.get("format_id")?.as_str()?;if !safe_id(id)||!validated.formats.iter().any(|x|x.format_id==id){return None;}
        let proto=protocol(f.get("protocol").and_then(Value::as_str).unwrap_or_default());
        Some(json!({"id":id,"protocol":proto,"segmented":matches!(proto,"hls"|"dash"),"has_video":f.get("vcodec").and_then(Value::as_str).is_some_and(|v|v!="none"),"has_audio":f.get("acodec").and_then(Value::as_str).is_some_and(|v|v!="none"),"height":f.get("height").and_then(Value::as_u64),"filesize":f.get("filesize").and_then(Value::as_u64)}))
    }).collect();
    emit(
        json!({"type":"metadata","formats":formats,"playlist":validated.is_playlist,"warnings_present":!validated.warnings.is_empty(),"raw_metadata":"discarded; no URL/title/media signature retained"}),
    );
    Ok(())
}
async fn manager_sample(input: &Input, output: &Path) -> Result<(), AppError> {
    let logs = Diagnostics::open(
        Some(output.join("logs")),
        "network-diagnostics",
        Policy::default(),
        None,
    );
    let m = Manager::init_with_diagnostics(output.join("data"), None, None, logs.clone()).await?;
    let media_dir = output.join("downloads");
    std::fs::create_dir(&media_dir).map_err(|_| invalid())?;
    let settings = settings(input, &media_dir);
    m.update_settings(settings.clone()).await?;
    let start = Instant::now();
    let pipeline = async {
        let media = m.probe(&input.url).await?;
        m.enqueue(EnqueueInput {
            media,
            mode: "video".into(),
            quality: "1080".into(),
            format_id: input.format_id.clone(),
            output_dir: Some(media_dir.to_string_lossy().into_owned()),
        })
        .await
    };
    let mut seen_stages = HashSet::new();
    let initial = match watch_manager_pipeline(
        pipeline,
        &logs,
        &mut seen_stages,
        Duration::from_secs_f64(input.max_seconds),
        emit,
    )
    .await
    {
        Ok(result) => result?,
        Err(_) => {
            let _ = m.shutdown().await;
            emit(
                json!({"type":"result","terminal":"cancelled","stop_reason":"round_time_budget","stages":stages(&logs),"http_errors":Value::Null}),
            );
            return Ok(());
        }
    };
    let diag = m.snapshot().await.engines;
    emit(
        json!({"type":"arguments","summary":argument_summary(&engine::build_download_args(&initial,&settings,&diag)?)}),
    );
    let mut last = 0;
    let mut cumulative = 0u64;
    let mut reason = None;
    let mut stop_at = None;
    let mut first_progress_s = None;
    let mut last_stage_poll = Instant::now();
    loop {
        let t = m.task(&initial.id).await?;
        if last_stage_poll.elapsed() >= Duration::from_millis(500) || !t.status.active() {
            publish_stages(&logs, &mut seen_stages);
            last_stage_poll = Instant::now();
        }
        if first_progress_s.is_none()
            && matches!(
                t.status,
                TaskStatus::Downloading
                    | TaskStatus::Postprocessing
                    | TaskStatus::Completed
                    | TaskStatus::Failed
                    | TaskStatus::Cancelled
            )
        {
            if let Some(duration_s) = manager_first_progress_s(&logs, &initial.id) {
                first_progress_s = Some(duration_s);
                // Emit the actual Rust span immediately, so an outer owned-Job
                // deadline cannot discard it while waiting for terminal cleanup.
                emit(json!({"type":"first_progress","duration_s":duration_s}));
            }
        }
        if t.status == TaskStatus::Downloading {
            cumulative = cumulative.saturating_add(if t.downloaded_bytes >= last {
                t.downloaded_bytes - last
            } else {
                t.downloaded_bytes
            });
            last = t.downloaded_bytes;
        }
        emit(
            json!({"type":"progress","elapsed_s":start.elapsed().as_secs_f64(),"bytes":cumulative,"phase":if t.status==TaskStatus::Downloading{"downloading"}else if t.status==TaskStatus::Postprocessing{"postprocessing"}else{"waiting_or_terminal"}}),
        );
        if !t.status.active() {
            emit(
                json!({"type":"result","terminal":t.status,"stop_reason":reason,"stages":stages(&logs),"first_progress_s":first_progress_s,"http_errors":Value::Null,"retries":Value::Null,"budget_observed_bytes":cumulative.max(directory_bytes(&media_dir)),"metrics_observation":"100ms Manager snapshots; a component shorter than polling interval may be missed","raw_error_collected":false}),
            );
            break;
        }
        if reason.is_none()
            && (start.elapsed().as_secs_f64() >= input.max_seconds
                || cumulative >= input.max_bytes
                || directory_bytes(&media_dir) >= input.max_bytes)
        {
            reason = Some(if start.elapsed().as_secs_f64() >= input.max_seconds {
                "round_time_budget"
            } else {
                "round_byte_budget"
            });
            stop_at = Some(Instant::now());
            m.cancel(&initial.id).await?;
        }
        if stop_at.is_some_and(|t| t.elapsed() > Duration::from_secs(10)) {
            return Err(AppError::new(
                ErrorCode::ProcessCleanup,
                ErrorPhase::Shutdown,
                "Owned task did not settle",
            ));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    m.shutdown().await?;
    let _ = logs.flush();
    Ok(())
}
#[derive(Default)]
struct CliCounters {
    stream: Option<String>,
    retired: HashSet<String>,
    last: u64,
    cumulative: u64,
    finished: bool,
    phase: &'static str,
    first_progress_s: Option<f64>,
}
impl CliCounters {
    fn observe(&mut self, fact: &engine::ParsedProgress, elapsed: f64) -> bool {
        match &fact.event {
            ProgressEvent::Postprocessing => {
                self.phase = "postprocessing";
                true
            }
            ProgressEvent::Download {
                downloaded_bytes, ..
            } => {
                if fact
                    .stream_id
                    .as_ref()
                    .is_some_and(|s| self.retired.contains(s))
                {
                    return false;
                }
                let new_stream = fact
                    .stream_id
                    .as_ref()
                    .is_some_and(|s| self.stream.as_ref().is_some_and(|old| old != s));
                if new_stream {
                    if self.retired.len() >= 32 {
                        return false;
                    }
                    if let Some(old) = self.stream.take() {
                        self.retired.insert(old);
                    }
                    self.last = 0;
                    self.finished = false;
                }
                if self.finished && !fact.finished || *downloaded_bytes < self.last {
                    return false;
                }
                if let Some(id) = &fact.stream_id {
                    self.stream = Some(id.clone());
                }
                self.cumulative = self
                    .cumulative
                    .saturating_add(downloaded_bytes.saturating_sub(self.last));
                self.last = *downloaded_bytes;
                self.finished = fact.finished;
                self.phase = "downloading";
                if self.first_progress_s.is_none() {
                    self.first_progress_s = Some(elapsed);
                }
                true
            }
            _ => false,
        }
    }
    fn phase(&self) -> &'static str {
        if self.phase.is_empty() {
            "waiting_or_terminal"
        } else {
            self.phase
        }
    }
}
#[derive(Default)]
struct Lines {
    buffer: Vec<u8>,
    truncated: bool,
}
impl Lines {
    fn feed(&mut self, bytes: &[u8]) -> Vec<String> {
        let mut lines = vec![];
        for &byte in bytes {
            if byte == b'\n' {
                if !self.truncated {
                    lines.push(String::from_utf8_lossy(&self.buffer).into_owned());
                }
                self.buffer.clear();
                self.truncated = false;
            } else if !self.truncated {
                if self.buffer.len() < 128 * 1024 {
                    self.buffer.push(byte);
                } else {
                    self.buffer.clear();
                    self.truncated = true;
                }
            }
        }
        lines
    }
}
fn consume_cli_line(
    line: &str,
    counters: &mut CliCounters,
    start: Instant,
    spawned_at: Instant,
    errors: &mut Vec<u16>,
    retries: &mut u32,
) -> bool {
    if let Some(code) = http_error(line) {
        if !errors.contains(&code) {
            errors.push(code);
            emit(json!({"type":"http_error","code":code}));
        }
        return true;
    }
    if line.contains("Retrying") || line.contains("retrying") {
        *retries = retries.saturating_add(1);
    }
    if let Some(fact) = engine::parse_progress_fact(line) {
        let (accepted, first_progress_s) = observe_cli_progress(
            counters,
            &fact,
            start.elapsed().as_secs_f64(),
            spawned_at.duration_since(start).as_secs_f64(),
        );
        if let Some(duration_s) = first_progress_s {
            emit(json!({"type":"first_progress","duration_s":duration_s}));
        }
        if accepted {
            emit(
                json!({"type":"progress","elapsed_s":start.elapsed().as_secs_f64(),"bytes":counters.cumulative,"phase":counters.phase()}),
            );
        }
    }
    false
}
fn observe_cli_progress(
    counters: &mut CliCounters,
    fact: &engine::ParsedProgress,
    elapsed_s: f64,
    spawn_offset_s: f64,
) -> (bool, Option<f64>) {
    let first_missing = counters.first_progress_s.is_none();
    let accepted = counters.observe(fact, elapsed_s);
    let first = if accepted && first_missing {
        counters
            .first_progress_s
            .map(|s| (s - spawn_offset_s).max(0.0))
    } else {
        None
    };
    (accepted, first)
}
async fn cli_sample(input: &Input, output: &Path) -> Result<(), AppError> {
    let round_start = Instant::now();
    let limit = Duration::from_secs_f64(input.max_seconds);
    let settings = settings(input, output);
    let diag = tokio::time::timeout(limit, engine::detect(&settings, None))
        .await
        .map_err(|_| cli_error(ErrorCode::ProcessTimeout))?;
    let (media, raw) = metadata_capture(
        input,
        output,
        &settings,
        &diag,
        limit.saturating_sub(round_start.elapsed()),
    )
    .await
    .map_err(probe_error)?;
    if input.fragments.is_some_and(|n| n > 1) {
        let segmented = raw
            .get("formats")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .any(|format| {
                input.format_id.as_deref() == format.get("format_id").and_then(Value::as_str)
                    && matches!(
                        protocol(
                            format
                                .get("protocol")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                        ),
                        "hls" | "dash"
                    )
            });
        if !segmented {
            return Err(invalid());
        }
    }
    drop(raw);
    let task = task(media, input, output);
    let mut args = engine::build_download_args(&task, &settings, &diag)?;
    add_network_arguments(&mut args, input)?;
    emit(json!({"type":"arguments","summary":argument_summary(&args)}));
    let start = Instant::now();
    let mut child = OwnedCli::spawn(
        diag.ytdlp.path.as_deref().ok_or_else(invalid)?,
        &args,
        output,
    )
    .await?;
    let spawn_ms = child.spawned_at.duration_since(start).as_secs_f64() * 1000.;
    let mut counters = CliCounters::default();
    let mut lines = [Lines::default(), Lines::default()];
    let mut reason = None;
    let mut errors = vec![];
    let mut retries = 0;
    let mut parent_exited = false;
    let mut tick = tokio::time::interval(Duration::from_millis(100));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            item=child.output.recv()=>match item {
                None=>{
                    for line in lines.iter_mut().flat_map(|buffer|buffer.feed(b"\n")) {
                        if consume_cli_line(&line,&mut counters,start,child.spawned_at,&mut errors,&mut retries) && reason.is_none() {reason=Some("site_access_restricted");child.stop()?;}
                    }
                    break;
                },
                Some(PipeChunk::Fault)=>{if reason.is_none(){reason=Some("process_read_fault");child.stop()?;}},
                Some(PipeChunk::Data{stderr,bytes})=>{
                    for line in lines[usize::from(stderr)].feed(&bytes) {
                        if consume_cli_line(&line,&mut counters,start,child.spawned_at,&mut errors,&mut retries) && reason.is_none() {reason=Some("site_access_restricted");child.stop()?;}
                    }
                    if reason.is_none() && counters.cumulative>=input.max_bytes {reason=Some("round_byte_budget");child.stop()?;}
                }
            },
            _=tick.tick()=>{
                emit(json!({"type":"progress","elapsed_s":start.elapsed().as_secs_f64(),"bytes":counters.cumulative,"phase":counters.phase()}));
                if reason.is_none() && (round_start.elapsed()>=limit || directory_bytes(output)>=input.max_bytes) {
                    reason=Some(if round_start.elapsed()>=limit {"round_time_budget"}else{"round_byte_budget"});child.stop()?;
                }
                if !parent_exited && child.child.try_wait().map_err(|_|cli_error(ErrorCode::ProcessWait))?.is_some() {
                    parent_exited=true;child.stop()?;
                }
            }
        }
    }
    let status = child.reap().await?;
    let mut stages =
        vec![json!({"phase":"download_engine_spawn","status":"ok","duration_ms":spawn_ms})];
    let first_progress_s = counters
        .first_progress_s
        .map(|n| (n - spawn_ms / 1000.0).max(0.0));
    if let Some(first) = first_progress_s {
        stages.push(
            json!({"phase":"download_first_progress","status":"ok","duration_ms":first*1000.}),
        );
    }
    emit(
        json!({"type":"result","terminal":if reason.is_some(){"cancelled"}else if status.success(){"completed"}else{"failed"},"stop_reason":reason,"stages":stages,"first_progress_s":first_progress_s,"http_errors":errors,"retries":retries,"budget_observed_bytes":counters.cumulative.max(directory_bytes(output)),"consumer":"independent CLI pipes and byte counters; shared only production argv, protocol parser, low-level Job guard"}),
    );
    Ok(())
}
#[tokio::main]
async fn main() {
    // Bounded stdin, deliberately no CLI URL arguments and no raw panic/error formatting.
    let mut raw = String::new();
    if std::io::stdin()
        .take(32 * 1024)
        .read_to_string(&mut raw)
        .is_err()
    {
        emit(json!({"type":"failure","error_code":"INVALID_INPUT"}));
        return;
    }
    let Ok(input) = serde_json::from_str::<Input>(&raw) else {
        emit(json!({"type":"failure","error_code":"INVALID_INPUT"}));
        return;
    };
    let result = async {
        let output = validate(&input)?;
        match input.action.as_str() {
            "metadata" => metadata(&input, &output).await,
            "manager" => manager_sample(&input, &output).await,
            "cli" => cli_sample(&input, &output).await,
            _ => Err(invalid()),
        }
    }
    .await;
    if let Err(error) = result {
        emit(safe_failure(&error));
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn input() -> Input {
        Input {
            action: "cli".into(),
            url: "https://fixture.invalid/PRIVATE_VIDEO?token=PRIVATE_TOKEN".into(),
            engine_path: String::new(),
            ffmpeg_path: String::new(),
            run_root: String::new(),
            sample_root: String::new(),
            format_id: None,
            proxy: None,
            fragments: None,
            max_seconds: 30.0,
            max_bytes: 40 * 1024 * 1024,
        }
    }
    fn fixture() -> (Input, Settings, EngineDiagnostics, PathBuf, DownloadTask) {
        let input = input();
        let output = std::env::temp_dir();
        let settings = settings(&input, &output);
        let diag = EngineDiagnostics {
            ytdlp: fluxdl_lib::models::EngineInfo {
                available: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let media = MediaInfo {
            url: input.url.clone(),
            title: "PRIVATE_TITLE".into(),
            media_id: "PRIVATE_ID".into(),
            source: String::new(),
            duration: None,
            thumbnail: None,
            is_playlist: false,
            formats: vec![fluxdl_lib::models::FormatOption {
                format_id: "18".into(),
                ext: "mp4".into(),
                label: "PRIVATE_LABEL".into(),
                height: Some(360),
                has_video: true,
                has_audio: true,
                filesize: None,
                requires_ffmpeg: false,
            }],
            warnings: vec![],
        };
        let task = task(media, &input, &output);
        (input, settings, diag, output, task)
    }
    #[test]
    fn cli_argv_is_exact_production_argv_and_proxy_applies_to_extraction_too() {
        let (mut input, settings, diag, output, task) = fixture();
        let production = engine::build_download_args(&task, &settings, &diag).unwrap();
        let mut plain = production.clone();
        add_network_arguments(&mut plain, &input).unwrap();
        assert_eq!(plain, production);
        for proxy in ["http://127.0.0.1:7897", "socks5://127.0.0.1:7897"] {
            input.proxy = Some(proxy.into());
            let mut explicit = production.clone();
            add_network_arguments(&mut explicit, &input).unwrap();
            let position = explicit.iter().position(|arg| arg == "--proxy").unwrap();
            assert_eq!(explicit[position + 1], proxy);
            assert!(position < explicit.iter().position(|arg| arg == "--").unwrap());
            explicit.drain(position..position + 2);
            assert_eq!(explicit, production);
            let meta = metadata_arguments(&input, &settings, &diag, &output).unwrap();
            let p = meta.iter().position(|arg| arg == "--proxy").unwrap();
            assert_eq!(meta[p + 1], proxy);
            assert!(p < meta.iter().position(|arg| arg == "--").unwrap());
            let summary = argument_summary(&meta).to_string();
            assert!(!summary.contains("PRIVATE") && !summary.contains("fixture.invalid"));
        }
    }
    #[test]
    fn proxy_only_accepts_explicit_local_ports_without_credentials_or_paths() {
        for valid in [
            "http://127.0.0.1:7897",
            "http://localhost:80",
            "socks5://localhost:1080",
            "socks5://[::1]:1080/",
        ] {
            assert!(proxy_valid(valid), "{valid}");
        }
        for invalid in [
            "http://localhost",
            "http://127.0.0.1:0",
            "http://user:secret@127.0.0.1:7897",
            "socks5://user@localhost:1080",
            "http://127.0.0.1:7897/private",
            "http://127.0.0.1:7897/?token=private",
            "http://127.0.0.1:7897/#private",
            "http://remote.invalid:7897",
            "file:///private",
        ] {
            assert!(!proxy_valid(invalid), "{invalid}");
        }
    }
    #[test]
    fn manager_first_progress_uses_current_trace_rust_span_only() {
        use fluxdl_lib::diagnostics::{Stage, Status};
        let diagnostics = Diagnostics::disabled();
        let task = uuid::Uuid::new_v4().to_string();
        let other = diagnostics.trace(None, None, Some(&task));
        other.begin(Stage::DownloadFirstProgress);
        other.finish(Stage::DownloadFirstProgress, Status::Ok, None);
        let current = diagnostics.trace(None, None, Some(&task));
        diagnostics.attach(&task, current.clone());
        current.begin(Stage::ParseTotal);
        current.finish(Stage::ParseTotal, Status::Ok, None);
        assert!(manager_first_progress_s(&diagnostics, &task).is_none());
        current.begin(Stage::DownloadFirstProgress);
        current.finish(Stage::DownloadFirstProgress, Status::Ok, None);
        let measured = diagnostics
            .read()
            .events
            .into_iter()
            .find(|e| {
                e.trace_id == current.id()
                    && e.phase == Stage::DownloadFirstProgress
                    && e.status == Status::Ok
            })
            .unwrap()
            .duration_ms
            .unwrap()
            / 1000.0;
        assert_eq!(
            manager_first_progress_s(&diagnostics, &task),
            Some(measured)
        );
        assert!(
            manager_first_progress_s(&diagnostics, &uuid::Uuid::new_v4().to_string()).is_none()
        );
    }
    #[tokio::test]
    async fn completed_stages_are_published_before_an_unfinished_pipeline_deadline() {
        use fluxdl_lib::diagnostics::{Stage, Status};
        let diagnostics = Diagnostics::disabled();
        let trace = diagnostics.trace(None, None, None);
        let pipeline = async {
            trace.begin(Stage::ParseValidate);
            trace.finish(Stage::ParseValidate, Status::Ok, None);
            trace.begin(Stage::ParseMetadataExtract);
            std::future::pending::<Result<(), AppError>>().await
        };
        let mut seen = HashSet::new();
        let mut records = vec![];
        let start = Instant::now();
        let limit = Duration::from_millis(1200);
        let result = watch_manager_pipeline(pipeline, &diagnostics, &mut seen, limit, |value| {
            records.push((start.elapsed(), value))
        })
        .await;
        assert!(result.is_err());
        assert_eq!(
            records.len(),
            1,
            "completed span was duplicated or unfinished span was invented"
        );
        assert!(
            records[0].0 < Duration::from_millis(1100),
            "evidence only appeared after the terminal deadline"
        );
        let record = &records[0].1;
        assert_eq!(record["type"], "stage");
        assert_eq!(record["stage"]["phase"], "parse_validate");
        assert_eq!(record["stage"]["status"], "ok");
        assert!(record["stage"]["duration_ms"].is_number());
        assert_eq!(record["stage"].as_object().unwrap().len(), 3);
        assert!(!record.to_string().contains(trace.id()));
        assert!(new_stage_records(&diagnostics, &mut seen).is_empty());
    }
    #[test]
    fn independent_cli_first_progress_is_available_once_before_any_terminal_result() {
        let first=engine::parse_progress_fact(r#"__FLUX_PROGRESS__{"progress.status":"downloading","progress.downloaded_bytes":1024,"progress.filename":"PRIVATE_PATH","info.protocol":"https"}"#).unwrap();
        let mut counters = CliCounters::default();
        let (accepted, duration) = observe_cli_progress(&mut counters, &first, 0.75, 0.25);
        assert!(accepted);
        assert_eq!(duration, Some(0.5));
        let record = json!({"type":"first_progress","duration_s":duration.unwrap()});
        assert!(!record.to_string().contains("PRIVATE"));
        assert_eq!(
            observe_cli_progress(&mut counters, &first, 1.0, 0.25).1,
            None
        );
        let mut unseen = CliCounters::default();
        let post = engine::ParsedProgress {
            event: ProgressEvent::Postprocessing,
            stream_id: None,
            finished: false,
            protocol: None,
        };
        assert_eq!(observe_cli_progress(&mut unseen, &post, 7.0, 0.25).1, None);
    }
    #[test]
    fn independent_counters_ignore_regressions_retired_streams_and_private_strings() {
        let fact = |id: &str, bytes, finished| engine::ParsedProgress {
            event: ProgressEvent::Download {
                downloaded_bytes: bytes,
                total_bytes: None,
                speed: None,
                eta: None,
                progress: None,
            },
            stream_id: Some(id.into()),
            finished,
            protocol: None,
        };
        let mut counter = CliCounters::default();
        assert!(counter.observe(&fact("PRIVATE_STREAM_A", 100, false), 1.));
        assert!(counter.observe(&fact("PRIVATE_STREAM_A", 200, true), 2.));
        assert!(!counter.observe(&fact("PRIVATE_STREAM_A", 150, false), 3.));
        assert!(counter.observe(&fact("PRIVATE_STREAM_B", 50, false), 4.));
        assert!(!counter.observe(&fact("PRIVATE_STREAM_A", 9999, true), 5.));
        assert_eq!(counter.cumulative, 250);
        assert_eq!(counter.first_progress_s, Some(1.));
        let safe = json!({"bytes":counter.cumulative,"phase":counter.phase()}).to_string();
        assert!(!safe.contains("PRIVATE"));
        let mut lines = Lines::default();
        assert!(lines.feed(&vec![b'x'; 128 * 1024 + 1]).is_empty());
        assert!(lines.feed(b"\n").is_empty());
        assert_eq!(lines.feed(b"safe\n"), vec!["safe".to_owned()]);
    }
    #[test]
    fn ownership_budget_and_directory_boundaries_are_checked_before_spawn() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let allowed = workspace.join("artifacts/network-diagnostics");
        let run = allowed.join(format!("rust-validation-{}", uuid::Uuid::new_v4()));
        let sample = run.join("sample");
        std::fs::create_dir_all(&sample).unwrap();
        std::fs::write(
            run.join(".network-owner.json"),
            br#"{"owner":"fluxdl-network-diagnostics-v1"}"#,
        )
        .unwrap();
        let mut input = input();
        input.run_root = run.to_string_lossy().into_owned();
        input.sample_root = sample.to_string_lossy().into_owned();
        assert!(validate(&input).is_ok());
        input.max_seconds = 46.;
        assert!(validate(&input).is_err());
        input.max_seconds = 30.;
        input.max_bytes = 40 * 1024 * 1024 + 1;
        assert!(validate(&input).is_err());
        input.max_bytes = 1;
        input.sample_root = run.join("sample/../sample").to_string_lossy().into_owned();
        assert!(validate(&input).is_err());
        input.sample_root = std::env::temp_dir().to_string_lossy().into_owned();
        assert!(validate(&input).is_err());
        assert!(run.starts_with(&allowed) && run != allowed);
        std::fs::remove_dir_all(run).unwrap();
    }
    #[cfg(windows)]
    #[tokio::test]
    async fn standalone_cli_reads_both_pipes_and_stops_only_its_owned_child() {
        let powershell =
            PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into()))
                .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let script="[Console]::Out.WriteLine('safe-out'); [Console]::Error.WriteLine('PRIVATE_STDERR_MUST_NOT_BE_EMITTED'); Start-Sleep -Seconds 60";
        let args = ["-NoProfile", "-NonInteractive", "-Command", script].map(str::to_owned);
        let mut child = OwnedCli::spawn(powershell.to_str().unwrap(), &args, &std::env::temp_dir())
            .await
            .unwrap();
        let mut streams = [false, false];
        tokio::time::timeout(Duration::from_secs(8), async {
            while !streams.iter().all(|v| *v) {
                if let Some(PipeChunk::Data { stderr, .. }) = child.output.recv().await {
                    streams[usize::from(stderr)] = true;
                }
            }
        })
        .await
        .unwrap();
        child.stop().unwrap();
        child.reap().await.unwrap();
        assert!(child.child.try_wait().unwrap().is_some());
    }
}
