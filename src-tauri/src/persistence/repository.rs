use crate::{
    errors::{AppError, AppResult, ErrorCode, ErrorPhase},
    models::{DownloadTask, Settings},
};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde_json::Value;
use std::{collections::HashSet, path::Path};

pub struct Repository {
    connection: Connection,
    max_seq: i64,
    opaque_ids: HashSet<String>,
}
pub struct Loaded {
    pub settings: Settings,
    pub tasks: Vec<DownloadTask>,
    pub recovery_warning: Option<String>,
    pub diagnostics: Vec<String>,
    pub max_mutation_seq: i64,
}
#[derive(Clone, Debug)]
pub enum Mutation {
    UpsertTask { seq: i64, task: DownloadTask },
    Progress { seq: i64, task: DownloadTask },
    DeleteTask { seq: i64, task_id: String },
    UpsertSettings { seq: i64, settings: Settings },
}
impl Mutation {
    pub fn seq(&self) -> i64 {
        match self {
            Self::UpsertTask { seq, .. }
            | Self::Progress { seq, .. }
            | Self::DeleteTask { seq, .. }
            | Self::UpsertSettings { seq, .. } => *seq,
        }
    }
    fn key(&self) -> (&str, &str) {
        match self {
            Self::UpsertTask { task, .. } | Self::Progress { task, .. } => ("task", &task.id),
            Self::DeleteTask { task_id, .. } => ("task", task_id),
            Self::UpsertSettings { .. } => ("settings", "1"),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplyOutcome {
    Applied,
    AlreadyCurrent,
    Stale,
}
pub(super) enum Verification {
    Confirmed(Vec<ApplyOutcome>),
    NotCommitted,
    Unknown(AppError),
}

impl Repository {
    pub(super) fn verify(directory: &Path, changes: &[Mutation]) -> Verification {
        let query = || -> AppResult<Verification> {
            let connection = Connection::open_with_flags(
                directory.join("fluxdl.sqlite3"),
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )
            .map_err(db_error)?;
            connection
                .busy_timeout(std::time::Duration::from_secs(5))
                .map_err(db_error)?;
            let mut outcomes = Vec::with_capacity(changes.len());
            for change in changes {
                let (kind, id) = change.key();
                let revision=connection.query_row("SELECT revision,deleted FROM persistence_versions WHERE kind=?1 AND record_id=?2",params![kind,id],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,bool>(1)?))).optional().map_err(db_error)?;
                match revision {
                    Some((seq, _)) if seq > change.seq() => {
                        outcomes.push(ApplyOutcome::Stale);
                        continue;
                    }
                    Some((seq, deleted)) if seq == change.seq() => {
                        if matches!(change, Mutation::DeleteTask { .. }) && deleted {
                            outcomes.push(ApplyOutcome::AlreadyCurrent);
                            continue;
                        }
                        if deleted {
                            return Ok(Verification::NotCommitted);
                        }
                        let raw: String = if kind == "task" {
                            connection
                                .query_row("SELECT document FROM tasks WHERE id=?1", [id], |r| {
                                    r.get(0)
                                })
                                .map_err(db_error)?
                        } else {
                            connection
                                .query_row(
                                    "SELECT document FROM app_settings WHERE id=1",
                                    [],
                                    |r| r.get(0),
                                )
                                .map_err(db_error)?
                        };
                        let matches = match change {
                            Mutation::UpsertTask { task, .. } | Mutation::Progress { task, .. } => {
                                decode_task(&raw, id)
                                    .ok()
                                    .and_then(|stored| serde_json::to_value(stored).ok())
                                    == serde_json::to_value(task).ok()
                            }
                            Mutation::UpsertSettings { settings, .. } => {
                                serde_json::from_str::<Settings>(&raw)
                                    .ok()
                                    .and_then(|stored| serde_json::to_value(stored).ok())
                                    == serde_json::to_value(settings).ok()
                            }
                            Mutation::DeleteTask { .. } => false,
                        };
                        if !matches {
                            return Ok(Verification::Unknown(persist_error(
                                "已提交版本内容与候选不一致，保留待核对状态。",
                            )));
                        }
                        outcomes.push(ApplyOutcome::AlreadyCurrent);
                    }
                    _ => return Ok(Verification::NotCommitted),
                }
            }
            Ok(Verification::Confirmed(outcomes))
        };
        query().unwrap_or_else(Verification::Unknown)
    }
    pub fn open(directory: &Path) -> AppResult<(Self, Loaded)> {
        std::fs::create_dir_all(directory).map_err(|error| {
            persist_error("无法创建应用数据目录。").with_details(error.to_string())
        })?;
        let path = directory.join("fluxdl.sqlite3");
        match Self::load(&path) {
            Ok(result) => Ok(result),
            Err(OpenError::Physical(error)) => {
                let backup = directory.join(format!(
                    "fluxdl-corrupt-{}.sqlite3",
                    chrono::Utc::now().format("%Y%m%d-%H%M%S-%f")
                ));
                std::fs::rename(&path, &backup).map_err(|error| {
                    persist_error("数据库物理损坏且无法备份，原文件未覆盖。")
                        .with_details(error.to_string())
                })?;
                let (store, mut loaded) = Self::load(&path).map_err(OpenError::message)?;
                loaded.recovery_warning = Some(format!(
                    "数据库物理损坏（{error}），已字节保全备份到 {} 并建立新数据库。",
                    backup.display()
                ));
                Ok((store, loaded))
            }
            Err(error) => Err(error.message()),
        }
    }
    fn load(path: &Path) -> Result<(Self, Loaded), OpenError> {
        let mut connection = Connection::open(path).map_err(|e| OpenError::Other(db_error(e)))?;
        connection
            .busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|e| OpenError::Other(db_error(e)))?;
        match connection.query_row("PRAGMA quick_check", [], |r| r.get::<_, String>(0)) {
            Ok(value) if value == "ok" => {}
            Ok(_) => return Err(OpenError::Physical("quick_check 未通过".into())),
            Err(rusqlite::Error::SqliteFailure(detail, _))
                if matches!(
                    detail.code,
                    rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase
                ) =>
            {
                return Err(OpenError::Physical("quick_check 确认文件损坏".into()))
            }
            Err(error) => return Err(OpenError::Other(db_error(error))),
        }
        migrate(&mut connection).map_err(OpenError::Other)?;
        connection
            .execute_batch("PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL;")
            .map_err(|e| OpenError::Other(db_error(e)))?;
        let settings_doc = connection
            .query_row("SELECT document FROM app_settings WHERE id=1", [], |r| {
                r.get::<_, String>(0)
            })
            .optional()
            .map_err(|e| OpenError::Other(db_error(e)))?;
        let settings = match settings_doc {
            Some(document) => serde_json::from_str(&document).map_err(|_| {
                OpenError::Other(persist_error(
                    "设置文档无法安全解释，原数据已保留；没有重置数据库。",
                ))
            })?,
            None => Settings::default(),
        };
        let max_seq = connection
            .query_row(
                "SELECT COALESCE(MAX(revision),0) FROM persistence_versions",
                [],
                |r| r.get(0),
            )
            .map_err(|e| OpenError::Other(db_error(e)))?;
        let mut tasks = Vec::new();
        let mut opaque_ids = HashSet::new();
        let mut recoveries = Vec::new();
        {
            let mut statement = connection
                .prepare("SELECT id,document FROM tasks")
                .map_err(|e| OpenError::Other(db_error(e)))?;
            let rows = statement
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                .map_err(|e| OpenError::Other(db_error(e)))?;
            for row in rows {
                let (id, document) = row.map_err(|e| OpenError::Other(db_error(e)))?;
                match decode_task(&document, &id) {
                    Ok(mut task) => {
                        if crate::transitions::recover(&mut task) {
                            task.updated_at = chrono::Utc::now().to_rfc3339();
                            recoveries.push(task.clone());
                        }
                        // Runtime availability is refreshed by explicit snapshots/details, never by SQL loading.
                        task.file_exists = false;
                        sanitize_task_diagnostics(&mut task);
                        tasks.push(task);
                    }
                    Err(_) => {
                        opaque_ids.insert(id);
                    }
                }
            }
        }
        tasks.sort_by(|a, b| {
            a.created_at
                .cmp(&b.created_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        let diagnostics = if opaque_ids.is_empty() {
            vec![]
        } else {
            vec![format!(
                "有 {} 条任务记录无法安全解释，原始文档已保留，其他记录仍可使用。",
                opaque_ids.len()
            )]
        };
        let mut repository = Self {
            connection,
            max_seq,
            opaque_ids,
        };
        let mut changes = Vec::new();
        for task in recoveries {
            let seq = repository.next_seq().map_err(OpenError::Other)?;
            changes.push(Mutation::UpsertTask { seq, task });
        }
        if !changes.is_empty() {
            repository.apply(&changes).map_err(OpenError::Other)?;
        }
        let loaded = Loaded {
            settings,
            tasks,
            recovery_warning: None,
            diagnostics,
            max_mutation_seq: repository.max_seq,
        };
        Ok((repository, loaded))
    }
    pub fn next_seq(&mut self) -> AppResult<i64> {
        self.max_seq = self
            .max_seq
            .checked_add(1)
            .ok_or_else(|| persist_error("持久化版本号已达到安全上限。"))?;
        Ok(self.max_seq)
    }
    pub fn apply(&mut self, changes: &[Mutation]) -> AppResult<Vec<ApplyOutcome>> {
        let tx = self.connection.transaction().map_err(db_error)?;
        let mut results = Vec::with_capacity(changes.len());
        for change in changes {
            if change.seq() <= 0 {
                return Err(persist_error("持久化版本号无效。"));
            }
            let (kind, id) = change.key();
            if kind == "task" && self.opaque_ids.contains(id) {
                return Err(persist_error(
                    "该记录无法安全解释，已保留原始文档并阻止覆盖。",
                ));
            }
            let previous=tx.query_row("SELECT revision,deleted FROM persistence_versions WHERE kind=?1 AND record_id=?2",params![kind,id],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,bool>(1)?))).optional().map_err(db_error)?;
            if previous.is_some_and(|(revision, _)| revision > change.seq()) {
                results.push(ApplyOutcome::Stale);
                continue;
            }
            let (document, delete) = match change {
                Mutation::DeleteTask { .. } => (None, true),
                Mutation::UpsertSettings { settings, .. } => {
                    let old = tx
                        .query_row("SELECT document FROM app_settings WHERE id=1", [], |r| {
                            r.get::<_, String>(0)
                        })
                        .optional()
                        .map_err(db_error)?;
                    (
                        Some(merge_document(
                            old.as_deref(),
                            serde_json::to_value(settings).map_err(json_error)?,
                        )?),
                        false,
                    )
                }
                Mutation::UpsertTask { task, .. } | Mutation::Progress { task, .. } => {
                    let old = tx
                        .query_row("SELECT document FROM tasks WHERE id=?1", [id], |r| {
                            r.get::<_, String>(0)
                        })
                        .optional()
                        .map_err(db_error)?;
                    if let Some(raw) = &old {
                        let old_task = decode_task(raw, id)
                            .map_err(|_| persist_error("任务原始文档无法安全更新，已保留。"))?;
                        if matches!(change, Mutation::Progress { .. }) && !old_task.status.active()
                        {
                            results.push(ApplyOutcome::Stale);
                            continue;
                        }
                    }
                    // A tombstone is permanent for this task ID; retries retain an existing row.
                    if previous.is_some_and(|(_, deleted)| deleted) {
                        results.push(ApplyOutcome::Stale);
                        continue;
                    }
                    (
                        Some(merge_document(
                            old.as_deref(),
                            serde_json::to_value(task).map_err(json_error)?,
                        )?),
                        false,
                    )
                }
            };
            if previous.is_some_and(|(revision, _)| revision == change.seq()) {
                if delete && previous.is_some_and(|(_, deleted)| deleted) {
                    results.push(ApplyOutcome::AlreadyCurrent);
                    continue;
                }
                let existing = read_document(&tx, kind, id)?;
                if !delete
                    && existing
                        .as_deref()
                        .and_then(|v| serde_json::from_str::<Value>(v).ok())
                        == document
                            .as_deref()
                            .and_then(|v| serde_json::from_str::<Value>(v).ok())
                {
                    results.push(ApplyOutcome::AlreadyCurrent);
                    continue;
                }
                return Err(persist_error("相同持久化版本出现冲突内容，事务已回滚。"));
            }
            match change {
                Mutation::DeleteTask { .. } => {
                    tx.execute("DELETE FROM tasks WHERE id=?1", [id])
                        .map_err(db_error)?;
                }
                Mutation::UpsertSettings { .. } => {
                    tx.execute("INSERT INTO app_settings(id,document) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET document=excluded.document",[document.as_deref().unwrap()]).map_err(db_error)?;
                }
                _ => {
                    tx.execute("INSERT INTO tasks(id,document) VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET document=excluded.document",params![id,document.as_deref().unwrap()]).map_err(db_error)?;
                }
            }
            tx.execute("INSERT INTO persistence_versions(kind,record_id,revision,deleted) VALUES(?1,?2,?3,?4) ON CONFLICT(kind,record_id) DO UPDATE SET revision=excluded.revision,deleted=excluded.deleted",params![kind,id,change.seq(),delete]).map_err(db_error)?;
            results.push(ApplyOutcome::Applied);
        }
        tx.commit().map_err(db_error)?;
        if let Some(seq) = changes.iter().map(Mutation::seq).max() {
            self.max_seq = self.max_seq.max(seq);
        }
        Ok(results)
    }
    /// Compatibility importer for fixtures. Production uses explicit per-record mutations.
    pub fn save(&mut self, settings: &Settings, tasks: &[DownloadTask]) -> AppResult<()> {
        let mut changes = Vec::with_capacity(tasks.len() + 1);
        changes.push(Mutation::UpsertSettings {
            seq: self.next_seq()?,
            settings: settings.clone(),
        });
        for task in tasks {
            changes.push(Mutation::UpsertTask {
                seq: self.next_seq()?,
                task: task.clone(),
            });
        }
        self.apply(&changes)?;
        Ok(())
    }
}

fn migrate(connection: &mut Connection) -> AppResult<()> {
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(db_error)?;
    if !(0..=2).contains(&version) {
        return Err(persist_error(format!(
            "数据库版本 {version} 不受本版本支持，原文件未更改。"
        )));
    }
    let names: HashSet<String> = {
        let mut s = connection
            .prepare(
                "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",
            )
            .map_err(db_error)?;
        let names = s
            .query_map([], |r| r.get(0))
            .map_err(db_error)?
            .collect::<Result<_, _>>()
            .map_err(db_error)?;
        names
    };
    let empty = names.is_empty();
    if !empty {
        for table in ["tasks", "app_settings"] {
            if !names.contains(table) {
                return Err(persist_error("数据库缺少必需表，原文件未重置。"));
            }
            require_columns(connection, table, &["id", "document"])?;
            require_primary_key(connection, table, &["id"])?;
        }
    }
    if version == 2 {
        require_columns(
            connection,
            "persistence_versions",
            &["kind", "record_id", "revision", "deleted"],
        )?;
        require_primary_key(connection, "persistence_versions", &["kind", "record_id"])?;
        return Ok(());
    }
    let tx = connection.transaction().map_err(db_error)?;
    if empty {
        tx.execute_batch("CREATE TABLE app_settings(id INTEGER PRIMARY KEY CHECK(id=1),document TEXT NOT NULL); CREATE TABLE tasks(id TEXT PRIMARY KEY,document TEXT NOT NULL);").map_err(db_error)?;
    }
    tx.execute_batch("CREATE TABLE persistence_versions(kind TEXT NOT NULL,record_id TEXT NOT NULL,revision INTEGER NOT NULL,deleted INTEGER NOT NULL DEFAULT 0,PRIMARY KEY(kind,record_id)); INSERT INTO persistence_versions SELECT 'task',id,0,0 FROM tasks; INSERT INTO persistence_versions SELECT 'settings','1',0,0 FROM app_settings WHERE id=1; PRAGMA user_version=2;").map_err(db_error)?;
    tx.commit().map_err(db_error)
}
fn require_columns(connection: &Connection, table: &str, expected: &[&str]) -> AppResult<()> {
    // table is one of the three internal literals, never an IPC value.
    let mut statement = connection
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(db_error)?;
    let columns = statement
        .query_map([], |r| r.get::<_, String>(1))
        .map_err(db_error)?
        .collect::<Result<HashSet<_>, _>>()
        .map_err(db_error)?;
    if expected.iter().any(|name| !columns.contains(*name)) {
        return Err(persist_error("数据库表结构不兼容，原文件未重置。"));
    }
    Ok(())
}
fn require_primary_key(connection: &Connection, table: &str, expected: &[&str]) -> AppResult<()> {
    let mut statement = connection
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(db_error)?;
    let mut keys = statement
        .query_map([], |r| Ok((r.get::<_, i64>(5)?, r.get::<_, String>(1)?)))
        .map_err(db_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_error)?;
    keys.retain(|(position, _)| *position > 0);
    keys.sort_by_key(|(position, _)| *position);
    if keys
        .iter()
        .map(|(_, name)| name.as_str())
        .collect::<Vec<_>>()
        != expected
    {
        return Err(persist_error("数据库缺少必需主键约束，原文件未更改。"));
    }
    Ok(())
}
fn decode_task(document: &str, id: &str) -> Result<DownloadTask, ()> {
    let mut value: Value = serde_json::from_str(document).map_err(|_| ())?;
    if let Some(object) = value.as_object_mut() {
        object
            .entry("logs")
            .or_insert_with(|| serde_json::json!([]));
        object.entry("fileExists").or_insert(Value::Bool(false));
        object
            .entry("downloadedBytes")
            .or_insert_with(|| serde_json::json!(0));
    }
    if let Some(media) = value.get_mut("media").and_then(Value::as_object_mut) {
        media
            .entry("warnings")
            .or_insert_with(|| serde_json::json!([]));
        media
            .entry("formats")
            .or_insert_with(|| serde_json::json!([]));
    }
    let task: DownloadTask = serde_json::from_value(value).map_err(|_| ())?;
    if task.id != id {
        return Err(());
    }
    Ok(task)
}

// Legacy documents may predate the common display filter. Clean only diagnostic
// fields in the loaded memory copy; URLs, titles and raw SQLite rows retain their
// original meaning. Stateful filtering also covers a header folded across logs.
fn sanitize_task_diagnostics(task: &mut DownloadTask) {
    let mut logs = crate::diagnostics::LogRedactor::default();
    for line in &mut task.logs {
        *line = logs.redact(line);
    }
    if task.logs.len() > 100 {
        task.logs.drain(..task.logs.len() - 100);
    }
    task.logs
        .iter_mut()
        .for_each(|line| *line = line.chars().take(1600).collect());
    task.error = task
        .error
        .as_ref()
        .map(|error| crate::diagnostics::redact_log(error));
    let mut warnings = crate::diagnostics::LogRedactor::default();
    for warning in &mut task.media.warnings {
        *warning = warnings.redact(warning);
    }
}
fn read_document(tx: &Transaction<'_>, kind: &str, id: &str) -> AppResult<Option<String>> {
    if kind == "task" {
        tx.query_row("SELECT document FROM tasks WHERE id=?1", [id], |r| r.get(0))
            .optional()
            .map_err(db_error)
    } else {
        tx.query_row("SELECT document FROM app_settings WHERE id=1", [], |r| {
            r.get(0)
        })
        .optional()
        .map_err(db_error)
    }
}
fn merge_document(previous: Option<&str>, fresh: Value) -> AppResult<String> {
    let mut previous = match previous {
        Some(raw) => serde_json::from_str(raw)
            .map_err(|_| persist_error("原始文档无法安全合并，未覆盖。"))?,
        None => serde_json::json!({}),
    };
    if !previous.is_object() {
        return Err(persist_error("原始文档不是对象，未覆盖。"));
    }
    merge_value(&mut previous, fresh, "");
    serde_json::to_string(&previous).map_err(json_error)
}
fn merge_value(previous: &mut Value, fresh: Value, key: &str) {
    if let (Some(old), Value::Object(new)) = (previous.as_object_mut(), &fresh) {
        for (k, v) in new {
            merge_value(old.entry(k.clone()).or_insert(Value::Null), v.clone(), k);
        }
        return;
    }
    if key == "formats" {
        if let (Some(old), Value::Array(new)) = (previous.as_array_mut(), &fresh) {
            for item in new {
                if let Some(existing) = old
                    .iter_mut()
                    .find(|entry| entry.get("formatId") == item.get("formatId"))
                {
                    merge_value(existing, item.clone(), "");
                } else {
                    old.push(item.clone());
                }
            }
            return;
        }
    }
    *previous = fresh;
}
enum OpenError {
    Physical(String),
    Other(AppError),
}
impl OpenError {
    fn message(self) -> AppError {
        match self {
            Self::Physical(message) => persist_error(message),
            Self::Other(error) => error,
        }
    }
}
fn persist_error(message: impl AsRef<str>) -> AppError {
    AppError::new(ErrorCode::PersistenceRejected, ErrorPhase::Persist, message)
}
fn db_error(error: rusqlite::Error) -> AppError {
    persist_error("任务数据库读写失败，请检查权限、磁盘空间或数据库占用；原数据未重置。")
        .with_details(error.to_string())
}
fn json_error(error: serde_json::Error) -> AppError {
    persist_error("任务数据库文档无法安全序列化，原数据未覆盖。").with_details(error.to_string())
}
