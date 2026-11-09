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
pub use download::{start_download, DownloadEvent, EngineDownload};
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
    let output = crate::process::capture(
        PathBuf::from(path),
        args,
        Duration::from_secs(75),
        METADATA_LIMIT,
        64 * 1024,
        ErrorPhase::Probe,
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
    media_from_json(&raw, &url, &stderr)
}

pub fn friendly_error(stderr: &str) -> String {
    let relevant = stderr
        .lines()
        .rev()
        .find(|line| line.trim_start().starts_with("ERROR:"))
        .or_else(|| stderr.lines().rev().find(|line| !line.trim().is_empty()))
        .unwrap_or("");
    let lower = relevant.to_lowercase();
    let safe_stderr = redact_log(stderr);
    let detail = safe_stderr
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| "引擎没有提供错误详情".into());
    let help = if lower.contains("javascript")
        || lower.contains("js runtime")
        || lower.contains("ejs")
        || lower.contains("n challenge")
    {
        format!("站点解析需要兼容的 JavaScript 运行时与 EJS。请配置 Deno ≥ 2.3 或 Node ≥ 22，并核对 yt-dlp-ejs。官方指引：{EJS_HELP}")
    } else if lower.contains("sign in")
        || lower.contains("login")
        || lower.contains("private video")
        || lower.contains("403")
    {
        "站点拒绝访问，媒体可能需要登录、已受限或链接已失效。请使用有权访问的公开链接；应用不会自动读取浏览器凭据。".into()
    } else if lower.contains("unsupported url") {
        "暂不支持这个站点或链接类型，请检查完整媒体链接，或更新 yt-dlp 后重试。".into()
    } else if lower.contains("ffmpeg") || lower.contains("ffprobe") {
        "媒体处理失败。请在设置中配置可运行的 FFmpeg / FFprobe，然后重试。".into()
    } else if lower.contains("no space")
        || lower.contains("disk full")
        || lower.contains("errno 28")
    {
        "保存磁盘空间不足，请清理空间或更换保存目录。".into()
    } else if lower.contains("permission denied") || lower.contains("access is denied") {
        "保存目录或引擎访问被拒绝，请检查文件权限并选择可写目录。".into()
    } else if lower.contains("timed out")
        || lower.contains("network")
        || lower.contains("unable to download")
    {
        "网络请求失败，请检查连接、媒体是否仍可访问，稍后重试。".into()
    } else if lower.contains("requested format") {
        "所选格式已不可用，请重新解析并选择其他格式。".into()
    } else {
        "媒体解析或下载失败，请检查链接是否有效，或更新 yt-dlp 后重试。".into()
    };
    format!("{help}\n诊断：{detail}")
}

fn process_failure(phase: ErrorPhase, technical: &str, exit: Option<i32>) -> AppError {
    let friendly = friendly_error(technical);
    let (message, detail) = friendly.split_once("\n诊断：").unwrap_or((&friendly, ""));
    AppError::new(ErrorCode::ProcessExit, phase, message).with_details(format!(
        "phase={phase:?}, exit={}; {detail}",
        exit.map(|value| value.to_string())
            .unwrap_or_else(|| "signal/unknown".into())
    ))
}

#[cfg(test)]
mod tests;
