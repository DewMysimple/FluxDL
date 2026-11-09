//! Repeatable synthetic persistence/serialization benchmark; never reads user data.
use fluxdl_lib::{
    manager::Manager,
    models::*,
    persistence::{Mutation, PersistenceWriter},
};
use serde::Serialize;
use std::{
    path::{Path, PathBuf},
    time::Instant,
};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Timing {
    samples: usize,
    p50_ms: f64,
    p95_ms: f64,
    min_ms: f64,
    max_ms: f64,
}
impl Timing {
    fn of(mut samples: Vec<f64>) -> Self {
        samples.sort_by(f64::total_cmp);
        let percentile =
            |p: f64| samples[((samples.len() as f64 * p).ceil() as usize).saturating_sub(1)];
        Self {
            samples: samples.len(),
            p50_ms: percentile(0.5),
            p95_ms: percentile(0.95),
            min_ms: samples[0],
            max_ms: *samples.last().unwrap(),
        }
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Dataset {
    history_tasks: usize,
    active_tasks: usize,
    database_load: Timing,
    insert_task: Timing,
    update_task: Timing,
    progress_batch: Timing,
    progress_writes_per_batch: usize,
    terminal_write: Timing,
    snapshot_serialization: Timing,
    snapshot_bytes: usize,
    explicit_snapshot_refresh: Timing,
    explicit_snapshot_tasks: usize,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Report {
    label: String,
    started_at: String,
    os: &'static str,
    architecture: &'static str,
    profile: &'static str,
    persistence_api: &'static str,
    temporary_data_only: bool,
    network_used: bool,
    notes: Vec<&'static str>,
    datasets: Vec<Dataset>,
}
struct TemporaryDirectory(PathBuf);
impl TemporaryDirectory {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let base = std::fs::canonicalize(std::env::temp_dir())?;
        let directory = base.join(format!("fluxdl-engineering-bench-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory)?;
        Ok(Self(directory))
    }
}
impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let safe = std::fs::canonicalize(std::env::temp_dir())
            .ok()
            .is_some_and(|base| self.0.starts_with(base))
            && self.0.file_name().is_some_and(|name| {
                name.to_string_lossy()
                    .starts_with("fluxdl-engineering-bench-")
            });
        if safe {
            if let Err(error) = std::fs::remove_dir_all(&self.0) {
                eprintln!("Temporary benchmark cleanup failed: {error}");
            }
        }
    }
}
fn task(index: usize, directory: &Path, status: TaskStatus) -> DownloadTask {
    DownloadTask {
        id: format!("bench-{index:06}"),
        media: MediaInfo {
            trusted_preview_ref: None,
            url: format!("https://example.invalid/media/{index}"),
            title: format!("工程性能基准 · 授权测试素材 {index:06}"),
            media_id: index.to_string(),
            source: "Synthetic fixture".into(),
            duration: Some(33.023),
            thumbnail: None,
            is_playlist: false,
            formats: vec![FormatOption {
                format_id: "direct-720".into(),
                ext: "mp4".into(),
                label: "720p · MP4".into(),
                height: Some(720),
                has_video: true,
                has_audio: true,
                filesize: Some(3_889_885),
                requires_ffmpeg: false,
            }],
            warnings: vec![],
        },
        mode: "video".into(),
        quality: "720".into(),
        format_id: Some("direct-720".into()),
        output_dir: directory
            .join("missing-media")
            .to_string_lossy()
            .into_owned(),
        status,
        progress: None,
        speed: None,
        eta: None,
        downloaded_bytes: 3_889_885,
        total_bytes: Some(3_889_885),
        final_path: Some(
            directory
                .join("missing-media")
                .join(format!("history-{index:06}.mp4"))
                .to_string_lossy()
                .into_owned(),
        ),
        error: None,
        logs: vec![
            "[generic] Synthetic fixture".into(),
            "[download] Test output only".into(),
            "[Merger] Processing finished".into(),
        ],
        created_at: format!("2026-01-01T00:{:02}:{:02}Z", (index / 60) % 60, index % 60),
        updated_at: "2026-01-01T01:00:00Z".into(),
        file_exists: false,
    }
}
fn measure(
    mut operation: impl FnMut() -> Result<(), Box<dyn std::error::Error>>,
    count: usize,
) -> Result<Timing, Box<dyn std::error::Error>> {
    let mut samples = Vec::with_capacity(count);
    for _ in 0..count {
        let start = Instant::now();
        operation()?;
        samples.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    Ok(Timing::of(samples))
}
fn benchmark(
    size: usize,
    samples: usize,
    progress_writes: usize,
) -> Result<Dataset, Box<dyn std::error::Error>> {
    let temporary = TemporaryDirectory::new()?;
    let directory = &temporary.0;
    let settings = Settings {
        download_dir: directory
            .join("missing-media")
            .to_string_lossy()
            .into_owned(),
        ..Settings::default()
    };
    let history = (0..size)
        .map(|i| task(i, directory, TaskStatus::Completed))
        .collect::<Vec<_>>();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    let (writer, _) = runtime.block_on(PersistenceWriter::open(directory.clone()))?;
    let mut seq = 0i64;
    let mut next = || {
        seq += 1;
        seq
    };
    let mut seed = vec![Mutation::UpsertSettings {
        seq: next(),
        settings: settings.clone(),
    }];
    for task in &history {
        seed.push(Mutation::UpsertTask {
            seq: next(),
            task: task.clone(),
        });
    }
    runtime.block_on(writer.apply(seed))?;
    runtime.block_on(writer.close())?;
    drop(writer);
    let database_load = measure(
        || {
            let (writer, loaded) = runtime.block_on(PersistenceWriter::open(directory.clone()))?;
            assert_eq!(loaded.tasks.len(), size);
            runtime.block_on(writer.close())?;
            drop(writer);
            Ok(())
        },
        samples,
    )?;
    let (writer, loaded) = runtime.block_on(PersistenceWriter::open(directory.clone()))?;
    seq = seq.max(loaded.max_mutation_seq);
    let mut next = || {
        seq += 1;
        seq
    };
    let mut current = task(size, directory, TaskStatus::Queued);
    current.final_path = None;
    current.downloaded_bytes = 0;
    current.total_bytes = Some(10_000_000);
    let mut insert_samples = Vec::with_capacity(samples);
    for index in 0..samples {
        let mut inserted = current.clone();
        inserted.id = format!("bench-insert-{index}");
        let change = Mutation::UpsertTask {
            seq: next(),
            task: inserted.clone(),
        };
        let start = Instant::now();
        runtime.block_on(writer.apply(vec![change]))?;
        insert_samples.push(start.elapsed().as_secs_f64() * 1000.0);
        runtime.block_on(writer.apply(vec![Mutation::DeleteTask {
            seq: next(),
            task_id: inserted.id,
        }]))?;
    }
    let insert_task = Timing::of(insert_samples);
    current.status = TaskStatus::Downloading;
    runtime.block_on(writer.apply(vec![Mutation::UpsertTask {
        seq: next(),
        task: current.clone(),
    }]))?;
    let update_task = measure(
        || {
            current.downloaded_bytes += 100;
            current.progress = Some(current.downloaded_bytes as f64 / 100_000.0);
            runtime.block_on(writer.apply(vec![Mutation::Progress {
                seq: next(),
                task: current.clone(),
            }]))?;
            Ok(())
        },
        samples,
    )?;
    let progress_batch = measure(
        || {
            for _ in 0..progress_writes {
                current.downloaded_bytes += 100;
                current.progress = Some(current.downloaded_bytes as f64 / 100_000.0);
                runtime.block_on(writer.apply(vec![Mutation::Progress {
                    seq: next(),
                    task: current.clone(),
                }]))?;
            }
            Ok(())
        },
        samples,
    )?;
    let mut terminal_samples = Vec::with_capacity(samples);
    for _ in 0..samples {
        current.status = TaskStatus::Downloading;
        runtime.block_on(writer.apply(vec![Mutation::UpsertTask {
            seq: next(),
            task: current.clone(),
        }]))?;
        current.status = TaskStatus::Completed;
        current.progress = None;
        current.speed = None;
        current.eta = None;
        let change = Mutation::UpsertTask {
            seq: next(),
            task: current.clone(),
        };
        let start = Instant::now();
        runtime.block_on(writer.apply(vec![change]))?;
        terminal_samples.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    let terminal_write = Timing::of(terminal_samples);
    runtime.block_on(writer.flush())?;
    runtime.block_on(writer.close())?;
    drop(writer);
    // This is the actual production explicit refresh path, outside repository loading.
    let manager = runtime.block_on(Manager::init_with_diagnostics(
        directory.clone(),
        None,
        None,
        fluxdl_lib::diagnostics::Diagnostics::disabled(),
    ))?;
    let explicit_snapshot_refresh = measure(
        || {
            let snapshot = runtime.block_on(manager.snapshot());
            assert_eq!(snapshot.tasks.len(), size + 1);
            std::hint::black_box(snapshot);
            Ok(())
        },
        samples,
    )?;
    runtime.block_on(manager.shutdown())?;
    drop(manager);
    let snapshot = AppSnapshot {
        session_id: "benchmark-session".into(),
        revision: 0,
        settings,
        engines: EngineDiagnostics::default(),
        tasks: history,
    };
    let snapshot_bytes = serde_json::to_vec(&snapshot)?.len();
    let snapshot_serialization = measure(
        || {
            std::hint::black_box(serde_json::to_vec(&snapshot)?);
            Ok(())
        },
        samples,
    )?;
    eprintln!("Measured {size} history tasks: update p50={:.3}ms; progress batch p50={:.3}ms; snapshot={snapshot_bytes}B",update_task.p50_ms,progress_batch.p50_ms);
    Ok(Dataset {
        history_tasks: size,
        active_tasks: 1,
        database_load,
        insert_task,
        update_task,
        progress_batch,
        progress_writes_per_batch: progress_writes,
        terminal_write,
        snapshot_serialization,
        snapshot_bytes,
        explicit_snapshot_refresh,
        explicit_snapshot_tasks: size + 1,
    })
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut output = PathBuf::from("artifacts/engineering-performance.json");
    let mut label = "after".to_string();
    let mut samples = 20usize;
    let mut progress_writes = 20usize;
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--output" => output = arguments.next().ok_or("--output needs a path")?.into(),
            "--label" => label = arguments.next().ok_or("--label needs a value")?,
            "--samples" => samples = arguments.next().ok_or("--samples needs a value")?.parse()?,
            "--progress-writes" => {
                progress_writes = arguments
                    .next()
                    .ok_or("--progress-writes needs a value")?
                    .parse()?
            }
            _ => return Err(format!("Unknown argument: {argument}").into()),
        }
    }
    if samples < 2 || progress_writes == 0 {
        return Err("At least 2 samples and 1 progress write are required".into());
    }
    if label == "before"
        || output
            .file_name()
            .is_some_and(|name| name == "performance-before.json")
    {
        return Err("The immutable pre-refactor baseline must not be overwritten by the incremental implementation".into());
    }
    let started_at = chrono::Utc::now().to_rfc3339();
    let mut datasets = Vec::new();
    for size in [100, 1000, 5000] {
        datasets.push(benchmark(size, samples, progress_writes)?);
    }
    let report=Report{label,started_at,os:std::env::consts::OS,architecture:std::env::consts::ARCH,profile:if cfg!(debug_assertions){"debug"}else{"release"},persistence_api:"PersistenceWriter::open + incremental apply with durable ack",temporary_data_only:true,network_used:false,notes:vec!["20 nearest-rank percentile samples by default; warm OS cache, not a cold-disk benchmark.","History fixture shape is unchanged: one format, three log lines, intentionally missing temporary media paths.","Load includes actual owned-thread open/load/close; it no longer includes historical file stat or whole-table startup rewrite.","Insert/update/progress/terminal use real single-record worker mutations including channel and durable ack, not the bulk fixture importer.","Each progress batch still consists of 20 persistence flush points with no real-time sleeps.","Snapshot serialization still uses N history tasks and excludes filesystem/IPC/render; payload fixture is unchanged.","ExplicitSnapshotRefresh separately measures production Manager::snapshot cloning and forced filesystem refresh for N+1 tasks; local engine version detection at Manager init is excluded from that timer.","All data is temporary; only local version commands run, no real network or user database access.","Before JSON is immutable historical output. Database load speedup must not be described as the same end-to-end application startup speedup."],datasets};
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&output, serde_json::to_string_pretty(&report)? + "\n")?;
    println!("Benchmark completed: {}", output.display());
    Ok(())
}
