//! Compatibility adapter: command names/successes stay unchanged; error DTO is opt-in.
use crate::{
    errors::{AppError, AppResult, ErrorCode, ErrorFormat, ErrorPhase, IpcError},
    manager::Manager,
    models::*,
    windows,
};
use std::sync::Arc;
use tauri::{Emitter, Manager as TauriManager, State};
type Backend<'a> = State<'a, Arc<Manager>>;
fn ipc<T>(result: AppResult<T>, format: Option<ErrorFormat>) -> Result<T, IpcError> {
    result.map_err(|error| error.into_ipc(format))
}

#[tauri::command]
pub async fn get_app_state(
    backend: Backend<'_>,
    error_format: Option<ErrorFormat>,
) -> Result<AppSnapshot, IpcError> {
    ipc(Ok(backend.snapshot().await), error_format)
}
#[tauri::command]
pub async fn get_task_details(
    backend: Backend<'_>,
    task_id: String,
    error_format: Option<ErrorFormat>,
) -> Result<TaskDetails, IpcError> {
    ipc(backend.details(&task_id).await, error_format)
}
#[tauri::command]
pub async fn detect_engines(
    backend: Backend<'_>,
    error_format: Option<ErrorFormat>,
) -> Result<EngineDiagnostics, IpcError> {
    ipc(Ok(backend.detect().await), error_format)
}
#[tauri::command]
pub async fn probe_media(
    backend: Backend<'_>,
    url: String,
    trace_id: Option<String>,
    error_format: Option<ErrorFormat>,
) -> Result<MediaInfo, IpcError> {
    let trace = backend
        .diagnostics
        .request_trace(trace_id.as_deref(), None)
        .map_err(|message| {
            AppError::new(ErrorCode::InvalidInput, ErrorPhase::Validate, message)
                .into_ipc(error_format)
        })?;
    ipc(backend.probe_with_trace(&url, trace).await, error_format)
}
#[tauri::command]
pub async fn enqueue_download(
    backend: Backend<'_>,
    input: EnqueueInput,
    trace_id: Option<String>,
    related_trace_id: Option<String>,
    error_format: Option<ErrorFormat>,
) -> Result<DownloadTask, IpcError> {
    let trace = backend
        .diagnostics
        .request_trace(trace_id.as_deref(), related_trace_id.as_deref())
        .map_err(|message| {
            AppError::new(ErrorCode::InvalidInput, ErrorPhase::Validate, message)
                .into_ipc(error_format)
        })?;
    ipc(backend.enqueue_with_trace(input, trace).await, error_format)
}
#[tauri::command]
pub async fn list_tasks(
    backend: Backend<'_>,
    error_format: Option<ErrorFormat>,
) -> Result<Vec<DownloadTask>, IpcError> {
    ipc(Ok(backend.snapshot().await.tasks), error_format)
}
#[tauri::command]
pub async fn cancel_task(
    backend: Backend<'_>,
    task_id: String,
    error_format: Option<ErrorFormat>,
) -> Result<(), IpcError> {
    ipc(backend.cancel(&task_id).await, error_format)
}
#[tauri::command]
pub async fn retry_task(
    backend: Backend<'_>,
    task_id: String,
    error_format: Option<ErrorFormat>,
) -> Result<(), IpcError> {
    ipc(backend.retry(&task_id).await, error_format)
}
#[tauri::command]
pub async fn remove_history_record(
    backend: Backend<'_>,
    task_id: String,
    error_format: Option<ErrorFormat>,
) -> Result<(), IpcError> {
    ipc(backend.remove_history(&task_id).await, error_format)
}
#[tauri::command]
pub async fn update_settings(
    backend: Backend<'_>,
    settings: Settings,
    error_format: Option<ErrorFormat>,
) -> Result<Settings, IpcError> {
    ipc(backend.update_settings(settings).await, error_format)
}
#[tauri::command]
pub async fn get_settings(
    backend: Backend<'_>,
    error_format: Option<ErrorFormat>,
) -> Result<Settings, IpcError> {
    ipc(Ok(backend.snapshot().await.settings), error_format)
}
#[tauri::command]
pub async fn list_history(
    backend: Backend<'_>,
    query: Option<String>,
    error_format: Option<ErrorFormat>,
) -> Result<Vec<DownloadTask>, IpcError> {
    let query = query.unwrap_or_default().to_lowercase();
    let mut tasks: Vec<_> = backend
        .snapshot()
        .await
        .tasks
        .into_iter()
        .filter(|task| {
            task.status == TaskStatus::Completed
                && (task.media.title.to_lowercase().contains(&query)
                    || task.media.source.to_lowercase().contains(&query))
        })
        .collect();
    tasks.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    ipc(Ok(tasks), error_format)
}
#[tauri::command]
pub async fn open_download_file(
    backend: Backend<'_>,
    task_id: String,
    error_format: Option<ErrorFormat>,
) -> Result<(), IpcError> {
    let result =
        async { windows::open_recorded_file(backend.task(&task_id).await?, false).await }.await;
    ipc(result, error_format)
}
#[tauri::command]
pub async fn reveal_in_folder(
    backend: Backend<'_>,
    task_id: String,
    error_format: Option<ErrorFormat>,
) -> Result<(), IpcError> {
    let result =
        async { windows::open_recorded_file(backend.task(&task_id).await?, true).await }.await;
    ipc(result, error_format)
}
#[tauri::command]
pub fn show_quick_capture(
    app: tauri::AppHandle,
    error_format: Option<ErrorFormat>,
) -> Result<(), IpcError> {
    ipc(windows::show_quick(&app), error_format)
}
#[tauri::command]
pub fn show_workspace(
    app: tauri::AppHandle,
    error_format: Option<ErrorFormat>,
) -> Result<(), IpcError> {
    ipc(windows::show_workspace(&app), error_format)
}
#[tauri::command]
pub fn hide_quick_capture(
    app: tauri::AppHandle,
    error_format: Option<ErrorFormat>,
) -> Result<(), IpcError> {
    ipc(
        (|| {
            let window = app
                .get_webview_window("quick")
                .ok_or_else(|| window_error("快速捕获窗口不存在"))?;
            window.hide().map_err(|error| {
                window_error("无法隐藏快速捕获窗口").with_details(error.to_string())
            })
        })(),
        error_format,
    )
}
fn window_error(message: &str) -> AppError {
    AppError::new(ErrorCode::WindowAction, ErrorPhase::Window, message)
}
#[tauri::command]
pub fn resize_quick(
    app: tauri::AppHandle,
    height: f64,
    error_format: Option<ErrorFormat>,
) -> Result<(), IpcError> {
    ipc(
        (|| {
            if !height.is_finite() {
                return Err(AppError::new(
                    ErrorCode::InvalidInput,
                    ErrorPhase::Validate,
                    "窗口尺寸无效",
                ));
            }
            let window = app
                .get_webview_window("quick")
                .ok_or_else(|| window_error("快速捕获窗口不存在"))?;
            let monitor = match window.current_monitor() {
                Ok(monitor) => monitor,
                Err(error) => {
                    crate::diagnostics::report_error(
                        &window_error("读取显示器失败，使用默认窗口尺寸")
                            .with_details(error.to_string()),
                    );
                    None
                }
            };
            let max = monitor
                .map(|m| m.size().height as f64 / m.scale_factor() - 64.0)
                .unwrap_or(720.0);
            window
                .set_size(tauri::LogicalSize::new(
                    632.0,
                    height.clamp(260.0, max.clamp(260.0, 720.0)),
                ))
                .map_err(|error| {
                    window_error("无法调整快速捕获窗口尺寸").with_details(error.to_string())
                })
        })(),
        error_format,
    )
}
#[tauri::command]
pub fn request_exit(
    app: tauri::AppHandle,
    error_format: Option<ErrorFormat>,
) -> Result<(), IpcError> {
    crate::graceful_exit(app);
    ipc(Ok(()), error_format)
}
#[tauri::command]
pub async fn open_help(topic: String, error_format: Option<ErrorFormat>) -> Result<(), IpcError> {
    ipc(windows::open_help(&topic).await, error_format)
}
#[tauri::command]
pub async fn window_action(
    window: tauri::WebviewWindow,
    action: String,
    backend: Backend<'_>,
    error_format: Option<ErrorFormat>,
) -> Result<(), IpcError> {
    let result = async {
        match action.as_str() {
            "minimize" => window.minimize(),
            "maximize" => {
                if window.is_maximized().map_err(|error| {
                    window_error("无法读取窗口状态").with_details(error.to_string())
                })? {
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
                    window.emit("workspace-close-requested", ())
                } else if backend.snapshot().await.settings.close_behavior == "exit" {
                    crate::graceful_exit(window.app_handle().clone());
                    Ok(())
                } else {
                    window.hide()
                }
            }
            _ => {
                return Err(AppError::new(
                    ErrorCode::InvalidInput,
                    ErrorPhase::Validate,
                    "未知窗口操作",
                ))
            }
        }
        .map_err(|error| window_error("窗口操作失败").with_details(error.to_string()))
    }
    .await;
    ipc(result, error_format)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn success_and_legacy_error_values_are_unchanged() {
        assert_eq!(
            ipc(Ok::<_, AppError>(vec![1, 2]), Some(ErrorFormat::Structured)).unwrap(),
            vec![1, 2]
        );
        let legacy = ipc::<()>(
            Err(AppError::new(
                ErrorCode::InvalidInput,
                ErrorPhase::Validate,
                "无效输入",
            )),
            None,
        )
        .unwrap_err();
        assert_eq!(serde_json::to_value(legacy).unwrap(), "无效输入");
    }
}
