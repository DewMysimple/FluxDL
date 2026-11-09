//! Local yt-dlp adapter. URLs and selectors are always individual process arguments.
use crate::models::{
    DownloadTask, EngineDiagnostics, EngineInfo, FormatOption, MediaInfo, Settings,
};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
};

const METADATA_LIMIT: usize = 16 * 1024 * 1024;
const STDERR_LIMIT: usize = 64 * 1024;
const EJS_HELP: &str = "https://github.com/yt-dlp/yt-dlp/wiki/EJS";

pub fn validate_url(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("请先输入媒体链接。".into());
    }
    if value.len() > 8192 || value.chars().any(char::is_control) {
        return Err("链接过长或包含控制字符，请重新复制完整链接。".into());
    }
    let parsed =
        url::Url::parse(value).map_err(|_| "链接格式不正确，请输入完整的 http 或 https 地址。")?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err("仅支持 http / https 媒体链接。".into());
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("不支持包含用户名或密码的链接，请使用公开媒体地址。".into());
    }
    Ok(parsed.into())
}

pub fn hidden_command(path: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut cmd = Command::new(path);
    cmd.stdin(Stdio::null()).kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW; never invoke a shell.
    cmd
}

/// Best-effort timeout cleanup for probe/version subprocesses. Download manager also
/// owns the live process handle and must await its termination before advancing.
pub async fn terminate_process_tree(pid: u32) {
    #[cfg(windows)]
    {
        let system = std::env::var_os("SystemRoot")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        let _ = tokio::time::timeout(
            Duration::from_secs(5),
            hidden_command(system.join("System32").join("taskkill.exe"))
                .args(["/PID", &pid.to_string(), "/T", "/F"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status(),
        )
        .await;
    }
    #[cfg(not(windows))]
    {
        let _ = pid;
    }
}

async fn bounded_read<R: AsyncRead + Unpin>(
    mut reader: R,
    limit: usize,
) -> Result<(Vec<u8>, bool), String> {
    let mut output = Vec::new();
    let mut buffer = [0u8; 8192];
    let mut truncated = false;
    loop {
        let n = reader
            .read(&mut buffer)
            .await
            .map_err(|e| format!("读取引擎输出失败：{e}"))?;
        if n == 0 {
            break;
        }
        let keep = n.min(limit.saturating_sub(output.len()));
        output.extend_from_slice(&buffer[..keep]);
        truncated |= keep < n;
        // Continue consuming both pipes even when memory bounds are reached.
    }
    Ok((output, truncated))
}

async fn capture(
    path: &Path,
    args: &[String],
    seconds: u64,
    limit: usize,
) -> Result<(bool, String, String), String> {
    let mut child = hidden_command(path)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("无法运行引擎：{e}。请检查所选可执行文件与权限。"))?;
    // Own a kill-on-close Job just like downloads: a probe may start a JS runtime,
    // and closing/exiting the app must not leave that descendant running.
    #[cfg(windows)]
    let _job = match crate::process_job::ProcessJob::assign(&child) {
        Ok(job) => job,
        Err(error) => {
            if let Some(pid) = child.id() {
                terminate_process_tree(pid).await;
            }
            let _ = child.kill().await;
            return Err(format!(
                "无法建立引擎进程隔离：{error}。请检查系统进程权限后重试。"
            ));
        }
    };
    let stdout = child.stdout.take().ok_or("引擎标准输出不可用")?;
    let stderr = child.stderr.take().ok_or("引擎错误输出不可用")?;
    let result = tokio::time::timeout(Duration::from_secs(seconds), async {
        let (status, out, err) = tokio::join!(
            child.wait(),
            bounded_read(stdout, limit),
            bounded_read(stderr, STDERR_LIMIT)
        );
        let status = status.map_err(|e| format!("等待引擎失败：{e}"))?;
        let (out, overflow) = out?;
        let (err, _) = err?;
        if overflow {
            return Err("媒体元数据超过安全大小上限；当前版本不支持解析此链接。".into());
        }
        Ok((
            status.success(),
            String::from_utf8_lossy(&out).into_owned(),
            String::from_utf8_lossy(&err).into_owned(),
        ))
    })
    .await;
    match result {
        Ok(value) => value,
        Err(_) => {
            if let Some(pid) = child.id() {
                terminate_process_tree(pid).await;
            }
            let _ = child.kill().await;
            Err(format!(
                "引擎在 {seconds} 秒内未完成。请检查网络连接后重试。"
            ))
        }
    }
}

fn executable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(windows)]
    {
        return path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("exe"));
    }
    #[cfg(not(windows))]
    {
        true
    }
}

