use super::EJS_HELP;
use crate::{
    diagnostics::redact_log,
    errors::ErrorPhase,
    models::{EngineDiagnostics, EngineInfo, Settings},
    process::capture,
};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
fn executable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(windows)]
    {
        path.extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
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
    match capture(
        path.clone(),
        vec![version_arg.into()],
        Duration::from_secs(8),
        64 * 1024,
        64 * 1024,
        ErrorPhase::Detect,
    )
    .await
    {
        Ok(output) if output.success => {
            let out = output.stdout;
            let err = output.stderr;
            let text = redact_log(&if out.trim().is_empty() { err } else { out });
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
        Ok(output) => EngineInfo {
            path: Some(display),
            source: source.into(),
            message: format!(
                "版本检测失败（exit={:?}）：{}",
                output.exit_code,
                redact_log(&output.stderr)
                    .chars()
                    .take(300)
                    .collect::<String>()
            ),
            ..Default::default()
        },
        Err(err) => EngineInfo {
            path: Some(display),
            source: source.into(),
            message: err.safe_text(),
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

pub(super) fn runtime_supported(name: &str, version: &str) -> bool {
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
