pub mod engine;
pub mod manager;
pub mod models;
pub mod persistence;
#[cfg(windows)]
pub(crate) mod process_job;
mod windows;

use manager::Manager;
use models::*;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tauri::{Emitter, Manager as TauriManager, State};

static EXITING: AtomicBool = AtomicBool::new(false);
type Backend<'a> = State<'a, Arc<Manager>>;

#[tauri::command]
async fn get_app_state(backend: Backend<'_>) -> Result<AppSnapshot, String> {
    Ok(backend.snapshot().await)
}
#[tauri::command]
async fn detect_engines(backend: Backend<'_>) -> Result<EngineDiagnostics, String> {
    Ok(backend.detect().await)
}
#[tauri::command]
async fn probe_media(backend: Backend<'_>, url: String) -> Result<MediaInfo, String> {
    backend.probe(&url).await
}
#[tauri::command]
async fn enqueue_download(
    backend: Backend<'_>,
    input: EnqueueInput,
) -> Result<DownloadTask, String> {
    backend.enqueue(input).await
}
#[tauri::command]
async fn list_tasks(backend: Backend<'_>) -> Result<Vec<DownloadTask>, String> {
    Ok(backend.snapshot().await.tasks)
}
#[tauri::command]
async fn cancel_task(backend: Backend<'_>, task_id: String) -> Result<(), String> {
    backend.cancel(&task_id).await
}
#[tauri::command]
async fn retry_task(backend: Backend<'_>, task_id: String) -> Result<(), String> {
    backend.retry(&task_id).await
}
#[tauri::command]
async fn remove_history_record(backend: Backend<'_>, task_id: String) -> Result<(), String> {
    backend.remove_history(&task_id).await
}
#[tauri::command]
async fn update_settings(backend: Backend<'_>, settings: Settings) -> Result<Settings, String> {
    backend.update_settings(settings).await
}
#[tauri::command]
async fn get_settings(backend: Backend<'_>) -> Result<Settings, String> {
    Ok(backend.snapshot().await.settings)
}
#[tauri::command]
async fn list_history(
    backend: Backend<'_>,
    query: Option<String>,
) -> Result<Vec<DownloadTask>, String> {
    let query = query.unwrap_or_default().to_lowercase();
    let mut tasks: Vec<_> = backend
        .snapshot()
        .await
        .tasks
        .into_iter()
        .filter(|t| {
            t.status == TaskStatus::Completed
                && (t.media.title.to_lowercase().contains(&query)
                    || t.media.source.to_lowercase().contains(&query))
        })
        .collect();
    tasks.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(tasks)
}
#[tauri::command]
async fn open_download_file(backend: Backend<'_>, task_id: String) -> Result<(), String> {
    windows::open_recorded_file(backend.task(&task_id).await?, false).await
}
#[tauri::command]
async fn reveal_in_folder(backend: Backend<'_>, task_id: String) -> Result<(), String> {
    windows::open_recorded_file(backend.task(&task_id).await?, true).await
}
#[tauri::command]
fn show_quick_capture(app: tauri::AppHandle) -> Result<(), String> {
    windows::show_quick(&app)
}
#[tauri::command]
fn show_workspace(app: tauri::AppHandle) -> Result<(), String> {
    windows::show_workspace(&app)
}
#[tauri::command]
fn hide_quick_capture(app: tauri::AppHandle) -> Result<(), String> {
    app.get_webview_window("quick")
        .ok_or("快速捕获窗口不存在")?
        .hide()
        .map_err(|e| e.to_string())
}
#[tauri::command]
fn resize_quick(app: tauri::AppHandle, height: f64) -> Result<(), String> {
    if !height.is_finite() {
        return Err("窗口尺寸无效".into());
    }
    let w = app
        .get_webview_window("quick")
        .ok_or("快速捕获窗口不存在")?;
    let monitor_max = w
        .current_monitor()
        .ok()
        .flatten()
        .map(|m| m.size().height as f64 / m.scale_factor() - 64.0)
        .unwrap_or(720.0);
    w.set_size(tauri::LogicalSize::new(
        632.0,
        height.clamp(260.0, monitor_max.clamp(260.0, 720.0)),
    ))
    .map_err(|e| e.to_string())
}
#[tauri::command]
fn request_exit(app: tauri::AppHandle) {
    graceful_exit(app);
}
#[tauri::command]
async fn open_help(topic: String) -> Result<(), String> {
    windows::open_help(&topic).await
}
#[tauri::command]
async fn window_action(
    window: tauri::WebviewWindow,
    action: String,
    backend: Backend<'_>,
) -> Result<(), String> {
    match action.as_str() {
        "minimize" => window.minimize(),
        "maximize" => {
            if window.is_maximized().map_err(|e| e.to_string())? {
                window.unmaximize()
            } else {
                window.maximize()
            }
        }
        "hide" => window.hide(),
        "close" => {
            if window.label() == "quick" {
                window.hide()
            } else if backend.has_active().await {
                window
                    .emit("workspace-close-requested", ())
                    .map_err(|e| e.to_string())?;
                Ok(())
            } else if backend.snapshot().await.settings.close_behavior == "exit" {
                graceful_exit(window.app_handle().clone());
                Ok(())
            } else {
                window.hide()
            }
        }
        _ => return Err("未知窗口操作".into()),
    }
    .map_err(|e| e.to_string())
}