async fn check(path: PathBuf, source: &str, version_arg: &str) -> EngineInfo {
    let display = path.to_string_lossy().into_owned();
    if !executable(&path) {
        return EngineInfo {
            path: Some(display),
            source: source.into(),
            message: "路径不是有效的本地可执行文件。".into(),
            ..Default::default()
        };
    }
    match capture(&path, &[version_arg.into()], 8, 64 * 1024).await {
        Ok((true, out, err)) => {
            let text = if out.trim().is_empty() { err } else { out };
            let version = text
                .lines()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("版本未知")
                .trim()
                .chars()
                .take(240)
                .collect();
            EngineInfo {
                path: Some(display),
                version: Some(version),
                source: source.into(),
                available: true,
                message: "已检测，可运行".into(),
            }
        }
        Ok((false, _, err)) => EngineInfo {
            path: Some(display),
            source: source.into(),
            message: format!(
                "版本检测失败：{}",
                redact_log(&err).chars().take(300).collect::<String>()
            ),
            ..Default::default()
        },
        Err(err) => EngineInfo {
            path: Some(display),
            source: source.into(),
            message: err,
            ..Default::default()
        },
    }
}

fn exe_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.into()
    }
}

/// Resolver order is user override, optional installation resource, then PATH.
pub struct EngineResolver<'a> {
    settings: &'a Settings,
    resource_dir: Option<&'a Path>,
}
impl<'a> EngineResolver<'a> {
    pub fn new(settings: &'a Settings, resource_dir: Option<&'a Path>) -> Self {
        Self {
            settings,
            resource_dir,
        }
    }
    pub async fn detect(&self) -> EngineDiagnostics {
        let mut warnings = Vec::new();
        let mut candidates = Vec::new();
        if !self.settings.engine_path.trim().is_empty() {
            candidates.push((PathBuf::from(&self.settings.engine_path), "custom"));
        }
        if let Some(root) = self.resource_dir {
            candidates.push((root.join("binaries").join(exe_name("yt-dlp")), "bundled"));
            candidates.push((root.join(exe_name("yt-dlp")), "bundled"));
        }
        if let Ok(path) = which::which("yt-dlp") {
            candidates.push((path, "PATH"));
        }
        let mut ytdlp = EngineInfo {
            source: "none".into(),
            message: "未找到 yt-dlp。请在设置中选择本机 yt-dlp.exe，或将其加入 PATH。".into(),
            ..Default::default()
        };
        for (path, source) in candidates {
            if source == "bundled" && !path.exists() {
                continue;
            }
            let found = check(path, source, "--version").await;
            if found.available {
                ytdlp = found;
                break;
            }
            warnings.push(format!("{source} yt-dlp：{}", found.message));
            if ytdlp.path.is_none() {
                ytdlp = found;
            }
        }

        let ffmpeg = find_ffmpeg(self.settings, "ffmpeg", &mut warnings).await;
        let ffprobe = find_ffmpeg(self.settings, "ffprobe", &mut warnings).await;
        let mut runtime = EngineInfo {
            source: "none".into(),
            message: "未发现受支持的 Deno / Node 运行时。部分 YouTube 链接需要它们及 yt-dlp-ejs。"
                .into(),
            ..Default::default()
        };
        for name in ["deno", "node"] {
            let mut paths = Vec::new();
            if let Some(parent) = ytdlp.path.as_ref().and_then(|p| Path::new(p).parent()) {
                paths.push(parent.join(exe_name(name)));
            }
            if let Ok(path) = which::which(name) {
                paths.push(path);
            }
            for path in paths {
                if !path.exists() {
                    continue;
                }
                let mut found = check(path, name, "--version").await;
                if found.available
                    && !runtime_supported(name, found.version.as_deref().unwrap_or(""))
                {
                    found.available = false;
                    found.message =
                        format!("{name} 版本过低：需要 Deno ≥ 2.3.0 或 Node ≥ 22。参考 {EJS_HELP}");
                    warnings.push(found.message.clone());
                }
                if found.available {
                    runtime = found;
                    break;
                }
            }
            if runtime.available {
                break;
            }
        }
        if !ffmpeg.available || !ffprobe.available {
            warnings.push(
                "FFmpeg / FFprobe 不完整：仅支持不需要合并的直接视频格式，MP3 转换不可用。".into(),
            );
        }
        warnings.push(format!("版本检测仅验证可执行性；EJS 脚本是否可用需以站点解析结果为准。未启用远程组件下载。官方指引：{EJS_HELP}"));
        EngineDiagnostics {
            ytdlp,
            ffmpeg,
            ffprobe,
            runtime,
            warnings,
        }
    }
}

