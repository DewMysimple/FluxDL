//! Production Manager + a deterministic fixture, with independent monotonic wall measurements.
use fluxdl_lib::{
    diagnostics::{Diagnostics, Filter, Policy, Stage, Status},
    manager::Manager,
    models::{DownloadTask, EnqueueInput, TaskStatus},
};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
async fn terminal(manager: &Manager, id: &str) -> DownloadTask {
    let start = Instant::now();
    loop {
        let t = manager.task(id).await.unwrap();
        if !t.status.active() {
            return t;
        }
        assert!(start.elapsed() < Duration::from_secs(15));
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}
async fn run(root: &Path, engine: &Path, enabled: bool, n: usize) -> serde_json::Value {
    let diagnostics = Diagnostics::open(
        if enabled {
            Some(root.join("logs"))
        } else {
            None
        },
        if enabled { "fixture" } else { "test-disabled" },
        Policy::default(),
        None,
    );
    let m = Manager::init_with_diagnostics(root.join("data"), None, None, diagnostics.clone())
        .await
        .unwrap();
    let out = root.join("media");
    std::fs::create_dir_all(&out).unwrap();
    let mut settings = m.snapshot().await.settings;
    settings.engine_path = engine.to_string_lossy().into_owned();
    settings.download_dir = out.to_string_lossy().into_owned();
    m.update_settings(settings).await.unwrap();
    let mut parse_ms = Vec::new();
    let mut enqueue_ms = Vec::new();
    for index in 0..n {
        let url = format!("https://fixture.invalid/success-{index}");
        let start = Instant::now();
        let media = m.probe(&url).await.unwrap();
        parse_ms.push(start.elapsed().as_secs_f64() * 1000.);
        let start = Instant::now();
        let task = m
            .enqueue(EnqueueInput {
                media,
                mode: "video".into(),
                quality: "1080".into(),
                format_id: Some("direct".into()),
                output_dir: None,
            })
            .await
            .unwrap();
        enqueue_ms.push(start.elapsed().as_secs_f64() * 1000.);
        assert_eq!(terminal(&m, &task.id).await.status, TaskStatus::Completed);
    }
    let normal_statistics = diagnostics.overview(&Filter::default()).unwrap().statistics;
    if enabled {
        assert!(m.probe("https://fixture.invalid/parse-fail").await.is_err());
        let name = format!("recheck-fail-{}", uuid::Uuid::new_v4());
        let media = m
            .probe(&format!("https://fixture.invalid/{name}"))
            .await
            .unwrap();
        assert!(m
            .enqueue(EnqueueInput {
                media,
                mode: "video".into(),
                quality: "1080".into(),
                format_id: None,
                output_dir: None
            })
            .await
            .is_err());
        for scenario in ["no-progress", "download-fail", "postprocess"] {
            let media = m
                .probe(&format!("https://fixture.invalid/{scenario}"))
                .await
                .unwrap();
            let t = m
                .enqueue(EnqueueInput {
                    media,
                    mode: "video".into(),
                    quality: "1080".into(),
                    format_id: None,
                    output_dir: None,
                })
                .await
                .unwrap();
            let end = terminal(&m, &t.id).await;
            assert_eq!(
                end.status,
                if scenario == "download-fail" {
                    TaskStatus::Failed
                } else {
                    TaskStatus::Completed
                }
            );
            let events = diagnostics
                .query(&Filter {
                    task_id: Some(t.id.clone()),
                    limit: 200,
                    ..Default::default()
                })
                .unwrap()
                .events;
            if scenario == "no-progress" {
                assert!(events
                    .iter()
                    .any(|e| e.phase == Stage::DownloadFirstProgress
                        && e.status == Status::Unobserved
                        && e.duration_ms.is_none()));
            }
        }
        let media = m.probe("https://fixture.invalid/slow").await.unwrap();
        let input = || EnqueueInput {
            media: media.clone(),
            mode: "video".into(),
            quality: "1080".into(),
            format_id: None,
            output_dir: None,
        };
        let a = m.enqueue(input()).await.unwrap();
        let b = m.enqueue(input()).await.unwrap();
        m.cancel(&b.id).await.unwrap();
        assert_eq!(terminal(&m, &b.id).await.status, TaskStatus::Cancelled);
        m.cancel(&a.id).await.unwrap();
        assert_eq!(terminal(&m, &a.id).await.status, TaskStatus::Cancelled);
        m.retry(&a.id).await.unwrap();
        assert_eq!(terminal(&m, &a.id).await.status, TaskStatus::Completed);
        let retry_events = diagnostics
            .query(&Filter {
                task_id: Some(a.id.clone()),
                limit: 200,
                ..Default::default()
            })
            .unwrap()
            .events;
        let attempts: std::collections::HashSet<_> = retry_events
            .iter()
            .filter_map(|e| e.fields.attempt_id.as_ref())
            .collect();
        assert!(
            attempts.len() >= 2,
            "retry must have an independent attempt ID"
        );
        let before = diagnostics.task_trace(&a.id).unwrap().id().to_owned();
        m.retry(&a.id)
            .await
            .expect_err("completed task must remain nonretryable");
        assert_eq!(diagnostics.task_trace(&a.id).unwrap().id(), before);
        // A valid discovered engine disappears between admission and actual spawn.
        let media = m
            .probe("https://fixture.invalid/slow-spawn-blocker")
            .await
            .unwrap();
        let a = m
            .enqueue(EnqueueInput {
                media: media.clone(),
                mode: "video".into(),
                quality: "1080".into(),
                format_id: None,
                output_dir: None,
            })
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        let b = m
            .enqueue(EnqueueInput {
                media,
                mode: "video".into(),
                quality: "1080".into(),
                format_id: None,
                output_dir: None,
            })
            .await
            .unwrap();
        let moved = engine.with_extension("held");
        std::fs::rename(engine, &moved).unwrap();
        terminal(&m, &a.id).await;
        let failed = terminal(&m, &b.id).await;
        std::fs::rename(&moved, engine).unwrap();
        assert_eq!(failed.status, TaskStatus::Failed);
        let blocker = root.join("blocked-logs");
        std::fs::write(&blocker, b"not a directory").unwrap();
        let restricted = Diagnostics::open(Some(blocker), "fixture", Policy::default(), None);
        let isolated = Manager::init_with_diagnostics(
            root.join("restricted-data"),
            None,
            None,
            restricted.clone(),
        )
        .await
        .unwrap();
        let mut settings = isolated.snapshot().await.settings;
        settings.engine_path = engine.to_string_lossy().into_owned();
        settings.download_dir = out.to_string_lossy().into_owned();
        isolated.update_settings(settings).await.unwrap();
        let media = isolated
            .probe("https://fixture.invalid/restricted-logger")
            .await
            .unwrap();
        let t = isolated
            .enqueue(EnqueueInput {
                media,
                mode: "video".into(),
                quality: "1080".into(),
                format_id: None,
                output_dir: None,
            })
            .await
            .unwrap();
        assert_eq!(
            terminal(&isolated, &t.id).await.status,
            TaskStatus::Completed
        );
        assert!(restricted.flush().is_err());
        isolated.shutdown().await.unwrap();
    }
    m.shutdown().await.unwrap();
    let query_fixture = if enabled {
        use std::io::Write;
        let query_root = root.join("query-fixture");
        std::fs::create_dir_all(query_root.join("sessions")).unwrap();
        let mut sample = diagnostics
            .read()
            .events
            .into_iter()
            .find(|e| e.phase == Stage::ParseTotal && e.status == Status::Ok)
            .unwrap();
        sample.session_id = uuid::Uuid::new_v4().to_string();
        sample.duration_ms = Some(0.5);
        for file in 0..5 {
            let name = format!(
                "session-{}-{}-{:06}.jsonl",
                chrono::Utc::now().timestamp_millis(),
                sample.session_id,
                file
            );
            let mut output = std::io::BufWriter::new(
                std::fs::File::create(query_root.join("sessions").join(name)).unwrap(),
            );
            for index in 0..6000 {
                sample.event_id = uuid::Uuid::new_v4().to_string();
                sample.trace_id = uuid::Uuid::new_v4().to_string();
                sample.span_id = uuid::Uuid::new_v4().to_string();
                sample.elapsed_since_session_start_ms = (file * 6000 + index) as f64;
                serde_json::to_writer(&mut output, &sample).unwrap();
                output.write_all(b"\n").unwrap();
            }
            output.flush().unwrap();
        }
        let reader = Diagnostics::open(Some(query_root), "fixture", Policy::default(), None);
        reader.flush().unwrap();
        let mut times = Vec::new();
        let mut packet = 0;
        for _ in 0..3 {
            let start = Instant::now();
            let page = reader
                .query(&Filter {
                    limit: 100,
                    ..Default::default()
                })
                .unwrap();
            times.push(start.elapsed().as_secs_f64() * 1000.);
            assert_eq!(page.events.len(), 100);
            assert!(page.truncated);
            assert_eq!(page.total, 20_000);
            packet = serde_json::to_vec(&page).unwrap().len();
        }
        Some(
            serde_json::json!({"synthetic":true,"generated_events":30_000,"analysis_limit":20_000,"page_size":100,"query_samples_ms":times,"ipc_packet_bytes":packet,"note":"synthetic stored events test I/O/pagination only, excluded from media stage statistics"}),
        )
    } else {
        None
    };
    diagnostics.flush().unwrap();
    let stats = diagnostics.overview(&Filter::default()).unwrap();
    let events = diagnostics.read().events;
    let serialized = serde_json::to_string(&events).unwrap();
    for secret in [
        "PRIVATE_TITLE_FIXTURE",
        "FIXTURE_SIG",
        "FIXTURE_COOKIE",
        "FIXTURE_TOKEN",
        "PrivatePerson",
    ] {
        assert!(!serialized.contains(secret));
    }
    fn distribution(mut values: Vec<f64>) -> serde_json::Value {
        values.sort_by(f64::total_cmp);
        let n = values.len();
        serde_json::json!({"samples":n,"p50_ms":values[(n as f64*0.5).ceil() as usize-1],"p95_ms":values[(n as f64*0.95).ceil() as usize-1],"max_ms":values[n-1]})
    }
    serde_json::json!({"file_writer_enabled":enabled,"independent_parse":distribution(parse_ms),"independent_enqueue":distribution(enqueue_ms),"statistics":normal_statistics,"all_scenario_statistics":stats.statistics,"events":events.len(),"query_fixture":query_fixture,"health":{"dropped":diagnostics.health().dropped,"critical_dropped":diagnostics.health().critical_dropped,"write_restricted":diagnostics.health().write_restricted},"scenarios":if enabled{"success;parse-fail;recheck-fail;no-progress;download-fail;postprocess;queued-cancel;running-cancel;retry;spawn-fail;serial-wait;restricted-logger;retry-attempt-ids"}else{"success-only"}})
}
#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let engine = std::fs::canonicalize(args.get(1).expect("fixture engine")).unwrap();
    let base = PathBuf::from(args.get(2).expect("artifacts output"));
    let root = base.join(format!("diagnostics-bench-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let root = std::fs::canonicalize(root).unwrap();
    let n = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(20);
    let mut results = Vec::new();
    for (name, enabled) in [("without-writer", false), ("with-writer", true)] {
        results.push(run(&root.join(name), &engine, enabled, n).await);
    }
    let value = serde_json::json!({"schema_version":1,"fixture":true,"app_version":env!("CARGO_PKG_VERSION"),"os":std::env::consts::OS,"architecture":std::env::consts::ARCH,"sample_count":n,"parse_fixture_delay_ms":60,"first_progress_fixture_delay_ms":100,"results":results});
    std::fs::write(
        root.join("result.json"),
        serde_json::to_vec_pretty(&value).unwrap(),
    )
    .unwrap();
    println!("{}", serde_json::to_string_pretty(&value).unwrap());
}
