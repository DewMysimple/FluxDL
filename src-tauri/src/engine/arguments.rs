use super::is_media_extension;
use crate::{
    errors::{AppError, AppResult, ErrorCode, ErrorPhase},
    models::{DownloadTask, EngineDiagnostics, Settings},
};
use std::path::Path;
// Build one bounded object, then JSON-encode once. A missing scalar's `j`
// conversion is bypassed by yt-dlp and emits bare NA, so never hand-build JSON.
pub(super) const DOWNLOAD_PROGRESS_TEMPLATE: &str = "download:__FLUX_PROGRESS__%(.{progress.status,progress.filename,info.format_id,info.protocol,progress.downloaded_bytes,progress.total_bytes,progress.total_bytes_estimate,progress.speed,progress.eta}|{})j";
fn invalid(message: &str) -> AppError {
    AppError::new(ErrorCode::InvalidInput, ErrorPhase::Validate, message)
}
fn dependency(message: &str) -> AppError {
    AppError::new(
        ErrorCode::DependencyUnavailable,
        ErrorPhase::Download,
        message,
    )
}
pub fn validate_url(value: &str) -> AppResult<String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(invalid("请先输入媒体链接。"));
    }
    if value.len() > 8192 || value.chars().any(char::is_control) {
        return Err(invalid("链接过长或包含控制字符，请重新复制完整链接。"));
    }
    let parsed = url::Url::parse(value)
        .map_err(|_| invalid("链接格式不正确，请输入完整的 http 或 https 地址。"))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(invalid("仅支持 http / https 媒体链接。"));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(invalid(
            "不支持包含用户名或密码的链接，请使用公开媒体地址。",
        ));
    }
    Ok(parsed.into())
}

pub(super) fn common_args(diagnostics: &EngineDiagnostics) -> Vec<String> {
    let mut args = vec![
        "--ignore-config",
        "--no-playlist",
        "--no-plugin-dirs",
        "--no-remote-components",
        "--no-color",
        "--encoding",
        "utf-8",
        "--socket-timeout",
        "20",
        "--retries",
        "2",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    if diagnostics.runtime.available {
        if let Some(path) = &diagnostics.runtime.path {
            args.extend([
                "--js-runtimes".into(),
                format!("{}:{path}", diagnostics.runtime.source),
            ]);
        }
    }
    if diagnostics.ffmpeg.available {
        if let Some(path) = &diagnostics.ffmpeg.path {
            args.extend(["--ffmpeg-location".into(), path.clone()]);
        }
    }
    args
}

pub fn build_download_args(
    task: &DownloadTask,
    _settings: &Settings,
    diagnostics: &EngineDiagnostics,
) -> AppResult<Vec<String>> {
    let url = validate_url(&task.media.url)?;
    if task.media.is_playlist {
        return Err(invalid(
            "当前 MVP 不执行整列表批量下载，请使用单个视频链接。",
        ));
    }
    if !matches!(task.mode.as_str(), "video" | "audio") {
        return Err(invalid("不支持的下载模式。"));
    }
    if !matches!(
        task.quality.as_str(),
        "best" | "2160" | "1440" | "1080" | "720" | "480"
    ) {
        return Err(invalid("不支持的质量预设。"));
    }
    if !diagnostics.ytdlp.available {
        return Err(dependency("未找到可用的 yt-dlp，请在设置中配置后重试。"));
    }
    let postprocess_available = diagnostics.ffmpeg.available && diagnostics.ffprobe.available;
    if task.mode == "audio" && !postprocess_available {
        return Err(dependency(
            "仅音频模式会转换为 MP3，需要 FFmpeg 和 FFprobe。请先在设置中配置完整 FFmpeg 目录。",
        ));
    }
    if !Path::new(&task.output_dir).is_absolute() || task.output_dir.chars().any(char::is_control) {
        return Err(invalid("保存目录必须是有效的绝对路径。"));
    }
    if task.id.is_empty()
        || !task
            .id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return Err(invalid("无效任务 ID。"));
    }
    let mut args = common_args(diagnostics);
    args.extend(
        [
            "--newline",
            "--progress",
            "--progress-delta",
            "0.2",
            "--no-simulate",
            "--windows-filenames",
            "--trim-filenames",
            "160",
            "--no-overwrites",
            "--paths",
        ]
        .into_iter()
        .map(str::to_owned),
    );
    args.push(task.output_dir.clone());
    args.extend([
        "--output".into(),
        format!("%(title).100B [%(id).30B] {}.%(ext)s", task.id),
    ]);
    args.extend([
        "--progress-template".into(),
        DOWNLOAD_PROGRESS_TEMPLATE.into(),
        "--progress-template".into(),
        "postprocess:__FLUX_POST__%(progress.status)s".into(),
        "--print".into(),
        "after_move:__FLUX_FILE__%(filepath)j".into(),
    ]);

    let explicit = if let Some(id) = &task.format_id {
        if id.is_empty()
            || !id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        {
            return Err(invalid("格式标识不合法，请重新解析后选择格式。"));
        }
        Some(
            task.media
                .formats
                .iter()
                .find(|f| &f.format_id == id && is_media_extension(&f.ext))
                .ok_or_else(|| invalid("所选格式已不可用，请重新解析媒体。"))?,
        )
    } else {
        None
    };
    let selector = if task.mode == "audio" {
        if let Some(f) = explicit {
            if !f.has_audio {
                return Err(invalid("所选格式不含音轨，请选择可用的音频格式。"));
            }
            f.format_id.clone()
        } else {
            "bestaudio/best".into()
        }
    } else if let Some(f) = explicit {
        if !f.has_video {
            return Err(invalid("所选格式仅含音频，请切换到仅音频模式。"));
        }
        if !f.has_audio {
            if !postprocess_available {
                return Err(dependency("此格式需要合并音轨，请先配置 FFmpeg 和 FFprobe，或选择同时含音视频的直接格式。"));
            }
            format!("{}+bestaudio", f.format_id)
        } else {
            f.format_id.clone()
        }
    } else {
        if task.media.formats.is_empty() {
            return Err(invalid("没有可下载格式，请重新解析媒体。"));
        }
        // Generic/direct URLs often omit height. yt-dlp's ? operator includes
        // unknown values while still enforcing the cap for known resolutions.
        let height = if task.quality == "best" {
            String::new()
        } else {
            format!("[height<=?{}]", task.quality)
        };
        if postprocess_available {
            format!("bv*{height}+ba/b{height}")
        } else {
            if !task
                .media
                .formats
                .iter()
                .any(|f| f.has_video && f.has_audio)
            {
                return Err(dependency(
                    "此视频只有分离音视频格式，需要 FFmpeg 合并。请先在设置中配置。",
                ));
            }
            format!("b{height}[vcodec!=none][acodec!=none]")
        }
    };
    args.extend(["--format".into(), selector]);
    if task.mode == "audio" {
        args.extend(
            [
                "--extract-audio",
                "--audio-format",
                "mp3",
                "--audio-quality",
                "0",
            ]
            .into_iter()
            .map(str::to_owned),
        );
    } else {
        args.extend(
            ["--format-sort", "ext:mp4:m4a"]
                .into_iter()
                .map(str::to_owned),
        );
        if postprocess_available {
            args.extend(
                ["--merge-output-format", "mp4/mkv"]
                    .into_iter()
                    .map(str::to_owned),
            );
        }
    }
    args.extend(["--".into(), url]);
    Ok(args)
}