pub async fn detect(settings: &Settings, resource_dir: Option<&Path>) -> EngineDiagnostics {
    EngineResolver::new(settings, resource_dir).detect().await
}

async fn find_ffmpeg(settings: &Settings, name: &str, warnings: &mut Vec<String>) -> EngineInfo {
    if !settings.ffmpeg_path.trim().is_empty() {
        let custom = PathBuf::from(&settings.ffmpeg_path);
        let path = if custom.is_dir() {
            custom.join(exe_name(name))
        } else if name == "ffmpeg" {
            custom
        } else {
            custom
                .parent()
                .unwrap_or(Path::new(""))
                .join(exe_name(name))
        };
        let info = check(path, "custom", "-version").await;
        if info.available {
            return info;
        }
        warnings.push(format!("手动 {name}：{}；尝试 PATH。", info.message));
    }
    if let Ok(path) = which::which(name) {
        return check(path, "PATH", "-version").await;
    }
    EngineInfo {
        source: "none".into(),
        message: format!("未检测到 {name}。请安装完整 FFmpeg 并在设置中选择其目录。"),
        ..Default::default()
    }
}

fn runtime_supported(name: &str, version: &str) -> bool {
    let version = version
        .trim()
        .trim_start_matches("deno ")
        .trim_start_matches('v');
    let fields: Vec<u32> = version
        .split(|c: char| !c.is_ascii_digit())
        .filter_map(|p| p.parse().ok())
        .take(3)
        .collect();
    if name == "node" {
        fields.first().copied().unwrap_or(0) >= 22
    } else {
        fields.first().copied().unwrap_or(0) > 2
            || (fields.first() == Some(&2) && fields.get(1).copied().unwrap_or(0) >= 3)
    }
}

