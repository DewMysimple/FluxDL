//! Typed, ordered publication state. It stores changes, never a second task source.
use crate::models::{DownloadTask, EngineDiagnostics, Settings, TaskStatus};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};

pub const APP_STATE_EVENT: &str = "app-state-changed";
const MAX_SAFE_REVISION: u64 = 9_007_199_254_740_991;
pub const LOG_LIMIT: usize = 100;
pub const LOG_LINE_LIMIT: usize = 1600;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppEvent {
    pub session_id: String,
    pub revision: u64,
    pub change: AppChange,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum AppChange {
    Checkpoint,
    TaskUpsert {
        task: DownloadTask,
        index: usize,
    },
    TaskProgress {
        task_id: String,
        status: TaskStatus,
        progress: Option<f64>,
        speed: Option<f64>,
        eta: Option<f64>,
        downloaded_bytes: u64,
        total_bytes: Option<u64>,
        updated_at: String,
        logs_append: Vec<String>,
    },
    TaskRemoved {
        task_id: String,
    },
    SettingsUpdated {
        settings: Settings,
    },
    DiagnosticsUpdated {
        engines: EngineDiagnostics,
    },
    FileAvailabilityUpdated {
        files: Vec<FileAvailability>,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileAvailability {
    pub task_id: String,
    pub file_exists: bool,
}

pub(crate) struct EventJournal {
    pub session_id: String,
    pub revision: u64,
    pub error: Option<String>,
    pending: VecDeque<AppEvent>,
    dirty: HashSet<String>,
    logs: HashMap<String, VecDeque<String>>,
}
impl EventJournal {
    pub fn new() -> Self {
        Self {
            session_id: uuid::Uuid::new_v4().to_string(),
            revision: 0,
            error: None,
            pending: VecDeque::new(),
            dirty: HashSet::new(),
            logs: HashMap::new(),
        }
    }
    pub fn capture(&mut self, change: AppChange) {
        let Some(next) = self
            .revision
            .checked_add(1)
            .filter(|value| *value <= MAX_SAFE_REVISION)
        else {
            self.error = Some("事件版本达到安全上限，请重新启动应用以恢复同步。".into());
            return;
        };
        self.revision = next;
        self.pending.push_back(AppEvent {
            session_id: self.session_id.clone(),
            revision: next,
            change,
        });
    }
    pub fn upsert(&mut self, task: DownloadTask, index: usize) {
        self.clear_task(&task.id);
        self.capture(AppChange::TaskUpsert { task, index });
    }
    pub fn removed(&mut self, id: &str) {
        self.clear_task(id);
        self.capture(AppChange::TaskRemoved { task_id: id.into() });
    }
    pub fn mark_progress(&mut self, id: &str) {
        self.dirty.insert(id.into());
    }
    pub fn append_log(&mut self, id: &str, line: &str) {
        self.mark_progress(id);
        let lines = self.logs.entry(id.into()).or_default();
        lines.push_back(line.chars().take(LOG_LINE_LIMIT).collect());
        while lines.len() > LOG_LIMIT {
            lines.pop_front();
        }
    }
    fn clear_task(&mut self, id: &str) {
        self.dirty.remove(id);
        self.logs.remove(id);
    }
    pub fn flush_progress(&mut self, tasks: &[DownloadTask]) {
        let mut ids = self.dirty.drain().collect::<Vec<_>>();
        ids.sort();
        for id in ids {
            let logs_append = self
                .logs
                .remove(&id)
                .unwrap_or_default()
                .into_iter()
                .collect();
            if let Some(task) = tasks
                .iter()
                .find(|task| task.id == id && task.status.active())
            {
                self.capture(AppChange::TaskProgress {
                    task_id: id,
                    status: task.status,
                    progress: task.progress,
                    speed: task.speed,
                    eta: task.eta,
                    downloaded_bytes: task.downloaded_bytes,
                    total_bytes: task.total_bytes,
                    updated_at: task.updated_at.clone(),
                    logs_append,
                });
            }
        }
    }
    pub fn take_pending(&mut self) -> Vec<AppEvent> {
        self.pending.drain(..).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn serde_envelope_uses_exact_camel_case_and_tag() {
        let event = AppEvent {
            session_id: "session".into(),
            revision: 7,
            change: AppChange::TaskProgress {
                task_id: "task".into(),
                status: TaskStatus::Postprocessing,
                progress: None,
                speed: None,
                eta: None,
                downloaded_bytes: 42,
                total_bytes: None,
                updated_at: "now".into(),
                logs_append: vec!["same text".into(), "same text".into()],
            },
        };
        let value = serde_json::to_value(&event).unwrap();
        assert_eq!(
            value,
            serde_json::json!({"sessionId":"session","revision":7,"change":{"type":"taskProgress","taskId":"task","status":"postprocessing","progress":null,"speed":null,"eta":null,"downloadedBytes":42,"totalBytes":null,"updatedAt":"now","logsAppend":["same text","same text"]}})
        );
        assert!(serde_json::from_value::<AppEvent>(value).is_ok());
    }
    #[test]
    fn revisions_are_per_session_and_do_not_wrap() {
        let mut a = EventJournal::new();
        let b = EventJournal::new();
        assert_ne!(a.session_id, b.session_id);
        a.capture(AppChange::TaskRemoved {
            task_id: "a".into(),
        });
        a.capture(AppChange::TaskRemoved {
            task_id: "b".into(),
        });
        assert_eq!(
            a.take_pending()
                .iter()
                .map(|event| event.revision)
                .collect::<Vec<_>>(),
            [1, 2]
        );
        a.revision = MAX_SAFE_REVISION;
        a.capture(AppChange::TaskRemoved {
            task_id: "c".into(),
        });
        assert_eq!(a.revision, MAX_SAFE_REVISION);
        assert!(a.take_pending().is_empty());
        assert!(a.error.is_some());
    }
    #[test]
    fn bounded_append_keeps_duplicate_text_and_unicode_line_limits() {
        let mut journal = EventJournal::new();
        for _ in 0..105 {
            journal.append_log("a", &"中".repeat(1800));
        }
        let lines = &journal.logs["a"];
        assert_eq!(lines.len(), 100);
        assert!(lines.iter().all(|line| line.chars().count() == 1600));
        assert_eq!(lines[0], lines[1]);
    }
    #[test]
    fn availability_batches_are_compact_and_have_no_full_task() {
        assert_eq!(
            serde_json::to_value(AppChange::Checkpoint).unwrap(),
            serde_json::json!({"type":"checkpoint"})
        );
        let change = AppChange::FileAvailabilityUpdated {
            files: vec![FileAvailability {
                task_id: "id".into(),
                file_exists: false,
            }],
        };
        assert_eq!(
            serde_json::to_value(change).unwrap(),
            serde_json::json!({"type":"fileAvailabilityUpdated","files":[{"taskId":"id","fileExists":false}]})
        );
    }
}
