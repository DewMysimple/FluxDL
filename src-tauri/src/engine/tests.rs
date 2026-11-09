use super::*;
use crate::models::{DownloadTask, EngineInfo};
use discovery::runtime_supported;
#[test]
fn bounded_object_progress_keeps_unknown_values_and_literal_na_filenames_valid() {
    let (task, diagnostics) = task_and_diagnostics();
    let args = build_download_args(&task, &Settings::default(), &diagnostics).unwrap();
    let template = &args[args
        .iter()
        .position(|arg| arg == "--progress-template")
        .unwrap()
        + 1];
    assert_eq!(
        template.matches(")j").count(),
        1,
        "JSON encoding must apply to the selected object rather than nullable scalars"
    );
    assert!(template.contains("progress.filename,info.format_id"));
    // Captured shape from yt-dlp 2026.07.04's real dict formatter: missing
    // optional fields are omitted, not inserted as unquoted NA tokens.
    let line = r#"__FLUX_PROGRESS__{"progress.status":"downloading","progress.filename":"C:/NA folder/中文.f137.mp4","info.format_id":"137","progress.downloaded_bytes":1024,"progress.total_bytes":4096}"#;
    let fact = parse_progress_fact(line).unwrap();
    assert_eq!(
        fact.stream_id.as_deref(),
        Some("137:C:/NA folder/中文.f137.mp4")
    );
    assert!(matches!(
        fact.event,
        ProgressEvent::Download {
            downloaded_bytes: 1024,
            total_bytes: Some(4096),
            progress: Some(25.0),
            speed: None,
            eta: None
        }
    ));
    let unknown = r#"__FLUX_PROGRESS__{"progress.status":"downloading","progress.filename":"NA literal.mp4","progress.downloaded_bytes":3,"progress.speed":null,"progress.eta":null}"#;
    assert!(matches!(
        parse_progress(unknown),
        Some(ProgressEvent::Download {
            downloaded_bytes: 3,
            total_bytes: None,
            progress: None,
            speed: None,
            eta: None
        })
    ));
    let finished = r#"__FLUX_PROGRESS__{"progress.status":"finished","progress.filename":"a.f140.m4a","info.format_id":"140","progress.downloaded_bytes":10,"progress.total_bytes":10}"#;
    let audio = parse_progress_fact(finished).unwrap();
    assert!(audio.finished);
    assert_ne!(audio.stream_id, fact.stream_id);
    assert!(parse_progress(r#"__FLUX_PROGRESS__{"progress.eta":NA}"#).is_none());
    for object in [
        "{}",
        r#"{"unrelated":true}"#,
        r#"{"progress.eta":null,"progress.speed":null}"#,
    ] {
        assert!(parse_progress(&format!("__FLUX_PROGRESS__{object}")).is_none());
    }
    assert!(matches!(
        parse_progress(
            r#"__FLUX_PROGRESS__{"progress.status":"downloading","progress.downloaded_bytes":null}"#
        ),
        Some(ProgressEvent::Download {
            downloaded_bytes: 0,
            total_bytes: None,
            progress: None,
            ..
        })
    ));
}
#[test]
fn nonobject_protocol_records_and_metadata_are_rejected() {
    for json in ["null", "[]", "42", "\"text\""] {
        assert!(parse_progress(&format!("__FLUX_PROGRESS__{json}")).is_none());
        assert_eq!(
            media_from_json(
                &serde_json::from_str::<Value>(json).unwrap(),
                "https://example.com/",
                ""
            )
            .unwrap_err()
            .code,
            ErrorCode::EngineProtocol
        );
    }
    let fact=parse_progress_fact(r#"__FLUX_PROGRESS__{"filename":"video.f137.mp4","format_id":"137","status":"finished","downloaded_bytes":20,"total_bytes":20}"#).unwrap();
    assert_eq!(fact.stream_id.as_deref(), Some("137:video.f137.mp4"));
    assert!(fact.finished);
}
use serde_json::json;
#[test]
fn validates_urls_without_shell_interpretation() {
    for url in [
        "",
        "file:///C:/secret",
        "--exec calc",
        "javascript:alert(1)",
        "https://u:p@example.com/video",
        "https://example.com/\n--exec",
    ] {
        assert!(validate_url(url).is_err(), "{url}");
    }
    assert!(validate_url("https://example.com/watch?v=x&title=$(calc);test").is_ok());
}
#[test]
fn parses_real_progress_json_and_unknown_total() {
    assert_eq!(parse_progress("\u{1b}[32m__FLUX_PROGRESS__{\"downloaded_bytes\":25,\"total_bytes\":100,\"speed\":50.5,\"eta\":2}\u{1b}[0m"), Some(ProgressEvent::Download { downloaded_bytes:25,total_bytes:Some(100),speed:Some(50.5),eta:Some(2.0),progress:Some(25.0) }));
    assert_eq!(
        parse_progress(
            "__FLUX_PROGRESS__{\"downloaded_bytes\":2048,\"total_bytes\":null,\"speed\":\"NA\"}"
        ),
        Some(ProgressEvent::Download {
            downloaded_bytes: 2048,
            total_bytes: None,
            speed: None,
            eta: None,
            progress: None
        })
    );
    assert!(parse_progress("__FLUX_PROGRESS__garbage").is_none());
    assert!(parse_progress("[download] 99%").is_none());
    assert_eq!(
        parse_progress("__FLUX_POST__processing"),
        Some(ProgressEvent::Postprocessing)
    );
    assert_eq!(
        parse_progress(r#"__FLUX_FILE__"C:\\Users\\测试\\video.mp4""#),
        Some(ProgressEvent::File("C:\\Users\\测试\\video.mp4".into()))
    );
}
#[test]
fn malformed_protocol_records_are_ignored_without_inventing_progress() {
    for line in [
        "__FLUX_PROGRESS__{",
        "__FLUX_PROGRESS__{\"downloaded_bytes\":NaN}",
        "__FLUX_PROGRESS__{\"speed\":Infinity}",
        "__FLUX_PROGRESS__{\"downloaded_bytes\":1,}",
        "__FLUX_FILE__null",
        "__FLUX_FILE__42",
        "__FLUX_FILE__\"\"",
        "__FLUX_FILE__\"unsafe\\npath.mp4\"",
    ] {
        assert!(parse_progress(line).is_none(), "accepted {line:?}");
    }
}
#[test]
fn unknown_progress_values_stay_indeterminate_and_future_fields_are_tolerated() {
    let line = r#"__FLUX_PROGRESS__{"downloaded_bytes":"4096","total_bytes":0,"speed":"NaN","eta":-1,"future_field":{"anything":true}}"#;
    assert_eq!(
        parse_progress(line),
        Some(ProgressEvent::Download {
            downloaded_bytes: 4096,
            total_bytes: None,
            speed: None,
            eta: None,
            progress: None,
        })
    );
    let estimate = r#"__FLUX_PROGRESS__{"downloaded_bytes":1024,"total_bytes":null,"total_bytes_estimate":2048,"speed":"Infinity","eta":"NA"}"#;
    assert_eq!(
        parse_progress(estimate),
        Some(ProgressEvent::Download {
            downloaded_bytes: 1024,
            total_bytes: Some(2048),
            speed: None,
            eta: None,
            progress: Some(50.0),
        })
    );
}
#[test]
fn playlist_is_explicitly_returned_without_batch_formats() {
    let media = media_from_json(
        &json!({"_type":"playlist","title":"List","entries":[]}),
        "https://example.com/list",
        "",
    )
    .unwrap();
    assert!(media.is_playlist);
    assert!(media.warnings.iter().any(|w| w.contains("整列表")));
}
#[test]
fn metadata_filters_storyboards_and_drm() {
    let media = media_from_json(&json!({"id":"x","title":"测试","formats":[{"format_id":"sb","ext":"mhtml"},{"format_id":"drm","ext":"mp4","has_drm":true},{"format_id":"18","ext":"mp4","height":360,"acodec":"aac","vcodec":"h264"}]}),"https://example.com/video", "").unwrap();
    assert_eq!(media.formats.len(), 1);
    assert!(media.formats[0].has_video && media.formats[0].has_audio);
}
#[test]
fn direct_media_preserves_unknown_metadata_and_rejects_non_media_results() {
    let media = media_from_json(
        &json!({
            "ext": "mp4",
            "duration": -1,
            "thumbnail": "javascript:alert(1)",
            "future_field": {"unrecognized": true}
        }),
        "https://example.com/direct.mp4",
        "",
    )
    .unwrap();
    assert_eq!(media.title, "未命名媒体");
    assert!(media.duration.is_none());
    assert!(media.thumbnail.is_none());
    assert_eq!(media.formats.len(), 1);
    assert!(media.formats[0].height.is_none());
    for raw in [
        json!(null),
        json!([]),
        json!({"formats": []}),
        json!({"formats": [{"ext":"mp4","vcodec":"none","acodec":"none"}]}),
    ] {
        assert!(media_from_json(&raw, "https://example.com/media", "").is_err());
    }
}
#[test]
fn generic_urls_cannot_turn_documents_or_programs_into_media() {
    for ext in [
        "exe", "bat", "cmd", "ps1", "js", "html", "url", "lnk", "pdf", "dll", "mhtml",
    ] {
        assert!(!is_media_extension(ext));
        assert!(media_from_json(
            &json!({"id":"x","title":"not media","ext":ext,"format_id":"generic"}),
            "https://example.com/download",
            ""
        )
        .is_err());
    }
    assert!(is_media_extension("MP4"));
    assert!(is_media_extension("m4v"));
}
#[test]
fn redacts_private_url_queries() {
    let line = redact_log("ERROR https://user:pass@example.com/video?token=secret#password failed");
    assert!(!line.contains("secret"));
    assert!(!line.contains("pass"));
    assert!(line.contains("example.com/video"));
}
#[test]
fn metadata_warnings_and_friendly_errors_share_http_credential_redaction() {
    let diagnostic = "WARNING: retry https://first-user:first-pass@example.com/video?access_token=first-secret#fragment-secret\nERROR: unable to download http://second-user:second-pass@cdn.example.com/file?signature=second-secret";
    let media = media_from_json(
        &json!({"ext":"mp4","format_id":"direct"}),
        "https://example.com/media",
        diagnostic,
    )
    .unwrap();
    let public = format!(
        "{}\n{}",
        media.warnings.join("\n"),
        friendly_error(diagnostic)
    );
    for secret in [
        "first-user",
        "first-pass",
        "first-secret",
        "fragment-secret",
        "second-user",
        "second-pass",
        "second-secret",
    ] {
        assert!(!public.contains(secret), "leaked {secret}");
    }
    assert!(public.contains("example.com/video"));
    assert!(public.contains("cdn.example.com/file"));
    assert!(public.contains("网络"));
}
#[test]
fn runtime_minimum_versions() {
    assert!(runtime_supported("deno", "deno 2.3.0"));
    assert!(!runtime_supported("deno", "deno 2.2.9"));
    assert!(runtime_supported("node", "v24.16.0"));
    assert!(!runtime_supported("node", "v20.1.0"));
}
fn task_and_diagnostics() -> (DownloadTask, EngineDiagnostics) {
    let media = media_from_json(&json!({"id":"x","title":"测试","formats":[{"format_id":"18","ext":"mp4","height":360,"acodec":"aac","vcodec":"h264"},{"format_id":"137","ext":"mp4","height":1080,"acodec":"none","vcodec":"h264"}]}),"https://example.com/watch?v=x&cmd=$(calc)", "").unwrap();
    let task = DownloadTask {
        id: "test-123".into(),
        media,
        mode: "video".into(),
        quality: "1080".into(),
        format_id: None,
        output_dir: std::env::temp_dir().to_string_lossy().into(),
        status: crate::models::TaskStatus::Queued,
        progress: None,
        speed: None,
        eta: None,
        downloaded_bytes: 0,
        total_bytes: None,
        final_path: None,
        error: None,
        logs: Vec::new(),
        created_at: String::new(),
        updated_at: String::new(),
        file_exists: false,
    };
    let diagnostics = EngineDiagnostics {
        ytdlp: EngineInfo {
            available: true,
            ..Default::default()
        },
        ..Default::default()
    };
    (task, diagnostics)
}
#[test]
fn download_arguments_are_safe_and_explicit() {
    let (mut task, diagnostics) = task_and_diagnostics();
    let args = build_download_args(&task, &Settings::default(), &diagnostics).unwrap();
    assert_eq!(args[args.len() - 2], "--");
    assert_eq!(args.last(), Some(&task.media.url));
    assert!(args.contains(&"--ignore-config".into()));
    assert!(args.contains(&"--no-playlist".into()));
    assert!(args
        .iter()
        .any(|a| a == "b[height<=?1080][vcodec!=none][acodec!=none]"));
    task.format_id = Some("137".into());
    assert!(
        build_download_args(&task, &Settings::default(), &diagnostics)
            .unwrap_err()
            .to_string()
            .contains("FFmpeg")
    );
    task.format_id = Some("18 --exec calc".into());
    assert!(build_download_args(&task, &Settings::default(), &diagnostics).is_err());
    task.format_id = Some("nonexistent".into());
    assert!(build_download_args(&task, &Settings::default(), &diagnostics).is_err());
}
#[test]
fn explicit_format_and_paths_remain_individual_arguments() {
    let (mut task, mut diagnostics) = task_and_diagnostics();
    task.format_id = Some("137".into());
    task.output_dir = std::env::temp_dir()
        .join("FluxDL 中文 & $(literal)")
        .to_string_lossy()
        .into_owned();
    diagnostics.ffmpeg.available = true;
    diagnostics.ffprobe.available = true;
    let engine_path = std::env::temp_dir()
        .join("FFmpeg tools & media")
        .join("ffmpeg.exe");
    diagnostics.ffmpeg.path = Some(engine_path.to_string_lossy().into_owned());
    let args = build_download_args(&task, &Settings::default(), &diagnostics).unwrap();
    let value_after = |flag: &str| &args[args.iter().position(|arg| arg == flag).unwrap() + 1];
    assert_eq!(value_after("--paths"), &task.output_dir);
    assert_eq!(
        value_after("--ffmpeg-location"),
        diagnostics.ffmpeg.path.as_ref().unwrap()
    );
    assert_eq!(value_after("--format"), "137+bestaudio");
    assert_eq!(args[args.len() - 2], "--");
    assert_eq!(args.last(), Some(&task.media.url));
    task.format_id = Some("18".into());
    let muxed = build_download_args(&task, &Settings::default(), &diagnostics).unwrap();
    assert_eq!(
        muxed[muxed.iter().position(|arg| arg == "--format").unwrap() + 1],
        "18"
    );
}
#[test]
fn mp3_requires_both_ffmpeg_and_ffprobe() {
    let (mut task, mut diagnostics) = task_and_diagnostics();
    task.mode = "audio".into();
    diagnostics.ffmpeg.available = true;
    assert!(build_download_args(&task, &Settings::default(), &diagnostics).is_err());
    diagnostics.ffprobe.available = true;
    let args = build_download_args(&task, &Settings::default(), &diagnostics).unwrap();
    assert!(args.contains(&"--extract-audio".into()));
    assert!(args.contains(&"mp3".into()));
    task.media.is_playlist = true;
    assert!(
        build_download_args(&task, &Settings::default(), &diagnostics)
            .unwrap_err()
            .to_string()
            .contains("整列表")
    );
}
#[test]
fn quality_caps_keep_unknown_height_direct_media() {
    let (mut task, mut diagnostics) = task_and_diagnostics();
    for f in &mut task.media.formats {
        f.height = None;
    }
    for quality in ["2160", "1440", "1080", "720", "480"] {
        task.quality = quality.into();
        let args = build_download_args(&task, &Settings::default(), &diagnostics).unwrap();
        let selector = &args[args.iter().position(|a| a == "--format").unwrap() + 1];
        assert!(selector.contains(&format!("[height<=?{quality}]")));
    }
    diagnostics.ffmpeg.available = true;
    diagnostics.ffprobe.available = true;
    let args = build_download_args(&task, &Settings::default(), &diagnostics).unwrap();
    assert!(args
        .iter()
        .any(|a| a == "bv*[height<=?480]+ba/b[height<=?480]"));
}
