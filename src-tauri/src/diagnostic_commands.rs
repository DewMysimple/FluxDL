//! Read/query only in both windows. System actions require Workspace and fixed trusted paths.
use crate::{
    diagnostics::{Filter, Health, Overview, Page, Stage},
    manager::Manager,
};
use std::sync::Arc;
use tauri::{Emitter, Manager as _, State};
use tauri_plugin_dialog::DialogExt;
type Backend<'a> = State<'a, Arc<Manager>>;
fn joined(_: impl std::fmt::Debug) -> String {
    "诊断操作未完成；下载不受影响。".into()
}
#[tauri::command]
pub fn get_diagnostics_health(backend: Backend<'_>) -> Health {
    backend.diagnostics.health()
}
#[tauri::command]
pub async fn query_diagnostic_events(
    backend: Backend<'_>,
    filter: Option<Filter>,
) -> Result<Page, String> {
    let service = backend.diagnostics.clone();
    tauri::async_runtime::spawn_blocking(move || service.query(&filter.unwrap_or_default()))
        .await
        .map_err(joined)?
}
#[tauri::command]
pub async fn get_diagnostics_overview(
    backend: Backend<'_>,
    filter: Option<Filter>,
) -> Result<Overview, String> {
    let service = backend.diagnostics.clone();
    tauri::async_runtime::spawn_blocking(move || service.overview(&filter.unwrap_or_default()))
        .await
        .map_err(joined)?
}
#[tauri::command]
pub fn record_frontend_diagnostic(
    backend: Backend<'_>,
    trace_id: String,
    phase: Stage,
    duration_ms: f64,
    success: bool,
) -> Result<(), String> {
    backend
        .diagnostics
        .record_frontend(&trace_id, phase, duration_ms, success)
}
#[tauri::command]
pub async fn refresh_diagnostic_report(backend: Backend<'_>) -> Result<(), String> {
    let service = backend.diagnostics.clone();
    tauri::async_runtime::spawn_blocking(move || service.flush())
        .await
        .map_err(joined)?
}
fn workspace(window: &tauri::WebviewWindow) -> Result<(), String> {
    if window.label() == "workspace" {
        Ok(())
    } else {
        Err("请在工作台诊断中心执行此操作。".into())
    }
}
async fn open(path: std::path::PathBuf, reveal: bool) -> Result<(), String> {
    #[cfg(windows)]
    let mut command = crate::process::hidden_command(
        std::path::PathBuf::from(
            std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into()),
        )
        .join("explorer.exe"),
    );
    #[cfg(not(windows))]
    let mut command = crate::process::hidden_command(if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    });
    #[cfg(windows)]
    if reveal {
        command.arg("/select,");
    }
    let _ = reveal;
    command
        .kill_on_drop(false)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .arg(path)
        .spawn()
        .map_err(|_| "无法打开资源管理器。".to_owned())?;
    Ok(())
}
#[tauri::command]
pub async fn open_diagnostics_directory(
    window: tauri::WebviewWindow,
    backend: Backend<'_>,
) -> Result<(), String> {
    workspace(&window)?;
    let service = backend.diagnostics.clone();
    let root = tauri::async_runtime::spawn_blocking(move || service.directory())
        .await
        .map_err(joined)??;
    open(root, false).await
}
#[tauri::command]
pub async fn export_diagnostics_bundle(
    window: tauri::WebviewWindow,
    backend: Backend<'_>,
    filter: Option<Filter>,
) -> Result<Option<String>, String> {
    workspace(&window)?;
    let filter = filter.unwrap_or_default();
    filter.validate()?;
    let app = window.app_handle().clone();
    let service = backend.diagnostics.clone();
    let engines = backend.snapshot().await.engines;
    let versions = [
        ("yt-dlp", engines.ytdlp.version),
        ("ffmpeg", engines.ffmpeg.version),
        ("ffprobe", engines.ffprobe.version),
    ]
    .into_iter()
    .filter_map(|(k, v)| v.map(|v| (k.to_owned(), v)))
    .collect::<Vec<_>>();
    tauri::async_runtime::spawn_blocking(move || {
        let filename = format!(
            "FluxDL-Diagnostics-{}.zip",
            chrono::Utc::now().format("%Y%m%d-%H%M%S")
        );
        let Some(chosen) = app
            .dialog()
            .file()
            .set_parent(&window)
            .set_title("导出脱敏诊断包（仅本地，不自动上传；请选择新文件名）")
            .set_file_name(&filename)
            .add_filter("ZIP诊断包", &["zip"])
            .blocking_save_file()
        else {
            return Ok(None);
        };
        let path = chosen.into_path().map_err(|_| "请选择本地文件路径。")?;
        let _ = service.flush();
        service.export(&path, &filter, &versions)?;
        // Only a safe success label crosses IPC. Reveal uses the internally remembered path.
        Ok(Some("诊断包已保存到您选择的位置，未上传。".to_owned()))
    })
    .await
    .map_err(joined)?
}
#[tauri::command]
pub async fn reveal_diagnostics_bundle(
    window: tauri::WebviewWindow,
    backend: Backend<'_>,
) -> Result<(), String> {
    workspace(&window)?;
    let path = backend
        .diagnostics
        .last_export()
        .ok_or("请先导出诊断包。")?;
    if !path.is_file() {
        return Err("导出文件已被移动或删除。".into());
    }
    open(path, true).await
}
#[tauri::command]
pub async fn show_task_diagnostics(app: tauri::AppHandle, task_id: String) -> Result<(), String> {
    if !crate::diagnostics::valid_id(&task_id) {
        return Err("任务ID无效。".into());
    }
    crate::windows::show_workspace(&app).map_err(|_| "无法打开工作台。")?;
    app.emit_to("workspace", "diagnostics-locate-task", task_id)
        .map_err(|_| "无法定位任务诊断。".into())
}
