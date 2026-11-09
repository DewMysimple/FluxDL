use crate::models::{DownloadTask, Settings, TaskStatus};
use rusqlite::{params, Connection};
use std::path::Path;

pub struct Persistence {
    connection: Connection,
}
pub struct Loaded {
    pub settings: Settings,
    pub tasks: Vec<DownloadTask>,
    pub recovery_warning: Option<String>,
}

impl Persistence {
    pub fn open(directory: &Path) -> Result<(Self, Loaded), String> {
        std::fs::create_dir_all(directory).map_err(|e| format!("无法创建应用数据目录：{e}"))?;
        let path = directory.join("fluxdl.sqlite3");
        match Self::load(&path) {
            Ok(value) => Ok(value),
            Err(error) => {
                // Preserve the original database before recovering. Never silently discard history.
                if path.is_file() && error.contains("数据库内容损坏") {
                    let backup = directory.join(format!(
                        "fluxdl-corrupt-{}.sqlite3",
                        chrono::Utc::now().format("%Y%m%d-%H%M%S-%f")
                    ));
                    std::fs::rename(&path, &backup)
                        .map_err(|e| format!("数据库损坏且无法备份：{e}。原文件未删除。"))?;
                    let (store, mut loaded) = Self::load(&path)?;
                    loaded.recovery_warning = Some(format!(
                        "数据库内容损坏，已安全备份到 {} 并建立新数据库。",
                        backup.display()
                    ));
                    Ok((store, loaded))
                } else {
                    Err(error)
                }
            }
        }
    }

    fn load(path: &Path) -> Result<(Self, Loaded), String> {
        let connection = Connection::open(path).map_err(|e| format!("无法打开任务数据库：{e}"))?;
        connection
            .busy_timeout(std::time::Duration::from_secs(5))
            .map_err(db_error)?;
        let check: String = connection
            .query_row("PRAGMA quick_check", [], |r| r.get(0))
            .map_err(db_error)?;
        if check != "ok" {
            return Err(format!("数据库内容损坏：{check}"));
        }
        connection.execute_batch("PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL; CREATE TABLE IF NOT EXISTS app_settings (id INTEGER PRIMARY KEY CHECK(id=1), document TEXT NOT NULL); CREATE TABLE IF NOT EXISTS tasks (id TEXT PRIMARY KEY, document TEXT NOT NULL); PRAGMA user_version=1;").map_err(db_error)?;
        let settings_doc =
            connection.query_row("SELECT document FROM app_settings WHERE id=1", [], |r| {
                r.get::<_, String>(0)
            });
        let settings = match settings_doc {
            Ok(doc) => serde_json::from_str(&doc)
                .map_err(|e| format!("数据库内容损坏：设置无法读取：{e}"))?,
            Err(rusqlite::Error::QueryReturnedNoRows) => Settings::default(),
            Err(e) => return Err(db_error(e)),
        };
        let mut tasks = Vec::new();
        {
            let mut statement = connection
                .prepare("SELECT document FROM tasks")
                .map_err(db_error)?;
            for row in statement
                .query_map([], |r| r.get::<_, String>(0))
                .map_err(db_error)?
            {
                let mut task: DownloadTask = serde_json::from_str(&row.map_err(db_error)?)
                    .map_err(|e| format!("数据库内容损坏：任务无法读取：{e}"))?;
                if task.status.active() {
                    task.status = TaskStatus::Interrupted;
                    task.error =
                        Some("上次运行尚未结束。请点击重试，FluxDL 不会自动恢复下载。".into());
                    task.updated_at = chrono::Utc::now().to_rfc3339();
                }
                task.file_exists = task
                    .final_path
                    .as_deref()
                    .is_some_and(|p| Path::new(p).is_file());
                tasks.push(task);
            }
        }
        tasks.sort_by(|a, b| {
            a.created_at
                .cmp(&b.created_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        let mut store = Self { connection };
        store.save(&settings, &tasks)?;
        Ok((
            store,
            Loaded {
                settings,
                tasks,
                recovery_warning: None,
            },
        ))
    }

    pub fn save(&mut self, settings: &Settings, tasks: &[DownloadTask]) -> Result<(), String> {
        let tx = self.connection.transaction().map_err(db_error)?;
        let settings_json = serde_json::to_string(settings).map_err(|e| e.to_string())?;
        tx.execute("INSERT INTO app_settings(id,document) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET document=excluded.document", [settings_json]).map_err(db_error)?;
        tx.execute("DELETE FROM tasks", []).map_err(db_error)?;
        for task in tasks {
            tx.execute(
                "INSERT INTO tasks(id,document) VALUES(?1,?2)",
                params![
                    task.id,
                    serde_json::to_string(task).map_err(|e| e.to_string())?
                ],
            )
            .map_err(db_error)?;
        }
        tx.commit().map_err(db_error)
    }
}

fn db_error(error: rusqlite::Error) -> String {
    match error {
        rusqlite::Error::SqliteFailure(ref detail, _)
            if matches!(
                detail.code,
                rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase
            ) =>
        {
            format!("数据库内容损坏：{error}")
        }
        _ => format!("任务数据库读写失败：{error}。请检查应用数据目录权限及磁盘空间。"),
    }
}

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
        drop(store);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
