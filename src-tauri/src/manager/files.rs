use crate::{
    engine,
    errors::{AppError, AppResult, ErrorCode, ErrorPhase},
};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
};
use tokio::process::Command;

pub(super) async fn prepare_directory(directory: String) -> AppResult<PathBuf> {
    tokio::task::spawn_blocking(move || writable_directory(&directory))
        .await
        .map_err(|error| {
            output_error(ErrorPhase::Validate, "保存目录检查任务未完成。")
                .with_details(error.to_string())
        })?
}

pub(super) async fn validate_output(raw: String, directory: String) -> AppResult<PathBuf> {
    tokio::task::spawn_blocking(move || validate_final_path(&raw, &directory))
        .await
        .map_err(|error| {
            output_error(ErrorPhase::Download, "输出文件检查任务未完成。")
                .with_details(error.to_string())
        })?
}

pub fn writable_directory(raw: &str) -> AppResult<PathBuf> {
    if raw.len() > 32700 || !Path::new(raw).is_absolute() {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            ErrorPhase::Validate,
            "请选择有效的绝对保存目录路径。",
        ));
    }
    std::fs::create_dir_all(raw).map_err(|error| {
        output_error(
            ErrorPhase::Validate,
            "无法创建保存目录，请检查路径、权限及磁盘空间。",
        )
        .with_details(error.to_string())
    })?;
    let directory = std::fs::canonicalize(raw).map_err(|error| {
        output_error(ErrorPhase::Validate, "无法访问保存目录。").with_details(error.to_string())
    })?;
    let test = directory.join(format!(".fluxdl-write-test-{}", uuid::Uuid::new_v4()));
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&test)
        .map_err(|error| {
            output_error(ErrorPhase::Validate, "保存目录不可写，请选择其他目录。")
                .with_details(error.to_string())
        })?;
    std::fs::remove_file(test).map_err(|error| {
        output_error(ErrorPhase::Validate, "无法清理目录权限检查文件。")
            .with_details(error.to_string())
    })?;
    Ok(directory)
}

pub fn validate_final_path(raw: &str, output_dir: &str) -> AppResult<PathBuf> {
    let path = std::fs::canonicalize(raw).map_err(|_| {
        output_error(
            ErrorPhase::Download,
            "本地文件不存在：引擎报告的输出文件没有落地或已被移动。",
        )
    })?;
    let directory = std::fs::canonicalize(output_dir).map_err(|error| {
        output_error(ErrorPhase::Download, "无法检查保存目录。").with_details(error.to_string())
    })?;
    if !path.is_file() || !path.starts_with(directory) {
        return Err(output_error(
            ErrorPhase::Download,
            "引擎报告的文件不在本任务保存目录内，已阻止不安全的文件操作。",
        ));
    }
    if !path
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(engine::is_media_extension)
    {
        return Err(output_error(
            ErrorPhase::Download,
            "引擎报告的输出不是受支持的媒体文件类型，已阻止完成和打开此文件。请检查任务日志。",
        ));
    }
    Ok(path)
}

pub(super) async fn open_folder(directory: &str) -> AppResult<()> {
    #[cfg(windows)]
    let executable = "explorer.exe";
    #[cfg(target_os = "macos")]
    let executable = "open";
    #[cfg(all(not(windows), not(target_os = "macos")))]
    let executable = "xdg-open";
    let mut command = Command::new(executable);
    command
        .arg(directory)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    command.spawn().map_err(|error| {
        AppError::new(
            ErrorCode::SystemOpen,
            ErrorPhase::Window,
            "无法打开保存目录。",
        )
        .with_details(error.to_string())
    })?;
    Ok(())
}

fn output_error(phase: ErrorPhase, message: &str) -> AppError {
    AppError::new(ErrorCode::OutputValidation, phase, message)
}