fn common_args(diagnostics: &EngineDiagnostics) -> Vec<String> {
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

pub async fn probe(
    _settings: &Settings,
    diagnostics: &EngineDiagnostics,
    url: &str,
) -> Result<MediaInfo, String> {
    let url = validate_url(url)?;
    let path = diagnostics
        .ytdlp
        .path
        .as_deref()
        .filter(|_| diagnostics.ytdlp.available)
        .ok_or("尚未配置可用的 yt-dlp。请打开设置 → 引擎，选择 yt-dlp.exe 后重新检测。")?;
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
    let (success, stdout, stderr) = capture(Path::new(path), &args, 75, METADATA_LIMIT).await?;
    if !success {
        return Err(friendly_error(&stderr));
    }
    let raw: Value = serde_json::from_str(stdout.trim())
        .map_err(|_| "引擎未返回有效的媒体 JSON。请确认 yt-dlp 版本，或尝试更新后重新解析。")?;
    media_from_json(&raw, &url, &stderr)
}

pub fn media_from_json(raw: &Value, url: &str, stderr: &str) -> Result<MediaInfo, String> {
    let is_playlist = raw
        .get("_type")
        .and_then(Value::as_str)
        .is_some_and(|s| s == "playlist" || s == "multi_video")
        || raw.get("entries").is_some();
    let mut warnings = stderr
        .lines()
        .filter(|l| l.contains("WARNING") || l.contains("ERROR"))
        .take(12)
        .map(redact_log)
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
        return Err("此媒体没有可下载的音视频格式。可能需要登录、已失效或受站点限制；FluxDL 不会自动读取凭据。".into());
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

pub fn build_download_args(
    task: &DownloadTask,
    _settings: &Settings,
    diagnostics: &EngineDiagnostics,
) -> Result<Vec<String>, String> {
    let url = validate_url(&task.media.url)?;
    if task.media.is_playlist {
        return Err("当前 MVP 不执行整列表批量下载，请使用单个视频链接。".into());
    }
    if !matches!(task.mode.as_str(), "video" | "audio") {
        return Err("不支持的下载模式。".into());
    }
    if !matches!(
        task.quality.as_str(),
        "best" | "2160" | "1440" | "1080" | "720" | "480"
    ) {
        return Err("不支持的质量预设。".into());
    }
    if !diagnostics.ytdlp.available {
        return Err("未找到可用的 yt-dlp，请在设置中配置后重试。".into());
    }
    let postprocess_available = diagnostics.ffmpeg.available && diagnostics.ffprobe.available;
    if task.mode == "audio" && !postprocess_available {
        return Err(
            "仅音频模式会转换为 MP3，需要 FFmpeg 和 FFprobe。请先在设置中配置完整 FFmpeg 目录。"
                .into(),
        );
    }
    if !Path::new(&task.output_dir).is_absolute() || task.output_dir.chars().any(char::is_control) {
        return Err("保存目录必须是有效的绝对路径。".into());
    }
    if task.id.is_empty()
        || !task
            .id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return Err("无效任务 ID。".into());
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
        "download:__FLUX_PROGRESS__%(progress)j".into(),
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
            return Err("格式标识不合法，请重新解析后选择格式。".into());
        }
        Some(
            task.media
                .formats
                .iter()
                .find(|f| &f.format_id == id && is_media_extension(&f.ext))
                .ok_or("所选格式已不可用，请重新解析媒体。")?,
        )
    } else {
        None
    };
    let selector = if task.mode == "audio" {
        if let Some(f) = explicit {
            if !f.has_audio {
                return Err("所选格式不含音轨，请选择可用的音频格式。".into());
            }
            f.format_id.clone()
        } else {
            "bestaudio/best".into()
        }
    } else if let Some(f) = explicit {
        if !f.has_video {
            return Err("所选格式仅含音频，请切换到仅音频模式。".into());
        }
        if !f.has_audio {
            if !postprocess_available {
                return Err("此格式需要合并音轨，请先配置 FFmpeg 和 FFprobe，或选择同时含音视频的直接格式。".into());
            }
            format!("{}+bestaudio", f.format_id)
        } else {
            f.format_id.clone()
        }
    } else {
        if task.media.formats.is_empty() {
            return Err("没有可下载格式，请重新解析媒体。".into());
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
                return Err(
                    "此视频只有分离音视频格式，需要 FFmpeg 合并。请先在设置中配置。".into(),
                );
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

#[derive(Debug, Clone, PartialEq)]
pub enum ProgressEvent {
    Download {
        downloaded_bytes: u64,
        total_bytes: Option<u64>,
        speed: Option<f64>,
        eta: Option<f64>,
        progress: Option<f64>,
    },
    Postprocessing,
    File(String),
}

pub fn parse_progress(line: &str) -> Option<ProgressEvent> {
    let clean = strip_ansi(line);
    let clean = clean.trim();
    if let Some(text) = clean.strip_prefix("__FLUX_PROGRESS__") {
        let value: Value = serde_json::from_str(text).ok()?;
        let downloaded_bytes = json_u64(&value["downloaded_bytes"]).unwrap_or(0);
        let total_bytes = json_u64(&value["total_bytes"])
            .or_else(|| json_u64(&value["total_bytes_estimate"]))
            .filter(|n| *n > 0);
        let progress = total_bytes
            .map(|total| (downloaded_bytes as f64 / total as f64 * 100.0).clamp(0.0, 100.0));
        Some(ProgressEvent::Download {
            downloaded_bytes,
            total_bytes,
            speed: json_f64(&value["speed"]),
            eta: json_f64(&value["eta"]),
            progress,
        })
    } else if clean.starts_with("__FLUX_POST__") {
        Some(ProgressEvent::Postprocessing)
    } else if let Some(text) = clean.strip_prefix("__FLUX_FILE__") {
        let path: String = serde_json::from_str(text).ok()?;
        if path.is_empty() || path.chars().any(char::is_control) {
            return None;
        }
        Some(ProgressEvent::File(path))
    } else {
        None
    }
}
fn json_f64(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|s| s.parse().ok()))
        .filter(|v| v.is_finite() && *v >= 0.0)
}
fn json_u64(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| json_f64(value).map(|f| f as u64))
}
fn strip_ansi(line: &str) -> String {
    let mut out = String::new();
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.next() == Some('[') {
                for n in chars.by_ref() {
                    if ('@'..='~').contains(&n) {
                        break;
                    }
                }
            }
        } else if !c.is_control() || c == '\t' {
            out.push(c);
        }
    }
    out
}