pub(crate) fn graceful_exit(app: tauri::AppHandle) {
    if EXITING.swap(true, Ordering::SeqCst) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        if let Some(manager) = app.try_state::<Arc<Manager>>() {
            if let Err(error) = manager.shutdown().await {
                eprintln!("FluxDL shutdown: {error}");
                EXITING.store(false, Ordering::SeqCst);
                let _ = windows::show_workspace(&app);
                let _ = app.emit(
                    "shutdown-failed",
                    "停止下载进程失败。请重试退出或在任务管理器中检查残留进程。",
                );
                return;
            }
        }
        app.exit(0);
    });
}

pub fn run() {
    #[allow(unused_mut)]
    let mut context = tauri::generate_context!();
    // This diagnostic port exists only in explicit debug smoke runs, never in release builds.
    #[cfg(debug_assertions)]
    if std::env::var_os("FLUXDL_TEST_DATA_DIR").is_some() {
        for window in &mut context.config_mut().app.windows {
            window.additional_browser_args =
                Some("--remote-debugging-address=127.0.0.1 --remote-debugging-port=9322".into());
            window.data_directory = Some("FluxDL-Smoke-WebView".into());
        }
    }
    let application = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            let _ = windows::show_workspace(app);
        }))
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let app_handle = app.handle().clone();
            let data_dir = app.path().app_data_dir()?;
            #[cfg(debug_assertions)]
            let data_dir = std::env::var_os("FLUXDL_TEST_DATA_DIR")
                .map(std::path::PathBuf::from)
                .unwrap_or(data_dir);
            let resources = app.path().resource_dir().ok();
            let manager = tauri::async_runtime::block_on(Manager::init(
                data_dir,
                resources,
                Some(app_handle),
            ))
            .map_err(std::io::Error::other)?;
            app.manage(manager);
            windows::install_tray(app)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_app_state,
            detect_engines,
            probe_media,
            enqueue_download,
            list_tasks,
            cancel_task,
            retry_task,
            remove_history_record,
            update_settings,
            get_settings,
            list_history,
            open_download_file,
            reveal_in_folder,
            show_quick_capture,
            show_workspace,
            hide_quick_capture,
            resize_quick,
            request_exit,
            open_help,
            window_action
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if EXITING.load(Ordering::SeqCst) {
                    return;
                }
                api.prevent_close();
                if window.label() == "quick" {
                    let _ = window.hide();
                    return;
                }
                let app = window.app_handle().clone();
                tauri::async_runtime::spawn(async move {
                    let backend = app.state::<Arc<Manager>>();
                    if backend.has_active().await {
                        let _ = app.emit_to("workspace", "workspace-close-requested", ());
                    } else if backend.snapshot().await.settings.close_behavior == "exit" {
                        graceful_exit(app.clone());
                    } else if let Some(w) = app.get_webview_window("workspace") {
                        let _ = w.hide();
                    }
                });
            }
        })
        .build(context)
        .expect("FluxDL 启动失败：请检查 AppData 目录权限与数据库状态");
    application.run(|app, event| {
        if let tauri::RunEvent::ExitRequested { api, .. } = event {
            if !EXITING.load(Ordering::SeqCst) {
                api.prevent_exit();
                graceful_exit(app.clone());
            }
        }
    });
}
