use super::validate_url;
use crate::{
    errors::{AppError, AppResult, ErrorCode, ErrorPhase},
    models::{FormatOption, MediaInfo},
};
use serde_json::Value;
pub fn media_from_json(raw: &Value, url: &str, stderr: &str) -> AppResult<MediaInfo> {
    if !raw.is_object() {
        return Err(AppError::new(
            ErrorCode::EngineProtocol,
            ErrorPhase::Probe,
            "引擎媒体元数据必须是 JSON 对象。",
        ));
    }
    let is_playlist = raw
        .get("_type")
        .and_then(Value::as_str)
        .is_some_and(|s| s == "playlist" || s == "multi_video")
        || raw.get("entries").is_some();
    let mut warnings = stderr
        .lines()
        .filter(|l| l.contains("WARNING") || l.contains("ERROR"))
        .take(12)
        .map(|_| "引擎报告警告；原始链接、标题和路径已省略，请查看诊断中心。".to_owned())
        .take(1)
        .collect::<Vec<_>>();
    if is_playlist {
        warnings.push(
            "当前 MVP 不执行整列表批量下载。请打开列表中的单个视频，复制该视频链接后重新解析。"
                .into(),
        );
    }
    let mut formats = raw
        .get("formats")
        .and_then(Value::as_array)
        .map(|v| v.iter().filter_map(format_from_json).collect::<Vec<_>>())
        .unwrap_or_default();
    if formats.is_empty() && !is_playlist {
        if let Some(format) = format_from_json(raw) {
            formats.push(format);
        }
    }
    // yt-dlp may expose storyboard-only formats. They are not media download choices.
    formats.retain(|f| f.has_audio || f.has_video);
    if formats.is_empty() && !is_playlist {
        return Err(AppError::new(ErrorCode::EngineProtocol,ErrorPhase::Probe,"此媒体没有可下载的音视频格式。可能需要登录、已失效或受站点限制；FluxDL 不会自动读取凭据。"));
    }
    let thumbnail = raw
        .get("thumbnail")
        .and_then(Value::as_str)
        .and_then(|s| validate_url(s).ok());
    let title = raw
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("未命名媒体")
        .chars()
        .take(2048)
        .collect();
    Ok(MediaInfo {
        trusted_preview_ref: None,
        url: url.into(),
        title,
        media_id: raw
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("media")
            .chars()
            .take(256)
            .collect(),
        source: raw
            .get("extractor_key")
            .or_else(|| raw.get("extractor"))
            .and_then(Value::as_str)
            .unwrap_or("媒体链接")
            .chars()
            .take(120)
            .collect(),
        duration: raw
            .get("duration")
            .and_then(Value::as_f64)
            .filter(|n| n.is_finite() && *n >= 0.0),
        thumbnail,
        is_playlist,
        formats,
        warnings,
    })
}

fn format_from_json(raw: &Value) -> Option<FormatOption> {
    let ext = raw.get("ext")?.as_str()?.to_owned();
    if !is_media_extension(&ext) || raw.get("has_drm").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let format_id = raw
        .get("format_id")
        .and_then(Value::as_str)
        .unwrap_or("best")
        .to_owned();
    let height = raw
        .get("height")
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok());
    let audio_ext = matches!(
        ext.as_str(),
        "mp3" | "m4a" | "aac" | "ogg" | "opus" | "wav" | "flac" | "wma" | "aiff" | "aif" | "alac"
    );
    let has_video = raw
        .get("vcodec")
        .and_then(Value::as_str)
        .map(|s| s != "none")
        .unwrap_or(!audio_ext);
    let has_audio = raw
        .get("acodec")
        .and_then(Value::as_str)
        .map(|s| s != "none")
        .unwrap_or(true);
    let resolution = height.map(|h| format!("{h}p")).unwrap_or_else(|| {
        if has_video {
            "视频".into()
        } else {
            "音频".into()
        }
    });
    let label = format!(
        "{} · {}{}",
        resolution,
        ext.to_uppercase(),
        if has_video && !has_audio {
            " · 需合并音轨"
        } else {
            ""
        }
    );
    Some(FormatOption {
        format_id,
        ext,
        label,
        height,
        has_video,
        has_audio,
        filesize: raw
            .get("filesize")
            .or_else(|| raw.get("filesize_approx"))
            .and_then(Value::as_u64),
        requires_ffmpeg: has_video && !has_audio,
    })
}

/// Generic URLs may point to executables or documents. Only media containers are
/// accepted, and the same allowlist can guard the eventual system "open file".
pub fn is_media_extension(ext: &str) -> bool {
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "mp4"
            | "mkv"
            | "webm"
            | "mov"
            | "avi"
            | "m4v"
            | "flv"
            | "wmv"
            | "mpg"
            | "mpeg"
            | "ts"
            | "mts"
            | "m2ts"
            | "3gp"
            | "ogv"
            | "m4a"
            | "mp3"
            | "aac"
            | "ogg"
            | "opus"
            | "wav"
            | "flac"
            | "wma"
            | "aiff"
            | "aif"
            | "alac"
    )
}