/// URLs in diagnostic logs retain the host/path but never query strings, fragments, or credentials.
pub fn redact_log(line: &str) -> String {
    let line = strip_ansi(line);
    let mut result = line.clone();
    // Scan conservatively; unparseable URL-like values are removed rather than leaked.
    for token in line
        .split_whitespace()
        .filter(|s| s.contains("http://") || s.contains("https://"))
    {
        let begin = token.find("http").unwrap_or(0);
        let candidate = &token[begin..];
        let replacement = if let Ok(mut parsed) =
            url::Url::parse(candidate.trim_end_matches(['\'', '"', ')', ']', ',']))
        {
            parsed.set_query(None);
            parsed.set_fragment(None);
            let _ = parsed.set_username("");
            let _ = parsed.set_password(None);
            parsed.to_string()
        } else {
            "[链接已隐藏]".into()
        };
        result = result.replace(candidate, &replacement);
    }
    result.chars().take(1800).collect()
}

pub fn friendly_error(stderr: &str) -> String {
    let lower = stderr.to_lowercase();
    let detail = stderr
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .map(redact_log)
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

#[cfg(test)]
mod tests {
    use super::*;
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
        assert_eq!(parse_progress("__FLUX_PROGRESS__{\"downloaded_bytes\":2048,\"total_bytes\":null,\"speed\":\"NA\"}"), Some(ProgressEvent::Download {downloaded_bytes:2048,total_bytes:None,speed:None,eta:None,progress:None}));
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
        let line =
            redact_log("ERROR https://user:pass@example.com/video?token=secret#password failed");
        assert!(!line.contains("secret"));
        assert!(!line.contains("pass"));
        assert!(line.contains("example.com/video"));
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
                .contains("FFmpeg")
        );
        task.format_id = Some("18 --exec calc".into());
        assert!(build_download_args(&task, &Settings::default(), &diagnostics).is_err());
        task.format_id = Some("nonexistent".into());
        assert!(build_download_args(&task, &Settings::default(), &diagnostics).is_err());
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
}
