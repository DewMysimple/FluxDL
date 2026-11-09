//! Local yt-dlp adapter. Internal modules are not a stable Rust SDK.
mod arguments;
mod discovery;
mod download;
mod metadata;
mod progress;
pub use crate::diagnostics::redact_log;
use crate::{
    errors::{AppError, AppResult, ErrorCode, ErrorPhase},
    models::{EngineDiagnostics, MediaInfo, Settings},
};
use arguments::common_args;
pub use arguments::{build_download_args, validate_url};
pub use discovery::{detect, EngineResolver};
pub use download::{start_download, start_download_traced, DownloadEvent, EngineDownload};
pub use metadata::{is_media_extension, media_from_json};
pub use progress::{parse_progress, parse_progress_fact, ParsedProgress, ProgressEvent};
use serde_json::Value;
use std::{path::PathBuf, time::Duration};
const METADATA_LIMIT: usize = 16 * 1024 * 1024;
const EJS_HELP: &str = "https://github.com/yt-dlp/yt-dlp/wiki/EJS";
pub async fn probe(
    _settings: &Settings,
    diagnostics: &EngineDiagnostics,
    url: &str,
) -> AppResult<MediaInfo> {
    probe_traced(_settings, diagnostics, url, None).await
}
pub async fn probe_traced(
    _settings: &Settings,
    diagnostics: &EngineDiagnostics,
    url: &str,
    trace: Option<&crate::diagnostics::Trace>,
) -> AppResult<MediaInfo> {
    use crate::diagnostics::{Stage, Status};
    if let Some(t) = trace {
        t.begin(Stage::ParseValidate);
    }
    let result = async {
        let url = validate_url(url)?;
        let path = diagnostics
            .ytdlp
            .path
            .as_deref()
            .filter(|_| diagnostics.ytdlp.available)
            .ok_or_else(|| {
                AppError::new(
                    ErrorCode::DependencyUnavailable,
                    ErrorPhase::Probe,
                    "尚未配置可用的 yt-dlp。请打开设置 → 引擎，选择 yt-dlp.exe 后重新检测。",
                )
            })?;
        let mut args = common_args(diagnostics);
        args.extend(
            [
                "--dump-single-json",
                "--skip-download",
                "--flat-playlist",
                "--playlist-end",
                "1",
                "--",
                &url,
            ]
            .into_iter()
            .map(str::to_owned),
        );
        if let Some(t) = trace {
            t.finish(Stage::ParseValidate, Status::Ok, None);
        }
        let output = crate::process::capture_traced(
            PathBuf::from(path),
            args,
            Duration::from_secs(75),
            METADATA_LIMIT,
            64 * 1024,
            ErrorPhase::Probe,
            trace.cloned(),
        )
        .await?;
        if output.stdout_truncated {
            return Err(AppError::new(
                ErrorCode::EngineProtocol,
                ErrorPhase::Probe,
                "媒体元数据超过安全大小上限。",
            ));
        }
        let (success, stdout, stderr, exit_code) = (
            output.success,
            output.stdout,
            output.stderr,
            output.exit_code,
        );
        if !success {
            return Err(process_failure(ErrorPhase::Probe, &stderr, exit_code));
        }
        let raw: Value = serde_json::from_str(stdout.trim()).map_err(|_| {
            AppError::new(
                ErrorCode::EngineProtocol,
                ErrorPhase::Probe,
                "引擎未返回有效的媒体 JSON。请确认 yt-dlp 版本，或尝试更新后重新解析。",
            )
        })?;
        if let Some(t) = trace {
            t.finish(Stage::ParseMetadataExtract, Status::Ok, None);
            t.begin(Stage::ParseResponse);
        }
        media_from_json(&raw, &url, &stderr)
    }
    .await;
    if let Some(t) = trace {
        t.result(Stage::ParseValidate, &result);
        t.result(Stage::ParseMetadataExtract, &result);
        t.result(Stage::ParseResponse, &result);
    }
    result
}

pub(crate) fn failure_kind(stderr: &str) -> crate::errors::ErrorKind {
    use crate::errors::ErrorKind::*;
    let relevant = stderr
        .lines()
        .rev()
        .find(|l| l.trim_start().starts_with("ERROR:"))
        .or_else(|| stderr.lines().rev().find(|l| !l.trim().is_empty()))
        .unwrap_or("")
        .to_lowercase();
    if ["javascript", "js runtime", "ejs", "n challenge"]
        .iter()
        .any(|v| relevant.contains(v))
    {
        JsRuntime
    } else if ["sign in", "login", "private video", "403"]
        .iter()
        .any(|v| relevant.contains(v))
    {
        AccessRestricted
    } else if relevant.contains("unsupported url") {
        UnsupportedUrl
    } else if relevant.contains("ffmpeg") || relevant.contains("ffprobe") {
        MediaProcessing
    } else if ["no space", "disk full", "errno 28"]
        .iter()
        .any(|v| relevant.contains(v))
    {
        DiskFull
    } else if relevant.contains("permission denied") || relevant.contains("access is denied") {
        PermissionDenied
    } else if ["timed out", "network", "unable to download"]
        .iter()
        .any(|v| relevant.contains(v))
    {
        Network
    } else if relevant.contains("requested format") {
        FormatUnavailable
    } else {
        Unknown
    }
}
pub fn friendly_error(stderr: &str) -> String {
    failure_summary(failure_kind(stderr))
}
pub(crate) fn failure_summary(kind: crate::errors::ErrorKind) -> String {
    use crate::errors::ErrorKind::*;
    match kind {
        JsRuntime => format!("站点解析需要兼容的 JavaScript 运行时与 EJS。请配置 Deno ≥ 2.3 或 Node ≥ 22，并核对 yt-dlp-ejs。官方指引：{EJS_HELP}"),
        AccessRestricted => "站点拒绝访问，媒体可能需要登录、已受限或链接已失效。请使用有权访问的公开链接；应用不会自动读取浏览器凭据。".into(),
        UnsupportedUrl => "暂不支持这个站点或链接类型，请检查完整媒体链接，或更新 yt-dlp 后重试。".into(),
        MediaProcessing => "媒体处理失败。请在设置中配置可运行的 FFmpeg / FFprobe，然后重试。".into(),
        DiskFull => "保存磁盘空间不足，请清理空间或更换保存目录。".into(),
        PermissionDenied => "保存目录或引擎访问被拒绝，请检查文件权限并选择可写目录。".into(),
        Network => "网络请求失败，请检查连接、媒体是否仍可访问，稍后重试。".into(),
        FormatUnavailable => "所选格式已不可用，请重新解析并选择其他格式。".into(),
        Unknown => "媒体解析或下载失败，请检查链接是否有效，或更新 yt-dlp 后重试。".into(),
    }
}
fn process_failure(phase: ErrorPhase, technical: &str, exit: Option<i32>) -> AppError {
    let kind = failure_kind(technical);
    AppError::new(ErrorCode::ProcessExit, phase, failure_summary(kind))
        .with_kind(kind)
        .with_details(format!(
            "phase={phase:?}, exit={}",
            exit.map(|v| v.to_string())
                .unwrap_or_else(|| "unknown".into())
        ))
}

#[cfg(test)]
mod tests;
