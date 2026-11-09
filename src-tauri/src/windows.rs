use crate::{
    graceful_exit,
    models::{DownloadTask, TaskStatus},
};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager,
};

pub fn show_workspace(app: &tauri::AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("workspace")
        .ok_or("工作台窗口不存在")?;
    window.unminimize().map_err(|e| e.to_string())?;
    window.show().map_err(|e| e.to_string())?;
    window.set_focus().map_err(|e| e.to_string())
}
pub fn show_quick(app: &tauri::AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("quick")
        .ok_or("快速捕获窗口不存在")?;
    let monitor = app
        .cursor_position()
        .ok()
        .and_then(|cursor| app.monitor_from_point(cursor.x, cursor.y).ok().flatten())
        .or_else(|| window.current_monitor().ok().flatten());
    if let Some(monitor) = monitor {
        let size = monitor.size();
        let pos = monitor.position();
        let scale = monitor.scale_factor();
        let width = (632.0 * scale) as i32;
        let x = pos.x + (size.width as i32 - width).max(0) / 2;
        let window_height = window.outer_size().map(|s| s.height).unwrap_or(540) as i32;
        let y_offset = ((size.height as f64 * 0.18) as i32)
            .min((size.height as i32 - window_height - 32).max(0));
        let y = pos.y + y_offset;
        window
            .set_position(tauri::PhysicalPosition::new(x, y))
            .map_err(|e| e.to_string())?;
    } else {
        window.center().map_err(|e| e.to_string())?;
    }
    window.show().map_err(|e| e.to_string())?;
    window.set_focus().map_err(|e| e.to_string())
}
pub fn install_tray(app: &tauri::App) -> tauri::Result<()> {
    let workspace = MenuItem::with_id(app, "workspace", "打开工作台", true, None::<&str>)?;
    let quick = MenuItem::with_id(app, "quick", "快速捕获", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出 FluxDL（停止下载）", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&workspace, &quick, &quit])?;
    let icon = app
        .default_window_icon()
        .cloned()
        .expect("application icon");
    TrayIconBuilder::with_id("fluxdl-tray")
        .icon(icon)
        .tooltip("FluxDL · 下载在后台继续")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "workspace" => {
                let _ = show_workspace(app);
            }
            "quick" => {
                let _ = show_quick(app);
            }
            "quit" => graceful_exit(app.clone()),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let _ = show_workspace(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

pub async fn open_recorded_file(task: DownloadTask, reveal: bool) -> Result<(), String> {
    if task.status != TaskStatus::Completed {
        return Err("该任务尚未完成，无法打开文件".into());
    }
    let path = task.final_path.ok_or("任务未记录输出文件")?;
    let canonical = tokio::fs::canonicalize(&path)
        .await
        .map_err(|_| "本地文件不存在，可能已移动或删除".to_string())?;
    let folder = tokio::fs::canonicalize(&task.output_dir)
        .await
        .map_err(|_| "保存目录不存在".to_string())?;
    if !canonical.starts_with(&folder) || !canonical.is_file() {
        return Err("输出路径未通过安全验证".into());
    }
    if !reveal
        && !canonical
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(crate::engine::is_media_extension)
    {
        return Err("此文件类型不属于受支持的媒体格式，仅允许在资源管理器中查看。".into());
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        let mut command = if reveal {
            let mut c = std::process::Command::new("explorer.exe");
            c.arg(format!("/select,{}", canonical.display()));
            c
        } else {
            // ShellExecuteW opens the registered application, with no shell command parsing.
            return tokio::task::spawn_blocking(move || shell_open(&canonical))
                .await
                .map_err(|e| e.to_string())?;
        };
        command.creation_flags(0x08000000);
        command
            .spawn()
            .map_err(|e| format!("无法打开资源管理器：{e}"))?;
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    {
        let target = if reveal { folder } else { canonical };
        #[cfg(target_os = "macos")]
        let program = "open";
        #[cfg(not(target_os = "macos"))]
        let program = "xdg-open";
        tokio::process::Command::new(program)
            .arg(target)
            .spawn()
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[cfg(target_os = "windows")]
fn shell_open(path: &std::path::Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "shell32")]
    extern "system" {
        fn ShellExecuteW(
            hwnd: isize,
            op: *const u16,
            file: *const u16,
            params: *const u16,
            dir: *const u16,
            show: i32,
        ) -> isize;
    }
    let filename: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let verb: Vec<u16> = "open\0".encode_utf16().collect();
    let result = unsafe {
        ShellExecuteW(
            0,
            verb.as_ptr(),
            filename.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1,
        )
    };
    if result <= 32 {
        Err(format!(
            "系统无法打开此文件（错误 {result}），请检查文件关联"
        ))
    } else {
        Ok(())
    }
}

pub async fn open_help(topic: &str) -> Result<(), String> {
    let url = match topic {
        "ejs" => "https://github.com/yt-dlp/yt-dlp/wiki/EJS",
        "ytdlp" => "https://github.com/yt-dlp/yt-dlp",
        "ffmpeg" => "https://ffmpeg.org/download.html",
        _ => return Err("未知帮助主题".into()),
    };
    #[cfg(windows)]
    {
        tokio::task::spawn_blocking(move || shell_open(std::path::Path::new(url)))
            .await
            .map_err(|e| e.to_string())?
    }
    #[cfg(not(windows))]
    {
        #[cfg(target_os = "macos")]
        let program = "open";
        #[cfg(not(target_os = "macos"))]
        let program = "xdg-open";
        tokio::process::Command::new(program)
            .arg(url)
            .spawn()
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}
