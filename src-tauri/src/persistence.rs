mod repository;
mod writer;
pub use repository::{ApplyOutcome, Loaded, Mutation, Repository};
pub use writer::{PersistenceWriter, WriteFailure};
pub type Persistence = Repository;
#[cfg(test)]
use crate::models::{DownloadTask, Settings, TaskStatus};
#[cfg(test)]
use rusqlite::{params, Connection};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::MediaInfo;
    fn directory() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("fluxdl-db-test-{}", uuid::Uuid::new_v4()))
    }
    fn task(status: TaskStatus) -> DownloadTask {
        DownloadTask {
            id: "test".into(),
            media: MediaInfo {
                url: "https://example.com/video".into(),
                title: "test".into(),
                media_id: "test".into(),
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
            output_dir: "C:/Downloads".into(),
            status,
            progress: Some(20.0),
            speed: None,
            eta: None,
            downloaded_bytes: 12,
            total_bytes: None,
            final_path: None,
            error: None,
            logs: vec![],
            created_at: "2026-01-01".into(),
            updated_at: "2026-01-01".into(),
            file_exists: false,
        }
    }
    #[test]
    fn persisted_active_tasks_become_interrupted_and_settings_survive() {
        let directory = directory();
        let (mut store, _) = Persistence::open(&directory).unwrap();
        let settings = Settings {
            theme: "light".into(),
            ..Settings::default()
        };
        store
            .save(&settings, &[task(TaskStatus::Downloading)])
            .unwrap();
        drop(store);
        let (store, loaded) = Persistence::open(&directory).unwrap();
        assert_eq!(loaded.settings.theme, "light");
        assert_eq!(loaded.tasks[0].status, TaskStatus::Interrupted);
        drop(store);
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn corrupted_database_is_preserved() {
        let directory = directory();
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("fluxdl.sqlite3"), b"not a sqlite database").unwrap();
        let (store, loaded) = Persistence::open(&directory).unwrap();
        assert!(loaded.recovery_warning.is_some());
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 2);
        let backup = std::fs::read_dir(&directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("fluxdl-corrupt-")
            })
            .unwrap();
        assert_eq!(std::fs::read(backup).unwrap(), b"not a sqlite database");
        drop(store);
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn legacy_v1_schema_reads_settings_and_completed_document() {
        let directory = directory();
        std::fs::create_dir_all(&directory).unwrap();
        let connection = Connection::open(directory.join("fluxdl.sqlite3")).unwrap();
        connection.execute_batch("CREATE TABLE app_settings (id INTEGER PRIMARY KEY CHECK(id=1), document TEXT NOT NULL); CREATE TABLE tasks (id TEXT PRIMARY KEY, document TEXT NOT NULL); PRAGMA user_version=1;").unwrap();
        let settings = Settings {
            download_dir: "C:/fixture-downloads".into(),
            theme: "light".into(),
            quality: "720".into(),
            ..Settings::default()
        };
        let mut completed = task(TaskStatus::Completed);
        completed.logs = vec!["旧版本中文日志".into()];
        let expected = serde_json::to_value(&completed).unwrap();
        connection
            .execute(
                "INSERT INTO app_settings VALUES(1,?1)",
                [serde_json::to_string(&settings).unwrap()],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO tasks VALUES(?1,?2)",
                params![completed.id, serde_json::to_string(&completed).unwrap()],
            )
            .unwrap();
        drop(connection);
        let (store, loaded) = Persistence::open(&directory).unwrap();
        assert_eq!(
            serde_json::to_value(loaded.settings).unwrap(),
            serde_json::to_value(settings).unwrap()
        );
        assert_eq!(loaded.tasks.len(), 1);
        assert_eq!(serde_json::to_value(&loaded.tasks[0]).unwrap(), expected);
        drop(store);
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn repeated_saves_preserve_latest_fields_without_duplicate_tasks() {
        let directory = directory();
        let (mut store, _) = Persistence::open(&directory).unwrap();
        let settings = Settings {
            download_dir: "C:/fixture-downloads".into(),
            density: "compact".into(),
            default_mode: "audio".into(),
            ..Settings::default()
        };
        let mut tasks = vec![task(TaskStatus::Completed), task(TaskStatus::Failed)];
        tasks[1].id = "second".into();
        for index in 0..12 {
            tasks[0].media.title = format!("第{index}次保存 · 长中文文件名");
            tasks[0].downloaded_bytes = 8_000_000_000 + index;
            tasks[0].logs = vec![format!("最新日志{index}")];
            tasks[1].error = Some(format!("错误{index}"));
            store.save(&settings, &tasks).unwrap();
        }
        drop(store);
        let (store, loaded) = Persistence::open(&directory).unwrap();
        let actual = loaded.tasks.iter().find(|t| t.id == "test").unwrap();
        assert_eq!(actual.downloaded_bytes, 8_000_000_011);
        assert_eq!(actual.media.title, tasks[0].media.title);
        assert_eq!(actual.logs, tasks[0].logs);
        assert_eq!(loaded.tasks.len(), 2);
        assert_eq!(
            loaded
                .tasks
                .iter()
                .find(|t| t.id == "second")
                .unwrap()
                .error,
            tasks[1].error
        );
        assert_eq!(loaded.settings.density, "compact");
        assert_eq!(loaded.settings.default_mode, "audio");
        drop(store);
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn every_active_status_interrupts_while_terminal_meanings_survive_restart() {
        let directory = directory();
        let (mut store, _) = Persistence::open(&directory).unwrap();
        let statuses = [
            TaskStatus::Queued,
            TaskStatus::Resolving,
            TaskStatus::Downloading,
            TaskStatus::Postprocessing,
            TaskStatus::Cancelling,
            TaskStatus::Completed,
            TaskStatus::Failed,
            TaskStatus::Cancelled,
            TaskStatus::Interrupted,
        ];
        let tasks = statuses
            .iter()
            .enumerate()
            .map(|(index, status)| {
                let mut task = task(*status);
                task.id = index.to_string();
                task.error = Some(format!("原错误{index}"));
                task
            })
            .collect::<Vec<_>>();
        store.save(&Settings::default(), &tasks).unwrap();
        drop(store);
        let (store, loaded) = Persistence::open(&directory).unwrap();
        assert_eq!(loaded.tasks.len(), statuses.len());
        for (index, status) in statuses.iter().enumerate() {
            let restored = loaded
                .tasks
                .iter()
                .find(|t| t.id == index.to_string())
                .unwrap();
            assert_eq!(
                restored.status,
                if status.active() {
                    TaskStatus::Interrupted
                } else {
                    *status
                }
            );
            if !status.active() {
                assert_eq!(restored.error, tasks[index].error);
            }
        }
        drop(store);
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn future_schema_is_refused_without_changing_database_bytes() {
        let directory = directory();
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("fluxdl.sqlite3");
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch("CREATE TABLE future_data(value TEXT); INSERT INTO future_data VALUES('keep me'); PRAGMA user_version=99;").unwrap();
        drop(connection);
        let before = std::fs::read(&path).unwrap();
        assert!(Repository::open(&directory).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 1);
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn unknown_rows_and_nested_fields_survive_migration_and_incremental_updates() {
        let directory = directory();
        let (mut store, _) = Repository::open(&directory).unwrap();
        store
            .save(&Settings::default(), &[task(TaskStatus::Completed)])
            .unwrap();
        drop(store);
        let path = directory.join("fluxdl.sqlite3");
        let connection = Connection::open(&path).unwrap();
        let mut raw_settings = serde_json::to_value(Settings::default()).unwrap();
        raw_settings["futureSettings"] = serde_json::json!({"preserve":[4,5,6]});
        connection
            .execute(
                "UPDATE app_settings SET document=?1 WHERE id=1",
                [raw_settings.to_string()],
            )
            .unwrap();
        let mut document = serde_json::to_value(task(TaskStatus::Completed)).unwrap();
        document["futureTop"] = serde_json::json!({"keep":[1,2,3]});
        document["media"]["futureMedia"] = serde_json::json!("keep");
        document["media"]["formats"] = serde_json::json!([{"formatId":"direct","ext":"mp4","label":"direct","height":360,"hasVideo":true,"hasAudio":true,"filesize":null,"requiresFfmpeg":false,"futureFormat":{"keep":true}}]);
        connection
            .execute(
                "UPDATE tasks SET document=?1 WHERE id='test'",
                [document.to_string()],
            )
            .unwrap();
        let opaque = [
            ("bad-json", "not valid json".to_string()),
            ("future-status", {
                let mut v = document.clone();
                v["id"] = serde_json::json!("future-status");
                v["status"] = serde_json::json!("future-phase");
                v.to_string()
            }),
            ("mismatch", document.to_string()),
        ];
        for (id, raw) in &opaque {
            connection
                .execute(
                    "INSERT INTO tasks(id,document) VALUES(?1,?2)",
                    params![id, raw],
                )
                .unwrap();
        }
        drop(connection);
        let (mut store, loaded) = Repository::open(&directory).unwrap();
        assert_eq!(loaded.tasks.len(), 1);
        assert_eq!(loaded.diagnostics.len(), 1);
        let mut updated = loaded.tasks[0].clone();
        updated.media.title = "updated".into();
        let seq = store.next_seq().unwrap();
        store
            .apply(&[Mutation::UpsertTask { seq, task: updated }])
            .unwrap();
        let seq = store.next_seq().unwrap();
        store
            .apply(&[Mutation::UpsertSettings {
                seq,
                settings: Settings {
                    theme: "light".into(),
                    ..loaded.settings
                },
            }])
            .unwrap();
        drop(store);
        let connection = Connection::open(&path).unwrap();
        let settings_raw: String = connection
            .query_row("SELECT document FROM app_settings WHERE id=1", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&settings_raw).unwrap()["futureSettings"],
            raw_settings["futureSettings"]
        );
        let raw: String = connection
            .query_row("SELECT document FROM tasks WHERE id='test'", [], |r| {
                r.get(0)
            })
            .unwrap();
        let updated: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(updated["futureTop"], document["futureTop"]);
        assert_eq!(updated["media"]["futureMedia"], "keep");
        assert_eq!(
            updated["media"]["formats"][0]["futureFormat"],
            document["media"]["formats"][0]["futureFormat"]
        );
        for (id, raw) in &opaque {
            assert_eq!(
                connection
                    .query_row("SELECT document FROM tasks WHERE id=?1", [id], |r| r
                        .get::<_, String>(0))
                    .unwrap(),
                *raw
            );
        }
        drop(connection);
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn revisions_terminal_progress_guard_and_tombstones_are_enforced() {
        let directory = directory();
        let (mut store, _) = Repository::open(&directory).unwrap();
        let completed = task(TaskStatus::Completed);
        assert_eq!(
            store
                .apply(&[Mutation::UpsertTask {
                    seq: 10,
                    task: completed.clone()
                }])
                .unwrap(),
            vec![ApplyOutcome::Applied]
        );
        assert_eq!(
            store
                .apply(&[Mutation::UpsertTask {
                    seq: 10,
                    task: completed.clone()
                }])
                .unwrap(),
            vec![ApplyOutcome::AlreadyCurrent]
        );
        let mut progress = completed.clone();
        progress.status = TaskStatus::Downloading;
        progress.downloaded_bytes = 99;
        assert_eq!(
            store
                .apply(&[Mutation::Progress {
                    seq: 20,
                    task: progress.clone()
                }])
                .unwrap(),
            vec![ApplyOutcome::Stale]
        );
        assert_eq!(
            store
                .apply(&[Mutation::UpsertTask {
                    seq: 9,
                    task: progress.clone()
                }])
                .unwrap(),
            vec![ApplyOutcome::Stale]
        );
        store
            .apply(&[Mutation::DeleteTask {
                seq: 30,
                task_id: "test".into(),
            }])
            .unwrap();
        assert_eq!(
            store
                .apply(&[Mutation::UpsertTask {
                    seq: 29,
                    task: progress
                }])
                .unwrap(),
            vec![ApplyOutcome::Stale]
        );
        drop(store);
        let (store, loaded) = Repository::open(&directory).unwrap();
        assert!(loaded.tasks.is_empty());
        assert!(loaded.max_mutation_seq >= 30);
        drop(store);
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn failed_batch_rolls_back_documents_and_version_guards() {
        let directory = directory();
        let (mut store, _) = Repository::open(&directory).unwrap();
        let original = task(TaskStatus::Completed);
        store
            .apply(&[Mutation::UpsertTask {
                seq: 1,
                task: original.clone(),
            }])
            .unwrap();
        let connection = Connection::open(directory.join("fluxdl.sqlite3")).unwrap();
        connection.execute_batch("CREATE TRIGGER reject_bad BEFORE INSERT ON tasks WHEN NEW.id='bad' BEGIN SELECT RAISE(ABORT,'fixture failure'); END;").unwrap();
        drop(connection);
        let mut changed = original.clone();
        changed.media.title = "must roll back".into();
        let mut bad = original.clone();
        bad.id = "bad".into();
        assert!(store
            .apply(&[
                Mutation::UpsertTask {
                    seq: 2,
                    task: changed
                },
                Mutation::UpsertTask { seq: 3, task: bad }
            ])
            .is_err());
        drop(store);
        let connection = Connection::open(directory.join("fluxdl.sqlite3")).unwrap();
        let raw: String = connection
            .query_row("SELECT document FROM tasks WHERE id='test'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(
            serde_json::from_str::<DownloadTask>(&raw)
                .unwrap()
                .media
                .title,
            original.media.title
        );
        assert_eq!(connection.query_row("SELECT revision FROM persistence_versions WHERE kind='task' AND record_id='test'",[],|r|r.get::<_,i64>(0)).unwrap(),1);
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM tasks", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        drop(connection);
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn failed_legacy_migration_keeps_version_and_rows() {
        let directory = directory();
        std::fs::create_dir_all(&directory).unwrap();
        let connection = Connection::open(directory.join("fluxdl.sqlite3")).unwrap();
        connection.execute_batch("CREATE TABLE app_settings(id INTEGER PRIMARY KEY,document TEXT NOT NULL);CREATE TABLE tasks(id TEXT PRIMARY KEY,document TEXT NOT NULL);CREATE TABLE persistence_versions(conflict TEXT);INSERT INTO tasks VALUES('opaque','keep raw');PRAGMA user_version=1;").unwrap();
        drop(connection);
        assert!(Repository::open(&directory).is_err());
        let connection = Connection::open(directory.join("fluxdl.sqlite3")).unwrap();
        assert_eq!(
            connection
                .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            connection
                .query_row("SELECT document FROM tasks WHERE id='opaque'", [], |r| {
                    r.get::<_, String>(0)
                })
                .unwrap(),
            "keep raw"
        );
        drop(connection);
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 1);
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn named_columns_without_primary_keys_are_rejected_without_writes() {
        for version in [1, 2] {
            let directory = directory();
            std::fs::create_dir_all(&directory).unwrap();
            let path = directory.join("fluxdl.sqlite3");
            let connection = Connection::open(&path).unwrap();
            if version == 1 {
                connection.execute_batch("CREATE TABLE app_settings(id INTEGER,document TEXT);CREATE TABLE tasks(id TEXT,document TEXT);").unwrap();
            } else {
                connection.execute_batch("CREATE TABLE app_settings(id INTEGER PRIMARY KEY,document TEXT);CREATE TABLE tasks(id TEXT PRIMARY KEY,document TEXT);").unwrap();
            }
            connection.execute_batch("INSERT INTO tasks VALUES('keep','raw');CREATE TABLE persistence_versions(kind TEXT,record_id TEXT,revision INTEGER,deleted INTEGER);").unwrap();
            connection
                .pragma_update(None, "user_version", version)
                .unwrap();
            drop(connection);
            let before = std::fs::read(&path).unwrap();
            assert!(Repository::open(&directory).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), before);
            std::fs::remove_dir_all(directory).unwrap();
        }
    }
}
