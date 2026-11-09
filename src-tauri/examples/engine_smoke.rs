//! Opt-in integration run against a user-installed yt-dlp and an authorized public URL.
//! cargo run --example engine_smoke -- <yt-dlp.exe> [url] [artifacts-directory] [video|audio]
use fluxdl_lib::{
    manager::Manager,
    models::{DownloadTask, EnqueueInput, TaskStatus},
};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let started = Instant::now();
    let args: Vec<String> = std::env::args().collect();
    let engine = args
        .get(1)
        .ok_or("用法：engine_smoke <yt-dlp.exe> [url] [artifacts目录] [video|audio]")?;
    let url = args
        .get(2)
        .map(String::as_str)
        .unwrap_or("https://download.blender.org/peach/trailer/trailer_400p.ogg");
    let base = PathBuf::from(
        args.get(3)
            .cloned()
            .unwrap_or_else(|| "../artifacts".into()),
    );
    std::fs::create_dir_all(&base)?;
    let base = std::fs::canonicalize(base)?;
    let output = base.join("test-media");
    let data = base.join("e2e-engine");
    std::fs::create_dir_all(&output)?;
    let backend = Manager::init(data.clone(), None, None).await?;
    let mut settings = backend.snapshot().await.settings;
    settings.engine_path = std::fs::canonicalize(engine)?
        .to_string_lossy()
        .into_owned();
    settings.download_dir = output.to_string_lossy().into_owned();
    settings.completion_action = "none".into();
    settings = backend.update_settings(settings).await?;
    let diagnostics = backend.snapshot().await.engines;
    if !diagnostics.ytdlp.available {
        return Err(diagnostics.ytdlp.message.into());
    }
    println!(
        "Engine: {} ({})",
        diagnostics.ytdlp.version.as_deref().unwrap_or("unknown"),
        diagnostics.ytdlp.path.as_deref().unwrap_or("unknown")
    );
    let media = backend.probe(url).await?;
    println!(
        "Media: {} · {} formats · {}",
        media.title,
        media.formats.len(),
        media.source
    );
    let mode = args.get(4).cloned().unwrap_or_else(|| {
        if media.formats.iter().any(|f| f.has_video) {
            "video".into()
        } else {
            "audio".into()
        }
    });
    let input = || EnqueueInput {
        media: media.clone(),
        mode: mode.clone(),
        quality: settings.quality.clone(),
        format_id: None,
        output_dir: Some(output.to_string_lossy().into_owned()),
    };
    let first = backend.enqueue(input()).await?;
    let second = backend.enqueue(input()).await?;
    let ids = [first.id, second.id];
    let mut max_running = 0;
    let mut progress_seen = false;
    let mut last_states = String::new();
    let tasks: Vec<DownloadTask> = loop {
        let snapshot = backend.snapshot().await;
        let tasks: Vec<_> = snapshot
            .tasks
            .into_iter()
            .filter(|t| ids.contains(&t.id))
            .collect();
        let running = tasks.iter().filter(|t| t.status.running()).count();
        max_running = max_running.max(running);
        if running > 1 {
            backend.shutdown().await?;
            return Err("串行队列异常：两个任务同时运行".into());
        }
        progress_seen |= tasks.iter().any(|t| t.downloaded_bytes > 0);
        let states = tasks
            .iter()
            .map(|t| format!("{}={:?} {:?}%", &t.id[..8], t.status, t.progress))
            .collect::<Vec<_>>()
            .join("; ");
        if states != last_states {
            println!("{states}");
            last_states = states;
        }
        if tasks.iter().any(|t| {
            matches!(
                t.status,
                TaskStatus::Failed | TaskStatus::Cancelled | TaskStatus::Interrupted
            )
        }) {
            let error = tasks
                .iter()
                .find_map(|t| t.error.clone())
                .unwrap_or_else(|| "下载未正常完成".into());
            backend.shutdown().await?;
            return Err(error.into());
        }
        if tasks.len() == 2 && tasks.iter().all(|t| t.status == TaskStatus::Completed) {
            break tasks;
        }
        if started.elapsed() > Duration::from_secs(240) {
            backend.shutdown().await?;
            return Err("真实联调超过 240 秒，请检查网络".into());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    for task in &tasks {
        let file = task.final_path.as_deref().ok_or("任务未记录最终路径")?;
        let meta = std::fs::metadata(file)?;
        if !meta.is_file() || meta.len() == 0 {
            return Err("下载文件为空或不存在".into());
        }
        println!("Verified: {file} ({} bytes)", meta.len());
    }
    backend.shutdown().await?;
    drop(backend);
    let restarted = Manager::init(data, None, None).await?;
    let snapshot = restarted.snapshot().await;
    if snapshot.settings.engine_path != settings.engine_path
        || snapshot.settings.download_dir != settings.download_dir
    {
        return Err("重启后设置未保留".into());
    }
    if !ids.iter().all(|id| {
        snapshot
            .tasks
            .iter()
            .any(|t| &t.id == id && t.status == TaskStatus::Completed && t.file_exists)
    }) {
        return Err("重启后历史或文件状态未保留".into());
    }
    restarted.shutdown().await?;
    let report = serde_json::json!({"passed":true,"publicUrl":url,"mode":mode,"elapsedSeconds":started.elapsed().as_secs_f64(),"maxSimultaneousRunningTasks":max_running,"realProgressObserved":progress_seen,"persistenceVerified":true,"engines":diagnostics,"media":media,"tasks":tasks});
    std::fs::write(
        base.join("engine-smoke-report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!(
        "PASS in {:.2}s; report: {}",
        started.elapsed().as_secs_f64(),
        base.join("engine-smoke-report.json").display()
    );
    Ok(())
}
