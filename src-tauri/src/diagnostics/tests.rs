use super::*;
fn directory() -> PathBuf {
    let path =
        std::env::temp_dir().join(format!("fluxdl-diagnostics-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&path).unwrap();
    path
}
fn service(root: &Path, policy: Policy) -> Arc<Diagnostics> {
    Diagnostics::open(Some(root.to_path_buf()), "fixture", policy, None)
}
fn event(service: &Arc<Diagnostics>) -> Event {
    let t = service.trace(None, None, None);
    t.begin(Stage::ParseTotal);
    t.finish(Stage::ParseTotal, Status::Ok, None);
    service
        .read()
        .events
        .into_iter()
        .find(|e| e.phase == Stage::ParseTotal && e.status == Status::Ok)
        .unwrap()
}
#[test]
fn typed_schema_rejects_unknown_sensitive_fields_and_future_versions() {
    let s = Diagnostics::disabled();
    let e = event(&s);
    let mut json = serde_json::to_value(&e).unwrap();
    json["cookie"] = serde_json::json!("FIXTURE_SECRET");
    assert!(serde_json::from_value::<Event>(json).is_err());
    let mut e = e;
    e.schema_version = 2;
    assert!(!e.valid());
}
#[test]
fn spans_are_monotonic_deduplicated_and_missing_has_no_duration() {
    let s = Diagnostics::disabled();
    let t = s.trace(None, None, None);
    t.begin(Stage::DownloadFirstProgress);
    t.missing(Stage::DownloadFirstProgress, Status::Unobserved);
    t.finish(Stage::DownloadFirstProgress, Status::Ok, None);
    let values: Vec<_> = s
        .read()
        .events
        .into_iter()
        .filter(|e| e.trace_id == t.id())
        .collect();
    assert_eq!(values.len(), 2);
    assert!(values[1].duration_ms.is_none());
    assert_eq!(values[1].status, Status::Unobserved);
}
#[test]
fn concurrent_writer_lines_reopen_and_export_are_private_and_valid_zip() {
    let root = directory();
    let s = service(&root, Policy::default());
    std::thread::scope(|scope| {
        for _ in 0..8 {
            let s = s.clone();
            scope.spawn(move || {
                for _ in 0..30 {
                    let t = s.trace(None, None, None);
                    t.begin(Stage::ParseTotal);
                    t.finish(Stage::ParseTotal, Status::Ok, None);
                }
            });
        }
    });
    s.flush().unwrap();
    let read = writer::read(&root);
    assert_eq!(read.unreadable, 0);
    assert_eq!(
        read.events
            .iter()
            .filter(|e| e.phase == Stage::ParseTotal)
            .count(),
        480
    );
    let secret = r"FIXTURE_SECRET https://example.com/private?sig=FIXTURE_SIG Cookie: PRIVATE_COOKIE C:\Users\PrivatePerson\secret.mp4 socks5://proxy:PRIVATE_PASSWORD@host";
    let error = AppError::new(
        crate::errors::ErrorCode::ProcessExit,
        crate::errors::ErrorPhase::Download,
        secret,
    );
    let t = s.trace(Some(secret), Some(secret), Some(secret));
    t.begin(Stage::DownloadTotal);
    t.finish(Stage::DownloadTotal, Status::Failed, Some(&error));
    s.flush().unwrap();
    let destination = root.join("FluxDL-Diagnostics-test.zip");
    s.export(
        &destination,
        &Filter::default(),
        &[("yt-dlp".into(), secret.into())],
    )
    .unwrap();
    for path in std::fs::read_dir(root.join("sessions"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .chain([root.join("latest-report.md"), destination.clone()])
    {
        let bytes = std::fs::read(path).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        for value in [
            "FIXTURE_SECRET",
            "FIXTURE_SIG",
            "PRIVATE_COOKIE",
            "PrivatePerson",
            "PRIVATE_PASSWORD",
        ] {
            assert!(!text.contains(value));
        }
    }
    let python=std::process::Command::new("python").args(["-c","import sys,zipfile,json; z=zipfile.ZipFile(sys.argv[1]); assert z.testzip() is None; assert len(z.namelist())==6; [json.loads(x) for x in z.read('events.jsonl').splitlines()]; assert 'raw stderr' in json.loads(z.read('privacy-manifest.json'))['excluded']"]).arg(&destination).status().unwrap();
    assert!(python.success());
    assert!(s.export(&destination, &Filter::default(), &[]).is_err());
    let old_session = s.health().session_id;
    drop(s);
    let s = service(&root, Policy::default());
    s.flush().unwrap();
    assert_ne!(old_session, s.health().session_id);
    assert!(s.read().events.iter().any(|e| e.session_id == old_session));
    drop(s);
}
#[test]
fn rotation_retention_only_remove_known_files_and_respect_age() {
    let root = directory();
    std::fs::create_dir(root.join("sessions")).unwrap();
    let foreign = root.join("sessions/user-private.jsonl");
    std::fs::write(&foreign, b"keep").unwrap();
    let s = service(
        &root,
        Policy {
            max_bytes: 1800,
            max_files: 3,
            max_age: Duration::from_secs(7 * 86400),
        },
    );
    for _ in 0..20 {
        event(&s);
    }
    s.flush().unwrap();
    let paths: Vec<_> = std::fs::read_dir(root.join("sessions"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p != &foreign)
        .collect();
    assert!(paths.len() <= 3);
    for p in &paths {
        writer::expire(p);
    }
    drop(s);
    let s = service(
        &root,
        Policy {
            max_bytes: 1800,
            max_files: 3,
            max_age: Duration::ZERO,
        },
    );
    s.flush().unwrap();
    assert_eq!(std::fs::read(&foreign).unwrap(), b"keep");
    assert!(paths.iter().all(|p| !p.exists()));
}
#[test]
fn corrupted_truncated_future_duplicate_lines_do_not_escape_into_query() {
    let root = directory();
    let s = service(&root, Policy::default());
    let e = event(&s);
    s.flush().unwrap();
    let path = std::fs::read_dir(root.join("sessions"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new().append(true).open(path).unwrap();
    let line = serde_json::to_string(&e).unwrap();
    writeln!(f, "{line}").unwrap();
    writeln!(f, "{{invalid json}}").unwrap();
    let mut future = e.clone();
    future.schema_version = 99;
    writeln!(f, "{}", serde_json::to_string(&future).unwrap()).unwrap();
    write!(f, "{{\"schema_version\":").unwrap();
    drop(f);
    let read = s.read();
    assert_eq!(
        read.events
            .iter()
            .filter(|v| v.event_id == e.event_id)
            .count(),
        1
    );
    assert_eq!(read.unreadable, 3);
}
#[test]
fn unavailable_root_degrades_without_waiting_or_panicking() {
    let root = directory();
    let blocker = root.join("blocked");
    std::fs::write(&blocker, b"not-directory").unwrap();
    let s = service(&blocker, Policy::default());
    let start = Instant::now();
    for _ in 0..3000 {
        let t = s.trace(None, None, None);
        t.begin(Stage::ParseTotal);
        t.finish(Stage::ParseTotal, Status::Ok, None);
    }
    assert!(start.elapsed() < Duration::from_secs(3));
    assert!(s.flush().is_err());
    assert!(s.health().write_restricted);
    assert!(s.read().events.len() <= MEMORY_LIMIT);
    assert!(s.directory().is_err());
}
#[test]
fn filters_are_bounded_and_uuid_not_paths() {
    let s = Diagnostics::disabled();
    event(&s);
    assert!(s
        .query(&Filter {
            task_id: Some("../private".into()),
            ..Default::default()
        })
        .is_err());
    assert!(s
        .query(&Filter {
            limit: 201,
            ..Default::default()
        })
        .is_err());
    let page = s
        .query(&Filter {
            limit: 1,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(page.events.len(), 1);
    assert!(page.next_cursor.is_some());
}
#[test]
fn percentile_uses_only_successful_samples_and_nearest_rank() {
    let s = Diagnostics::disabled();
    let base = event(&s);
    let mut events = Vec::new();
    for n in 1..=20 {
        let mut e = base.clone();
        e.duration_ms = Some(n as f64);
        e.event_id = uuid::Uuid::new_v4().to_string();
        events.push(e);
    }
    let mut bad = base;
    bad.status = Status::Failed;
    bad.duration_ms = Some(9999.);
    events.push(bad);
    let stats = report::overview(events, "none", 0, false).statistics;
    assert_eq!(stats[0].samples, 20);
    assert_eq!(stats[0].p50_ms, 10.);
    assert_eq!(stats[0].p95_ms, 19.);
    assert_eq!(stats[0].max_ms, 20.);
}

#[test]
fn reserved_error_queue_replays_from_memory_without_blocking_producers() {
    let root = directory();
    let s = service(&root, Policy::default());
    s.flush().unwrap();
    s.shared.paused.store(true, Ordering::Relaxed);
    std::thread::sleep(Duration::from_millis(30));
    let start = Instant::now();
    for _ in 0..600 {
        let t = s.trace(None, None, None);
        t.begin(Stage::DownloadTotal);
        let error = AppError::new(
            crate::errors::ErrorCode::ProcessExit,
            crate::errors::ErrorPhase::Download,
            "fixture secret never stored",
        );
        t.finish(Stage::DownloadTotal, Status::Failed, Some(&error));
    }
    assert!(start.elapsed() < Duration::from_secs(2));
    assert!(s.health().critical_dropped > 0);
    s.shared.paused.store(false, Ordering::Relaxed);
    s.flush().unwrap();
    assert_eq!(
        writer::read(&root)
            .events
            .iter()
            .filter(|e| e.level == Level::Error)
            .count(),
        600
    );
}
#[test]
fn storage_full_and_partial_writes_are_explicit_errors_not_panics() {
    struct Full;
    impl std::io::Write for Full {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::from(std::io::ErrorKind::StorageFull))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    assert_eq!(
        writer::write_record(&mut Full, b"fixture\n")
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::StorageFull
    );
    struct Partial(Vec<u8>);
    impl std::io::Write for Partial {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.push(bytes[0]);
            Ok(1)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut partial = Partial(vec![]);
    writer::write_record(&mut partial, b"fixture\n").unwrap();
    assert_eq!(partial.0, b"fixture\n");
}
#[cfg(windows)]
#[test]
fn windows_locked_report_degrades_then_recovers_without_changing_private_files() {
    use std::os::windows::fs::OpenOptionsExt;
    let root = directory();
    let s = service(&root, Policy::default());
    event(&s);
    s.flush().unwrap();
    let held = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(root.join("latest-report.md"))
        .unwrap();
    event(&s);
    assert!(s.flush().is_err());
    assert!(s.health().write_restricted);
    assert!(!s.read().events.is_empty());
    drop(held);
    s.flush().unwrap();
}
#[cfg(windows)]
#[test]
fn windows_readonly_report_is_nonfatal_and_unknown_files_are_retained() {
    let root = directory();
    let s = service(&root, Policy::default());
    s.flush().unwrap();
    let report = root.join("latest-report.md");
    let original = std::fs::metadata(&report).unwrap().permissions();
    let mut p = original.clone();
    p.set_readonly(true);
    std::fs::set_permissions(&report, p).unwrap();
    let result = s.flush();
    std::fs::set_permissions(&report, original).unwrap();
    assert!(result.is_err());
    event(&s);
    s.flush().unwrap();
}
#[test]
fn frontend_association_rejects_reused_ids_and_deduplicates_elapsed_records() {
    let s = Diagnostics::disabled();
    let id = uuid::Uuid::new_v4().to_string();
    let t = s.request_trace(Some(&id), None).unwrap();
    t.begin(Stage::ParseTotal);
    t.finish(Stage::ParseTotal, Status::Ok, None);
    assert!(s.request_trace(Some(&id), None).is_err());
    assert!(s.request_trace(Some("../private"), None).is_err());
    s.record_frontend(&id, Stage::ParseUiRequest, 20., true)
        .unwrap();
    s.record_frontend(&id, Stage::ParseUiRequest, 999., true)
        .unwrap();
    let events = s.read().events;
    assert_eq!(
        events
            .iter()
            .filter(|e| e.phase == Stage::ParseUiRequest)
            .count(),
        1
    );
    assert!(s
        .record_frontend(&id, Stage::DownloadActive, 1., true)
        .is_err());
    assert!(s
        .record_frontend(&id, Stage::EnqueueUiRequest, 1., true)
        .is_err());
}
#[test]
fn independent_frontend_clock_cannot_change_rust_durations_or_span_completion() {
    let s = Diagnostics::disabled();
    let t = s.trace(None, None, None);
    t.begin(Stage::ParseTotal);
    std::thread::sleep(Duration::from_millis(3));
    t.finish(Stage::ParseTotal, Status::Ok, None);
    t.frontend(Stage::ParseUiRequest, 500_000., true);
    let events = s.read().events;
    let backend = events
        .iter()
        .find(|e| e.phase == Stage::ParseTotal && e.status == Status::Ok)
        .unwrap();
    assert!(backend.duration_ms.unwrap() < 1000.);
    assert!(backend.duration_ms.unwrap() >= 3.);
    let mut shifted = backend.clone();
    shifted.timestamp_utc = "2001-01-01T00:00:00Z".into();
    assert_eq!(shifted.duration_ms, backend.duration_ms);
}
#[test]
fn ordinary_engine_output_and_native_errors_hide_urls_paths_and_titles() {
    let private = r"ERROR: PRIVATE_TITLE_FIXTURE https://example.test/PRIVATE_PATH?sig=FIXTURE_SIG C:\Users\PrivatePerson\PrivateName.mp4 Cookie: FIXTURE_COOKIE";
    let safe = engine_output_summary(private);
    let friendly = crate::engine::friendly_error(private);
    let native = redact_log(r"failed at C:\Users\PrivatePerson\PrivateName.mp4");
    for output in [safe, &friendly, &native] {
        for secret in [
            "PRIVATE_TITLE_FIXTURE",
            "PRIVATE_PATH",
            "FIXTURE_SIG",
            "PrivatePerson",
            "PrivateName.mp4",
            "FIXTURE_COOKIE",
        ] {
            assert!(!output.contains(secret), "{output}");
        }
    }
}
#[cfg(windows)]
#[test]
fn junction_logs_root_never_reads_writes_or_cleans_the_target() {
    let owner = directory();
    let target = directory();
    let private = target.join("private.txt");
    std::fs::write(&private, b"PRIVATE_TARGET_KEEP").unwrap();
    let link = owner.join("linked-logs");
    let status = std::process::Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-Command",
            "New-Item -ItemType Junction -Path $env:FLUXDL_FIXTURE_LINK -Target $env:FLUXDL_FIXTURE_TARGET | Out-Null",
        ])
        .env("FLUXDL_FIXTURE_LINK", &link)
        .env("FLUXDL_FIXTURE_TARGET", &target)
        .status()
        .unwrap();
    assert!(status.success());
    let s = service(&link, Policy::default());
    event(&s);
    assert!(s.flush().is_err());
    assert!(s.directory().is_err());
    assert_eq!(std::fs::read(&private).unwrap(), b"PRIVATE_TARGET_KEEP");
    assert!(!target.join("sessions").exists());
    // A junction is removed as a directory entry only, never recursively.
    drop(s);
    std::fs::remove_dir(&link).unwrap();
}
#[test]
fn slow_traces_rank_total_spans_without_adding_nested_stages_or_frontend_clocks() {
    let s = Diagnostics::disabled();
    let first = event(&s);
    let mut second = first.clone();
    second.trace_id = uuid::Uuid::new_v4().to_string();
    second.duration_ms = Some(20.);
    let mut first = first;
    first.duration_ms = Some(10.);
    let mut nested = first.clone();
    nested.phase = Stage::ParseMetadataExtract;
    nested.duration_ms = Some(100.);
    let mut ui = first.clone();
    ui.source = Source::Frontend;
    ui.duration_ms = Some(500_000.);
    let stats = report::overview(vec![first, second.clone(), nested, ui], "fixture", 0, false);
    assert_eq!(stats.slow_traces.len(), 2);
    assert_eq!(stats.slow_traces[0].trace_id, second.trace_id);
    assert_eq!(stats.slow_traces[0].duration_ms, Some(20.));
}
#[test]
fn cached_queries_invalidate_on_memory_disk_changes_and_export_always_redecodes() {
    use std::io::Write;
    let root = directory();
    let s = service(&root, Policy::default());
    event(&s);
    s.flush().unwrap();
    let first = s.read_snapshot();
    let warm = s.read_snapshot();
    assert!(Arc::ptr_eq(&first, &warm));
    s.cache.lock().unwrap().as_mut().unwrap().created = Instant::now() - Duration::from_secs(6);
    let expired = s.read_snapshot();
    assert!(!Arc::ptr_eq(&warm, &expired));
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..8).map(|_| scope.spawn(|| s.read_snapshot())).collect();
        for handle in handles {
            assert!(Arc::ptr_eq(&expired, &handle.join().unwrap()));
        }
    });
    let gate = s.cache.lock().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let producer = s.clone();
    let thread = std::thread::spawn(move || {
        let t = producer.trace(None, None, None);
        t.begin(Stage::ParseTotal);
        t.finish(Stage::ParseTotal, Status::Ok, None);
        tx.send(()).unwrap();
    });
    let delivered = rx.recv_timeout(Duration::from_secs(1));
    drop(gate);
    thread.join().unwrap();
    assert!(
        delivered.is_ok(),
        "a diagnostic reader cannot block the event producer"
    );
    s.shared.paused.store(true, Ordering::Relaxed);
    std::thread::sleep(Duration::from_millis(25));
    let t = s.trace(None, None, None);
    t.begin(Stage::DownloadTotal);
    t.finish(Stage::DownloadTotal, Status::Ok, None);
    let changed = s.read_snapshot();
    assert!(!Arc::ptr_eq(&first, &changed));
    assert!(changed.events.iter().any(|e| e.trace_id == t.id()));
    let path = std::fs::read_dir(root.join("sessions"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    writeln!(f, "{{\"unknown\":\"PRIVATE_CACHE_SECRET\"}}").unwrap();
    drop(f);
    assert!(s.read_snapshot().unreadable > 0);
    s.shared.paused.store(false, Ordering::Relaxed);
    s.flush().unwrap();
    let before = s.read_snapshot();
    let destination = root.join("FluxDL-Diagnostics-cache.zip");
    s.export(&destination, &Filter::default(), &[]).unwrap();
    let after = s.read_snapshot();
    assert!(!Arc::ptr_eq(&before, &after));
    assert!(
        !String::from_utf8_lossy(&std::fs::read(destination).unwrap())
            .contains("PRIVATE_CACHE_SECRET")
    );
}
#[test]
fn report_and_export_share_numeric_engine_versions_without_private_suffixes() {
    let root = directory();
    let s = service(&root, Policy::default());
    let mut engines = crate::models::EngineDiagnostics::default();
    engines.ytdlp.version = Some("2026.07.04 PRIVATE_VERSION_SECRET".into());
    engines.ffmpeg.version = Some("ffmpeg version 8.0.1-full_build PRIVATE_VERSION_SECRET".into());
    s.set_engine_versions(&engines);
    s.flush().unwrap();
    let text = std::fs::read_to_string(root.join("latest-report.md")).unwrap();
    assert!(text.contains("2026.07.04") && text.contains("8.0.1"));
    assert!(!text.contains("PRIVATE_VERSION_SECRET"));
    let destination = root.join("FluxDL-Diagnostics-versions.zip");
    s.export(
        &destination,
        &Filter::default(),
        &[("ffmpeg".into(), engines.ffmpeg.version.unwrap())],
    )
    .unwrap();
    assert!(
        !String::from_utf8_lossy(&std::fs::read(&destination).unwrap())
            .contains("PRIVATE_VERSION_SECRET")
    );
    assert_eq!(
        export::safe_versions(&[("yt-dlp".into(), "Cookie: PRIVATE_VERSION_SECRET".into())]).len(),
        0
    );
}
#[test]
fn rotation_counts_the_current_file_even_after_the_wall_clock_moves_backwards() {
    let root = directory();
    let sessions = root.join("sessions");
    std::fs::create_dir(&sessions).unwrap();
    let id = uuid::Uuid::new_v4();
    for index in 0..5 {
        std::fs::write(
            sessions.join(format!("session-9999999999999-{id}-{index:06}.jsonl")),
            b"fixture\n",
        )
        .unwrap();
    }
    let s = service(&root, Policy::default());
    s.flush().unwrap();
    assert_eq!(std::fs::read_dir(sessions).unwrap().count(), 5);
}

#[test]
fn throughput_optional_schema_rejects_private_objects_and_invalid_numbers() {
    let s = Diagnostics::disabled();
    let t = s.trace(None, None, None);
    t.download_progress(1000, false, false, None);
    t.stop_throughput();
    t.finish_throughput(Status::Cancelled);
    let event = s
        .read()
        .events
        .into_iter()
        .find(|e| e.fields.throughput.is_some())
        .unwrap();
    assert!(event.valid());
    let mut json = serde_json::to_value(&event).unwrap();
    json["fields"]["throughput"]["proxy"] = serde_json::json!("http://user:PRIVATE@127.0.0.1:1234");
    assert!(serde_json::from_value::<Event>(json).is_err());
    let mut invalid = event.clone();
    invalid.fields.throughput.as_mut().unwrap().mean_bps = Some(f64::INFINITY);
    assert!(!invalid.valid());
    invalid = event.clone();
    invalid.fields.throughput.as_mut().unwrap().duration_ms = -1.0;
    assert!(!invalid.valid());
    invalid = event;
    invalid.phase = Stage::ParseTotal;
    assert!(!invalid.valid());
    let mut legacy = serde_json::to_value(super::tests::event(&s)).unwrap();
    legacy["fields"]
        .as_object_mut()
        .unwrap()
        .remove("throughput");
    assert!(serde_json::from_value::<Event>(legacy).unwrap().valid());
}

#[test]
fn throughput_terminal_is_once_throttled_correlated_and_reported_without_stage_duration() {
    let s = Diagnostics::disabled();
    let task = uuid::Uuid::new_v4().to_string();
    let trace = s.trace(None, None, Some(&task));
    for bytes in 0..100 {
        trace.download_progress(bytes, false, false, None);
    }
    trace.stop_throughput();
    trace.finish_throughput(Status::Failed);
    trace.finish_throughput(Status::Ok);
    trace.download_progress(1_000_000, true, false, None);
    trace.tick_throughput();
    let events = s.read().events;
    let samples: Vec<_> = events
        .iter()
        .filter(|e| e.fields.throughput.is_some())
        .collect();
    assert_eq!(samples.len(), 2);
    assert_eq!(samples[1].status, Status::Failed);
    assert!(samples[1].important());
    assert!(samples[1].duration_ms.is_none());
    assert_eq!(samples[1].task_id.as_deref(), Some(task.as_str()));
    assert_eq!(samples[0].fields.attempt_id, samples[1].fields.attempt_id);
    let value = samples[1].fields.throughput.as_ref().unwrap();
    assert_eq!(value.observed_bytes, 99);
    assert!(value.final_sample);
    assert!(value.window_5s_bps.is_none());
    assert!(report::overview(events.clone(), &s.session, 0, false)
        .statistics
        .is_empty());
    let report = report::markdown(&events, "fixture", &s.session, 0, false, 0, 0);
    assert!(report.contains("下载吞吐诊断") && report.contains("99字节"));
    assert!(report.contains("进度缺失") && report.contains("KiB/s"));
}

#[test]
fn stage_view_keeps_stages_and_latest_throughput_per_attempt_before_pagination() {
    let service = Diagnostics::disabled();
    let task = uuid::Uuid::new_v4().to_string();
    let original = service.trace(None, None, Some(&task));
    // A same-Trace retry fixture proves attempts are not collapsed together.
    let retry = service.trace(Some(original.id()), None, Some(&task));
    assert_ne!(original.attempt, retry.attempt);
    original.begin(Stage::QueueWait);
    original.finish(Stage::QueueWait, Status::Ok, None);
    original.fault(
        Stage::PersistenceIssue,
        &crate::errors::AppError::new(
            crate::errors::ErrorCode::PersistenceRejected,
            crate::errors::ErrorPhase::Persist,
            "fixture source error never retained",
        ),
    );
    let mut latest_ids = vec![];
    for trace in [&original, &retry] {
        let mut counters = throughput::Throughput::default();
        for n in 0..320 {
            let sample = counters
                .observe(Duration::from_secs(n * 5), n * 1024, false, false)
                .unwrap();
            trace.record_throughput(sample, Status::Ok);
        }
        latest_ids.push(service.read().events.last().unwrap().event_id.clone());
    }
    let stage_filter = Filter {
        task_id: Some(task.clone()),
        stage_view: true,
        limit: 100,
        ..Default::default()
    };
    let stage_page = service.query(&stage_filter).unwrap();
    assert_eq!(stage_page.total, 5);
    assert!(stage_page.next_cursor.is_none());
    assert_eq!(
        stage_page
            .events
            .iter()
            .filter(|e| e.phase == Stage::QueueWait)
            .count(),
        2
    );
    assert!(stage_page
        .events
        .iter()
        .any(|e| e.event == EventName::FaultObserved && e.phase == Stage::PersistenceIssue));
    let samples: Vec<_> = stage_page
        .events
        .iter()
        .filter(|e| e.event == EventName::ThroughputSample)
        .collect();
    assert_eq!(samples.len(), 2);
    assert!(samples.iter().all(|e| latest_ids.contains(&e.event_id)));
    assert!(samples
        .iter()
        .all(|e| e.fields.throughput.as_ref().unwrap().samples == 320));
    let first = service
        .query(&Filter {
            limit: 2,
            ..stage_filter.clone()
        })
        .unwrap();
    let next = service
        .query(&Filter {
            limit: 2,
            cursor: first.next_cursor.unwrap(),
            ..stage_filter.clone()
        })
        .unwrap();
    assert!(first
        .events
        .iter()
        .all(|a| next.events.iter().all(|b| a.event_id != b.event_id)));
    assert_eq!(next.total, 5);

    // Normal events, overview and export keep all 640 actual typed fixture samples.
    let raw_page = service
        .query(&Filter {
            stage_view: false,
            ..stage_filter.clone()
        })
        .unwrap();
    assert_eq!(raw_page.total, 643);
    assert_eq!(raw_page.events.len(), 100);
    assert!(raw_page
        .events
        .iter()
        .all(|e| e.event == EventName::ThroughputSample));
    assert_eq!(service.overview(&stage_filter).unwrap().total, 643);
    let root = directory();
    let destination = root.join("stage-view-export.zip");
    service.export(&destination, &stage_filter, &[]).unwrap();
    let zip = std::fs::read(destination).unwrap();
    assert_eq!(
        String::from_utf8_lossy(&zip)
            .matches("\"event\":\"throughput_sample\"")
            .count(),
        640
    );
}

#[test]
fn stage_view_is_optional_camel_case_and_requires_task_or_trace_scope() {
    let default: Filter = serde_json::from_str("{}").unwrap();
    assert!(!default.stage_view);
    let scoped = Filter {
        stage_view: true,
        trace_id: Some(uuid::Uuid::new_v4().to_string()),
        ..Default::default()
    };
    let json = serde_json::to_value(&scoped).unwrap();
    assert_eq!(json["stageView"], true);
    assert!(serde_json::from_value::<Filter>(json)
        .unwrap()
        .validate()
        .is_ok());
    assert!(Filter {
        stage_view: true,
        ..Default::default()
    }
    .validate()
    .is_err());
    assert!(Filter {
        stage_view: true,
        task_id: Some("private/path".into()),
        ..Default::default()
    }
    .validate()
    .is_err());
    assert!(serde_json::from_str::<Filter>(r#"{"stageView":"true"}"#).is_err());
}
