mod commands;
mod diagnostic_commands;
pub mod diagnostics;
pub mod engine;
pub mod errors;
pub mod events;
pub mod manager;
pub mod models;
pub mod persistence;
pub mod process;
#[cfg(windows)]
pub(crate) mod process_job;
pub mod transitions;
mod windows;

use manager::Manager;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tauri::{Emitter, Manager as TauriManager};

static EXITING: AtomicBool = AtomicBool::new(false);
pub(crate) fn graceful_exit(app: tauri::AppHandle) {
    if EXITING.swap(true, Ordering::SeqCst) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        if let Some(manager) = app.try_state::<Arc<Manager>>() {
            if let Err(error) = manager.shutdown().await {
                diagnostics::report_error(&error);
                EXITING.store(false, Ordering::SeqCst);
                if let Err(error) = windows::show_workspace(&app) {
                    diagnostics::report_error(&error);
                }
                if let Err(emit_error) = app.emit(
                    "shutdown-failed",
                    format!("退出未完成：{}", error.safe_text()),
                ) {
                    diagnostics::report_error(
                        &errors::AppError::new(
                            errors::ErrorCode::WindowAction,
                            errors::ErrorPhase::Shutdown,
                            "无法显示退出失败通知",
                        )
                        .with_details(emit_error.to_string()),
                    );
                }
                return;
            }
        }
        if let Some(manager) = app.try_state::<Arc<Manager>>() {
            let service = manager.diagnostics.clone();
            let _ = tauri::async_runtime::spawn_blocking(move || service.flush()).await;
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
        // The smoke launcher sets WEBVIEW2_USER_DATA_FOLDER for this child.
        // This Tauri runtime does not forward the config data_directory field.
        for window in &mut context.config_mut().app.windows {
            window.additional_browser_args =
                Some("--remote-debugging-address=127.0.0.1 --remote-debugging-port=9322".into());
        }
    }
    let builder = tauri::Builder::default();
    // Explicit debug smoke runs own their data and WebView profile. They must
    // never redirect to, modify, or terminate an existing user's application.
    #[cfg(debug_assertions)]
    let isolated_smoke = std::env::var_os("FLUXDL_TEST_DATA_DIR").is_some();
    #[cfg(not(debug_assertions))]
    let isolated_smoke = false;
    let builder = if isolated_smoke {
        builder
    } else {
        builder.plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Err(error) = windows::show_workspace(app) {
                diagnostics::report_error(&error);
            }
        }))
    };
    let application = builder
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
            diagnostic_commands::get_diagnostics_health,
            diagnostic_commands::query_diagnostic_events,
            diagnostic_commands::get_diagnostics_overview,
            diagnostic_commands::record_frontend_diagnostic,
            diagnostic_commands::refresh_diagnostic_report,
            diagnostic_commands::open_diagnostics_directory,
            diagnostic_commands::export_diagnostics_bundle,
            diagnostic_commands::reveal_diagnostics_bundle,
            diagnostic_commands::show_task_diagnostics,
            commands::get_app_state,
            commands::get_task_details,
            commands::detect_engines,
            commands::probe_media,
            commands::enqueue_download,
            commands::list_tasks,
            commands::cancel_task,
            commands::retry_task,
            commands::remove_history_record,
            commands::update_settings,
            commands::get_settings,
            commands::list_history,
            commands::open_download_file,
            commands::reveal_in_folder,
            commands::show_quick_capture,
            commands::show_workspace,
            commands::hide_quick_capture,
            commands::resize_quick,
            commands::request_exit,
            commands::open_help,
            commands::window_action
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if EXITING.load(Ordering::SeqCst) {
                    return;
                }
                api.prevent_close();
                if window.label() == "quick" {
                    if let Err(error) = window.hide() {
                        diagnostics::report_error(
                            &errors::AppError::new(
                                errors::ErrorCode::WindowAction,
                                errors::ErrorPhase::Window,
                                "无法隐藏快速捕获窗口",
                            )
                            .with_details(error.to_string()),
                        );
                    }
                    return;
                }
                let app = window.app_handle().clone();
                tauri::async_runtime::spawn(async move {
                    let backend = app.state::<Arc<Manager>>();
                    if backend.has_active().await {
                        if let Err(error) =
                            app.emit_to("workspace", "workspace-close-requested", ())
                        {
                            diagnostics::report_error(
                                &errors::AppError::new(
                                    errors::ErrorCode::WindowAction,
                                    errors::ErrorPhase::Window,
                                    "无法显示后台下载关闭提示",
                                )
                                .with_details(error.to_string()),
                            );
                        }
                    } else if backend.snapshot().await.settings.close_behavior == "exit" {
                        graceful_exit(app.clone());
                    } else if let Some(w) = app.get_webview_window("workspace") {
                        if let Err(error) = w.hide() {
                            diagnostics::report_error(
                                &errors::AppError::new(
                                    errors::ErrorCode::WindowAction,
                                    errors::ErrorPhase::Window,
                                    "无法将工作台隐藏到后台",
                                )
                                .with_details(error.to_string()),
                            );
                        }
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
