//! Small, bounded ZIP32 STORE writer. Fixed entry names; no filesystem traversal.
use super::*;
use std::{fs::OpenOptions, io::Write};
fn crc(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for b in bytes {
        crc ^= *b as u32;
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb88320 & (0u32.wrapping_sub(crc & 1)));
        }
    }
    !crc
}
fn u16le(b: &mut Vec<u8>, v: u16) {
    b.extend(v.to_le_bytes());
}
fn u32le(b: &mut Vec<u8>, v: u32) {
    b.extend(v.to_le_bytes());
}
pub(super) fn zip(entries: &[(String, Vec<u8>)]) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    let mut central = Vec::new();
    for (name, data) in entries {
        if data.len() > 32 * 1024 * 1024 || output.len() + data.len() > 64 * 1024 * 1024 {
            return Err("诊断包超过64MiB安全上限；请缩小时间范围。".into());
        }
        let offset = output.len() as u32;
        let checksum = crc(data);
        u32le(&mut output, 0x04034b50);
        for value in [20, 0x800, 0, 0, 33] {
            u16le(&mut output, value);
        }
        u32le(&mut output, checksum);
        u32le(&mut output, data.len() as u32);
        u32le(&mut output, data.len() as u32);
        u16le(&mut output, name.len() as u16);
        u16le(&mut output, 0);
        output.extend(name.as_bytes());
        output.extend(data);
        u32le(&mut central, 0x02014b50);
        for value in [20, 20, 0x800, 0, 0, 33] {
            u16le(&mut central, value);
        }
        u32le(&mut central, checksum);
        u32le(&mut central, data.len() as u32);
        u32le(&mut central, data.len() as u32);
        for value in [name.len() as u16, 0, 0, 0, 0] {
            u16le(&mut central, value);
        }
        u32le(&mut central, 0);
        u32le(&mut central, offset);
        central.extend(name.as_bytes());
    }
    let size = central.len() as u32;
    let offset = output.len() as u32;
    output.extend(central);
    u32le(&mut output, 0x06054b50);
    u16le(&mut output, 0);
    u16le(&mut output, 0);
    u16le(&mut output, entries.len() as u16);
    u16le(&mut output, entries.len() as u16);
    u32le(&mut output, size);
    u32le(&mut output, offset);
    u16le(&mut output, 0);
    Ok(output)
}
pub(super) fn bundle(
    destination: &Path,
    events: &[Event],
    health: &Health,
    versions: &[(String, String)],
    unreadable: usize,
    truncated: bool,
) -> Result<(), String> {
    let parent = destination.parent().ok_or("请选择有效的导出目录。")?;
    writer::check_directory(parent).map_err(|_| "导出目录不可用或包含链接。")?;
    if destination.extension().and_then(|v| v.to_str()) != Some("zip") {
        return Err("诊断包仅允许.zip扩展名。".into());
    }
    let mut lines = Vec::new();
    let mut errors = Vec::new();
    for event in events.iter().filter(|e| e.valid()) {
        let mut line = serde_json::to_vec(event).map_err(|_| "诊断序列化失败。")?;
        line.push(b'\n');
        if event.level == Level::Error {
            errors.extend(&line);
        }
        lines.extend(line);
    }
    let safe_versions = safe_versions(versions);
    let env = serde_json::json!({"app_version":env!("CARGO_PKG_VERSION"),"os":std::env::consts::OS,"os_major_version":os_major_version(),"architecture":std::env::consts::ARCH,"mode":health.mode,"engine_versions":safe_versions,"engine_versions_scope":"current detected only; historical samples use session_environments","session_environments":events.iter().filter(|e| e.fields.environment.is_some()).map(|e| serde_json::json!({"session_id":e.session_id,"environment_id":e.fields.environment_id,"environment":e.fields.environment})).collect::<Vec<_>>(),"configuration":"external-engines;serial-queue;no-credentials-collected"});
    let manifest = serde_json::json!({"schema_version":1,"included":["stage enums","UUID correlation","monotonic durations","error categories","safe engine versions"],"excluded":["URLs","titles","paths","cookies","tokens","headers","proxy credentials","raw stderr","command lines","clipboard","SQLite","environment variables"],"policy":"typed whitelist at ingestion and strict validation on read; never uploaded","unreadable_lines":unreadable,"truncated":truncated});
    let entries=vec![
        ("summary.md".into(),(report::markdown(events,health.mode,&health.session_id,unreadable,truncated,health.dropped,health.critical_dropped)+&report::engine_versions(&safe_versions)).into_bytes()),
        ("events.jsonl".into(),lines),("errors.jsonl".into(),errors),
        ("environment.json".into(),serde_json::to_vec_pretty(&env).unwrap()),
        ("privacy-manifest.json".into(),serde_json::to_vec_pretty(&manifest).unwrap()),
        ("README.md".into(),"# FluxDL 本地诊断包\n\n仅由用户主动导出，未上传或分享给Codex。请检查内容后自行决定分享。先读summary.md，再用Trace ID查询events.jsonl。错误索引派生自相同事件。缺失阶段不能视为0ms。配置、媒体文件和数据库均未收集。\n".as_bytes().to_vec()),
    ];
    let bytes = zip(&entries)?;
    // create_new refuses overwrite, symlinks and pre-existing files even after dialog confirmation.
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|_| "无法创建诊断包；不会覆盖已有文件，请选择新文件名。")?;
    if file.write_all(&bytes).and_then(|_| file.flush()).is_err() {
        drop(file);
        let _ = std::fs::remove_file(destination);
        return Err("诊断包写入失败，请检查空间和权限。".into());
    }
    Ok(())
}
pub(super) fn safe_versions(
    versions: &[(String, String)],
) -> std::collections::BTreeMap<String, String> {
    versions
        .iter()
        .filter_map(|(name, value)| {
            if !matches!(name.as_str(), "yt-dlp" | "ffmpeg" | "ffprobe") {
                return None;
            }
            let value = value
                .strip_prefix("ffmpeg version ")
                .or_else(|| value.strip_prefix("ffprobe version "))
                .unwrap_or(value)
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim_start_matches(['v', 'n']);
            let core = value
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect::<String>();
            let core = core.trim_end_matches('.');
            let parts: Vec<_> = core.split('.').collect();
            ((2..=4).contains(&parts.len()) && parts.iter().all(|p| !p.is_empty() && p.len() <= 4))
                .then(|| (name.clone(), core.to_owned()))
        })
        .collect()
}
#[cfg(windows)]
fn os_major_version() -> Option<u32> {
    // https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/wdm/nf-wdm-rtlgetversion
    #[repr(C)]
    struct Version {
        size: u32,
        major: u32,
        minor: u32,
        build: u32,
        platform: u32,
        service_pack: [u16; 128],
    }
    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn RtlGetVersion(version: *mut Version) -> i32;
    }
    let mut version = Version {
        size: std::mem::size_of::<Version>() as u32,
        major: 0,
        minor: 0,
        build: 0,
        platform: 0,
        service_pack: [0; 128],
    };
    // SAFETY: an initialized writable OSVERSIONINFOW-compatible buffer with exact cbSize.
    if unsafe { RtlGetVersion(&mut version) } == 0 {
        Some(version.major)
    } else {
        None
    }
}
#[cfg(not(windows))]
fn os_major_version() -> Option<u32> {
    None
}
