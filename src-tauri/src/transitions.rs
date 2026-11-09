//! Pure candidate transitions. Manager owns admission locks, persistence and publication.
use crate::{
    engine::{ParsedProgress, ProgressEvent},
    errors::{AppError, AppResult, ErrorCode, ErrorPhase},
    models::{DownloadTask, TaskStatus},
    process::OutputStream,
};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeChange {
    Unchanged,
    Progress,
    FullUpsert,
    Diagnostic(&'static str),
}

#[derive(Debug, Default)]
pub struct RuntimeTracker {
    sequence: HashMap<OutputStream, u64>,
    stream: Option<String>,
    retired: HashSet<String>,
    finished: bool,
    regressed: bool,
}
impl RuntimeTracker {
    pub fn admit_line(&mut self, source: OutputStream, sequence: u64) -> bool {
        let previous = self.sequence.entry(source).or_default();
        if sequence <= *previous {
            return false;
        }
        *previous = sequence;
        true
    }
}

pub fn admits_runtime(
    current_attempt: Option<u64>,
    expected: u64,
    status: &TaskStatus,
    pending_terminal: bool,
    cancel_requested: bool,
) -> bool {
    current_attempt == Some(expected)
        && status.running()
        && *status != TaskStatus::Cancelling
        && !pending_terminal
        && !cancel_requested
}

pub fn claim(task: &mut DownloadTask) -> bool {
    if task.status != TaskStatus::Queued {
        return false;
    }
    task.status = TaskStatus::Resolving;
    clear_runtime(task);
    task.error = None;
    true
}

pub fn recover(task: &mut DownloadTask) -> bool {
    if !task.status.active() {
        return false;
    }
    task.status = TaskStatus::Interrupted;
    task.error = Some("上次运行尚未结束。请点击重试，FluxDL 不会自动恢复下载。".into());
    true
}

pub fn request_cancel(task: &mut DownloadTask) -> bool {
    match task.status {
        TaskStatus::Queued => task.status = TaskStatus::Cancelled,
        TaskStatus::Resolving | TaskStatus::Downloading | TaskStatus::Postprocessing => {
            task.status = TaskStatus::Cancelling
        }
        _ => return false,
    }
    task.speed = None;
    task.eta = None;
    true
}

pub fn retry(task: &mut DownloadTask) -> AppResult<()> {
    if !matches!(
        task.status,
        TaskStatus::Failed | TaskStatus::Cancelled | TaskStatus::Interrupted
    ) {
        return Err(AppError::new(
            ErrorCode::StateConflict,
            ErrorPhase::Validate,
            "仅失败、已取消或已中断的任务可以重试。",
        ));
    }
    task.status = TaskStatus::Queued;
    clear_runtime(task);
    task.final_path = None;
    task.file_exists = false;
    task.error = None;
    task.logs.clear();
    Ok(())
}

/// Must be applied to a private candidate, then published only after durable ack.
pub fn settle(task: &mut DownloadTask, cancelled: bool, result: AppResult<()>) -> bool {
    if !task.status.running() {
        return false;
    }
    task.progress = None;
    task.speed = None;
    task.eta = None;
    match (cancelled, result) {
        (true, Ok(())) => {
            task.status = TaskStatus::Cancelled;
            task.error = None;
        }
        (false, Ok(())) => {
            task.status = TaskStatus::Completed;
            task.file_exists = true;
            task.error = None;
        }
        (_, Err(error)) => {
            task.status = TaskStatus::Failed;
            task.error = Some(error.safe_text());
        }
    }
    true
}

fn clear_runtime(task: &mut DownloadTask) {
    task.progress = None;
    task.speed = None;
    task.eta = None;
    task.downloaded_bytes = 0;
    task.total_bytes = None;
}

pub fn apply_progress(
    task: &mut DownloadTask,
    tracker: &mut RuntimeTracker,
    source: OutputStream,
    line_seq: u64,
    fact: &ParsedProgress,
) -> RuntimeChange {
    if !task.status.running() || task.status == TaskStatus::Cancelling {
        return RuntimeChange::Unchanged;
    }
    if !tracker.admit_line(source, line_seq) {
        return RuntimeChange::Unchanged;
    }
    match &fact.event {
        ProgressEvent::Download {
            downloaded_bytes,
            total_bytes,
            speed,
            eta,
            progress,
        } => {
            if fact
                .stream_id
                .as_ref()
                .is_some_and(|id| tracker.retired.contains(id))
            {
                return RuntimeChange::Unchanged;
            }
            let new_stream = fact
                .stream_id
                .as_ref()
                .is_some_and(|id| tracker.stream.as_ref().is_some_and(|old| old != id));
            // A late record cannot reopen postprocessing or rewind one known stream.
            if !new_stream
                && (task.status == TaskStatus::Postprocessing
                    || (tracker.finished && !fact.finished))
            {
                return RuntimeChange::Unchanged;
            }
            if !new_stream && *downloaded_bytes < task.downloaded_bytes {
                if tracker.regressed {
                    return RuntimeChange::Unchanged;
                }
                tracker.regressed = true;
                return RuntimeChange::Diagnostic(
                    "引擎报告同一下载流的字节数回退，已保留最近进度；未合成为总体百分比。",
                );
            }
            tracker.regressed = false;
            if new_stream {
                // Single-video jobs have few component streams. Refuse unbounded identities.
                if tracker.retired.len() >= 32 {
                    return RuntimeChange::Unchanged;
                }
                if let Some(old) = tracker.stream.take() {
                    tracker.retired.insert(old);
                }
            }
            if let Some(id) = &fact.stream_id {
                tracker.stream = Some(id.clone());
            }
            tracker.finished = fact.finished;
            task.status = TaskStatus::Downloading;
            task.downloaded_bytes = *downloaded_bytes;
            task.total_bytes = *total_bytes;
            task.speed = *speed;
            task.eta = *eta;
            task.progress = *progress;
            if new_stream {
                RuntimeChange::FullUpsert
            } else {
                RuntimeChange::Progress
            }
        }
        ProgressEvent::Postprocessing => {
            task.status = TaskStatus::Postprocessing;
            task.progress = None;
            task.speed = None;
            task.eta = None;
            RuntimeChange::Progress
        }
        ProgressEvent::File(path) => {
            if task.final_path.as_ref() == Some(path) {
                return RuntimeChange::Unchanged;
            }
            task.final_path = Some(path.clone());
            RuntimeChange::FullUpsert
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::MediaInfo;
    fn task() -> DownloadTask {
        DownloadTask {
            id: "transition-test".into(),
            media: MediaInfo {
                url: "https://example.com/a".into(),
                title: "a".into(),
                media_id: "a".into(),
                source: "test".into(),
                duration: None,
                thumbnail: None,
                is_playlist: false,
                formats: vec![],
                warnings: vec![],
            },
            mode: "video".into(),
            quality: "best".into(),
            format_id: None,
            output_dir: "C:/test".into(),
            status: TaskStatus::Queued,
            progress: None,
            speed: None,
            eta: None,
            downloaded_bytes: 0,
            total_bytes: None,
            final_path: None,
            error: None,
            logs: vec![],
            created_at: "original".into(),
            updated_at: "now".into(),
            file_exists: false,
        }
    }
    fn fact(id: &str, bytes: u64, total: u64) -> ParsedProgress {
        ParsedProgress {
            event: ProgressEvent::Download {
                downloaded_bytes: bytes,
                total_bytes: Some(total),
                speed: Some(1.0),
                eta: Some(1.0),
                progress: Some(bytes as f64 / total as f64 * 100.0),
            },
            stream_id: Some(id.into()),
            finished: false,
        }
    }
    #[test]
    fn admission_cancel_terminal_and_retry_follow_one_transition_table() {
        let mut task = task();
        assert!(!admits_runtime(Some(1), 1, &task.status, false, false));
        assert!(claim(&mut task));
        assert!(!claim(&mut task));
        assert!(admits_runtime(Some(1), 1, &task.status, false, false));
        assert!(!admits_runtime(Some(2), 1, &task.status, false, false));
        assert!(!admits_runtime(Some(1), 1, &task.status, true, false));
        assert!(request_cancel(&mut task));
        assert!(!request_cancel(&mut task));
        assert!(settle(&mut task, true, Ok(())));
        assert_eq!(task.status, TaskStatus::Cancelled);
        assert!(!settle(&mut task, false, Ok(())));
        retry(&mut task).unwrap();
        assert_eq!(task.status, TaskStatus::Queued);
        assert_eq!(task.created_at, "original");
        assert!(!settle(&mut task, false, Ok(())));
        claim(&mut task);
        settle(&mut task, false, Ok(()));
        assert_eq!(task.status, TaskStatus::Completed);
        assert!(!request_cancel(&mut task));
        assert!(retry(&mut task).is_err());
        let mut failed = task.clone();
        failed.status = TaskStatus::Cancelling;
        settle(
            &mut failed,
            true,
            Err(AppError::new(
                ErrorCode::ProcessCleanup,
                ErrorPhase::Shutdown,
                "清理失败",
            )),
        );
        assert_eq!(failed.status, TaskStatus::Failed);
        assert!(failed.error.unwrap().contains("清理失败"));
        for status in [
            TaskStatus::Queued,
            TaskStatus::Resolving,
            TaskStatus::Downloading,
            TaskStatus::Postprocessing,
            TaskStatus::Cancelling,
        ] {
            let mut interrupted = task.clone();
            interrupted.status = status;
            assert!(recover(&mut interrupted));
            assert_eq!(interrupted.status, TaskStatus::Interrupted);
            assert!(!recover(&mut interrupted));
        }
    }
    #[test]
    fn component_reset_is_real_while_retired_cross_pipe_streams_and_terminal_progress_are_rejected()
    {
        let mut task = task();
        claim(&mut task);
        let mut tracker = RuntimeTracker::default();
        assert_eq!(
            apply_progress(
                &mut task,
                &mut tracker,
                OutputStream::Stdout,
                1,
                &fact("A", 100, 100)
            ),
            RuntimeChange::Progress
        );
        assert_eq!(
            apply_progress(
                &mut task,
                &mut tracker,
                OutputStream::Stderr,
                1,
                &fact("B", 10, 100)
            ),
            RuntimeChange::FullUpsert
        );
        assert_eq!(task.progress, Some(10.0));
        assert_eq!(
            apply_progress(
                &mut task,
                &mut tracker,
                OutputStream::Stdout,
                2,
                &fact("A", 100, 100)
            ),
            RuntimeChange::Unchanged
        );
        assert_eq!(task.downloaded_bytes, 10);
        apply_progress(
            &mut task,
            &mut tracker,
            OutputStream::Stderr,
            2,
            &fact("B", 10, 200),
        );
        assert_eq!(task.progress, Some(5.0));
        assert!(matches!(
            apply_progress(
                &mut task,
                &mut tracker,
                OutputStream::Stderr,
                3,
                &fact("B", 9, 200)
            ),
            RuntimeChange::Diagnostic(_)
        ));
        let post = ParsedProgress {
            event: ProgressEvent::Postprocessing,
            stream_id: None,
            finished: false,
        };
        apply_progress(&mut task, &mut tracker, OutputStream::Stderr, 4, &post);
        assert_eq!(
            apply_progress(
                &mut task,
                &mut tracker,
                OutputStream::Stderr,
                5,
                &fact("B", 20, 200)
            ),
            RuntimeChange::Unchanged
        );
        assert_eq!(
            apply_progress(
                &mut task,
                &mut tracker,
                OutputStream::Stderr,
                6,
                &fact("C", 1, 100)
            ),
            RuntimeChange::FullUpsert
        );
        settle(&mut task, false, Ok(()));
        assert_eq!(
            apply_progress(
                &mut task,
                &mut tracker,
                OutputStream::Stderr,
                7,
                &fact("D", 1, 100)
            ),
            RuntimeChange::Unchanged
        );
    }
}
