use crate::{
    errors::{AppError, AppResult, ErrorCode, ErrorPhase},
    graceful_exit,
    models::{DownloadTask, TaskStatus},
};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager,
};

pub fn show_workspace(app: &tauri::AppHandle) -> AppResult<()> {
    let window = app
        .get_webview_window("workspace")
        .ok_or_else(|| window_error("工作台窗口不存在"))?;
    window
        .unminimize()
        .map_err(|e| window_error("原生窗口操作失败").with_details(e.to_string()))?;
    window
        .show()
        .map_err(|e| window_error("原生窗口操作失败").with_details(e.to_string()))?;
    window
        .set_focus()
        .map_err(|e| window_error("原生窗口操作失败").with_details(e.to_string()))
}
pub fn show_quick(app: &tauri::AppHandle) -> AppResult<()> {
    let window = app
        .get_webview_window("quick")
        .ok_or_else(|| window_error("快速捕获窗口不存在"))?;
    let monitor = fallback_diagnostic(
        app.cursor_position(),
        "读取光标位置失败，使用窗口所在显示器",
    )
    .and_then(|cursor| {
        fallback_diagnostic(
            app.monitor_from_point(cursor.x, cursor.y),
            "读取光标所在显示器失败",
        )
        .flatten()
    })
    .or_else(|| fallback_diagnostic(window.current_monitor(), "读取窗口所在显示器失败").flatten());
    if let Some(monitor) = monitor {
        let size = monitor.size();
        let pos = monitor.position();
        let scale = monitor.scale_factor();
        let width = (632.0 * scale) as i32;
        let x = pos.x + (size.width as i32 - width).max(0) / 2;
        let window_height =
            fallback_diagnostic(window.outer_size(), "读取窗口外框尺寸失败，使用默认尺寸")
                .map(|s| s.height)
                .unwrap_or(540) as i32;
        let y_offset = ((size.height as f64 * 0.18) as i32)
            .min((size.height as i32 - window_height - 32).max(0));
        let y = pos.y + y_offset;
        window
            .set_position(tauri::PhysicalPosition::new(x, y))
            .map_err(|e| window_error("原生窗口操作失败").with_details(e.to_string()))?;
    } else {
        window
            .center()
            .map_err(|e| window_error("原生窗口操作失败").with_details(e.to_string()))?;
    }
    window
        .show()
        .map_err(|e| window_error("原生窗口操作失败").with_details(e.to_string()))?;
    window
        .set_focus()
        .map_err(|e| window_error("原生窗口操作失败").with_details(e.to_string()))
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
    #[cfg(debug_assertions)]
    let tooltip = if std::env::var_os("FLUXDL_TEST_DATA_DIR").is_some() {
        format!("FluxDL · 测试实例 {}", std::process::id())
    } else {
        "FluxDL · 下载在后台继续".into()
    };
    #[cfg(not(debug_assertions))]
    let tooltip = "FluxDL · 下载在后台继续";
    TrayIconBuilder::with_id("fluxdl-tray")
        .icon(icon)
        .tooltip(tooltip)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "workspace" => {
                if let Err(error) = show_workspace(app) {
                    crate::diagnostics::report_error(&error);
                }
            }
            "quick" => {
                if let Err(error) = show_quick(app) {
                    crate::diagnostics::report_error(&error);
                }
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
                if let Err(error) = show_workspace(tray.app_handle()) {
                    crate::diagnostics::report_error(&error);
                }
            }
        })
        .build(app)?;
    Ok(())
}

pub async fn open_recorded_file(task: DownloadTask, reveal: bool) -> AppResult<()> {
    if task.status != TaskStatus::Completed {
        return Err(AppError::new(
            ErrorCode::StateConflict,
            ErrorPhase::Validate,
            "该任务尚未完成，无法打开文件",
        ));
    }
    let path = task.final_path.ok_or_else(|| {
        AppError::new(
            ErrorCode::OutputValidation,
            ErrorPhase::Validate,
            "任务未记录输出文件",
        )
    })?;
    let canonical = tokio::fs::canonicalize(&path).await.map_err(|_| {
        AppError::new(
            ErrorCode::OutputValidation,
            ErrorPhase::Validate,
            "本地文件不存在，可能已移动或删除",
        )
    })?;
    let folder = tokio::fs::canonicalize(&task.output_dir)
        .await
        .map_err(|_| {
            AppError::new(
                ErrorCode::OutputValidation,
                ErrorPhase::Validate,
                "保存目录不存在",
            )
        })?;
    if !canonical.starts_with(&folder) || !canonical.is_file() {
        return Err(AppError::new(
            ErrorCode::OutputValidation,
            ErrorPhase::Validate,
            "输出路径未通过安全验证",
        ));
    }
    if !reveal
        && !canonical
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(crate::engine::is_media_extension)
    {
        return Err(AppError::new(
            ErrorCode::OutputValidation,
            ErrorPhase::Validate,
            "此文件类型不属于受支持的媒体格式，仅允许在资源管理器中查看。",
        ));
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
                .map_err(|e| window_error("原生窗口操作失败").with_details(e.to_string()))?;
        };
        command.creation_flags(0x08000000);
        command.spawn().map_err(|e| {
            AppError::new(
                ErrorCode::SystemOpen,
                ErrorPhase::Window,
                "无法打开资源管理器",
            )
            .with_details(e.to_string())
        })?;
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
            .map_err(|e| window_error("原生窗口操作失败").with_details(e.to_string()))?;
        Ok(())
    }
}

#[cfg(target_os = "windows")]
fn shell_open(path: &std::path::Path) -> AppResult<()> {
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
        Err(AppError::new(
            ErrorCode::SystemOpen,
            ErrorPhase::Window,
            format!("系统无法打开此文件（错误 {result}），请检查文件关联"),
        ))
    } else {
        Ok(())
    }
}

pub async fn open_help(topic: &str) -> AppResult<()> {
    let url = match topic {
        "ejs" => "https://github.com/yt-dlp/yt-dlp/wiki/EJS",
        "ytdlp" => "https://github.com/yt-dlp/yt-dlp",
        "ffmpeg" => "https://ffmpeg.org/download.html",
        _ => {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                ErrorPhase::Validate,
                "未知帮助主题",
            ))
        }
    };
    #[cfg(windows)]
    {
        tokio::task::spawn_blocking(move || shell_open(std::path::Path::new(url)))
            .await
            .map_err(|e| window_error("原生窗口操作失败").with_details(e.to_string()))?
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
            .map_err(|e| window_error("原生窗口操作失败").with_details(e.to_string()))?;
        Ok(())
    }
}

fn window_error(message: &str) -> AppError {
    AppError::new(ErrorCode::WindowAction, ErrorPhase::Window, message)
}
fn fallback_diagnostic<T, E: std::fmt::Display>(result: Result<T, E>, message: &str) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(error) => {
            crate::diagnostics::report_error(
                &window_error(message).with_details(error.to_string()),
            );
            None
        }
    }
}
