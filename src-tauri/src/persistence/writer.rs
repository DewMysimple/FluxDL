use super::repository::Verification;
use super::{ApplyOutcome, Loaded, Mutation, Repository};
use crate::errors::{AppError, AppResult, ErrorCode, ErrorPhase};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use tokio::sync::{mpsc, oneshot};

enum Request {
    Apply(Vec<Mutation>, oneshot::Sender<AppResult<Vec<ApplyOutcome>>>),
    Flush(oneshot::Sender<()>),
    Close(oneshot::Sender<()>),
}
#[derive(Debug, Clone)]
pub enum WriteFailure {
    Rejected(AppError),
    Unconfirmed(AppError),
}
impl std::fmt::Display for WriteFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Rejected(error) | Self::Unconfirmed(error) => write!(f, "{error}"),
        }
    }
}
impl From<WriteFailure> for AppError {
    fn from(value: WriteFailure) -> Self {
        match value {
            WriteFailure::Rejected(error) | WriteFailure::Unconfirmed(error) => error,
        }
    }
}
#[derive(Clone)]
pub struct PersistenceWriter {
    // Drop order matters: disconnect all senders before the final join guard.
    sender: mpsc::Sender<Request>,
    closed: Arc<AtomicBool>,
    directory: PathBuf,
    #[cfg(test)]
    lose_ack: Arc<AtomicBool>,
    _thread: Arc<ThreadGuard>,
}
struct ThreadGuard(Mutex<Option<std::thread::JoinHandle<()>>>);
impl Drop for ThreadGuard {
    fn drop(&mut self) {
        if let Some(handle) = self.0.get_mut().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = handle.join();
        }
    }
}
impl PersistenceWriter {
    pub async fn open(directory: PathBuf) -> AppResult<(Self, Loaded)> {
        let (sender, mut receiver) = mpsc::channel::<Request>(64);
        let (ready_sender, ready_receiver) = oneshot::channel();
        let closed = Arc::new(AtomicBool::new(false));
        let worker_closed = closed.clone();
        let worker_directory = directory.clone();
        #[cfg(test)]
        let lose_ack = Arc::new(AtomicBool::new(false));
        #[cfg(test)]
        let worker_lose_ack = lose_ack.clone();
        let handle = std::thread::Builder::new()
            .name("fluxdl-sqlite".into())
            .spawn(move || {
                let (mut repository, loaded) = match Repository::open(&worker_directory) {
                    Ok(value) => value,
                    Err(error) => {
                        let _ = ready_sender.send(Err(error));
                        worker_closed.store(true, Ordering::SeqCst);
                        return;
                    }
                };
                if ready_sender.send(Ok(loaded)).is_err() {
                    return;
                }
                while let Some(request) = receiver.blocking_recv() {
                    match request {
                        Request::Apply(changes, ack) => {
                            let result = repository.apply(&changes);
                            #[cfg(test)]
                            if worker_lose_ack.swap(false, Ordering::SeqCst) {
                                drop(ack);
                                continue;
                            }
                            let _ = ack.send(result);
                        }
                        // A flush proves ordered processing. Every failed apply still has
                        // its own failure ack; Manager must retain/retry critical candidates.
                        Request::Flush(ack) => {
                            let _ = ack.send(());
                        }
                        Request::Close(ack) => {
                            drop(repository);
                            worker_closed.store(true, Ordering::SeqCst);
                            let _ = ack.send(());
                            return;
                        }
                    }
                }
                worker_closed.store(true, Ordering::SeqCst);
            })
            .map_err(|error| {
                rejected("无法建立数据库写入线程。").with_details(error.to_string())
            })?;
        let loaded = ready_receiver
            .await
            .map_err(|_| rejected("数据库写入线程在启动时停止。"))??;
        Ok((
            Self {
                sender,
                closed,
                directory,
                #[cfg(test)]
                lose_ack,
                _thread: Arc::new(ThreadGuard(Mutex::new(Some(handle)))),
            },
            loaded,
        ))
    }
    pub async fn apply(&self, changes: Vec<Mutation>) -> AppResult<Vec<ApplyOutcome>> {
        self.apply_checked(changes).await.map_err(Into::into)
    }
    pub async fn apply_checked(
        &self,
        changes: Vec<Mutation>,
    ) -> Result<Vec<ApplyOutcome>, WriteFailure> {
        let (ack, reply) = oneshot::channel();
        self.sender
            .send(Request::Apply(changes.clone(), ack))
            .await
            .map_err(|_| {
                WriteFailure::Rejected(rejected("数据库写入线程已停止，未接受本次变更。"))
            })?;
        match reply.await {
            Ok(result) => result.map_err(WriteFailure::Rejected),
            Err(_) => self.reconcile(changes).await,
        }
    }
    pub async fn reconcile(
        &self,
        changes: Vec<Mutation>,
    ) -> Result<Vec<ApplyOutcome>, WriteFailure> {
        let directory = self.directory.clone();
        match tokio::task::spawn_blocking(move || Repository::verify(&directory, &changes)).await {
            Ok(Verification::Confirmed(outcomes)) => Ok(outcomes),
            Ok(Verification::NotCommitted) => Err(WriteFailure::Rejected(rejected(
                "只读核对确认该变更未提交，原有状态保留。",
            ))),
            Ok(Verification::Unknown(error)) => Err(WriteFailure::Unconfirmed(
                unconfirmed("数据库回执丢失且尚不能核实提交结果。").with_details(error.safe_text()),
            )),
            Err(error) => Err(WriteFailure::Unconfirmed(
                unconfirmed("数据库核对任务未完成。").with_details(error.to_string()),
            )),
        }
    }
    #[cfg(test)]
    pub(crate) fn lose_next_ack(&self) {
        self.lose_ack.store(true, Ordering::SeqCst);
    }
    pub async fn flush(&self) -> AppResult<()> {
        let (ack, reply) = oneshot::channel();
        self.sender
            .send(Request::Flush(ack))
            .await
            .map_err(|_| rejected("数据库线程无法刷新：线程已停止。"))?;
        reply
            .await
            .map_err(|_| unconfirmed("数据库刷新未获得确认。"))
    }
    pub async fn close(&self) -> AppResult<()> {
        if self.closed.load(Ordering::SeqCst) {
            return Ok(());
        }
        let (ack, reply) = oneshot::channel();
        self.sender
            .send(Request::Close(ack))
            .await
            .map_err(|_| rejected("数据库线程无法关闭：连接状态未确认。"))?;
        reply
            .await
            .map_err(|_| unconfirmed("数据库关闭未获得确认。"))
    }
}

fn rejected(message: &str) -> AppError {
    AppError::new(ErrorCode::PersistenceRejected, ErrorPhase::Persist, message)
}
fn unconfirmed(message: &str) -> AppError {
    AppError::new(
        ErrorCode::PersistenceUnconfirmed,
        ErrorPhase::Persist,
        message,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Settings;
    fn directory() -> PathBuf {
        std::env::temp_dir().join(format!("fluxdl-writer-test-{}", uuid::Uuid::new_v4()))
    }
    #[tokio::test]
    async fn single_owner_ack_flush_close_and_restart() {
        let directory = directory();
        let (writer, _) = PersistenceWriter::open(directory.clone()).await.unwrap();
        let settings = Settings {
            theme: "light".into(),
            ..Settings::default()
        };
        assert_eq!(
            writer
                .apply(vec![Mutation::UpsertSettings {
                    seq: 1,
                    settings: settings.clone()
                }])
                .await
                .unwrap(),
            vec![ApplyOutcome::Applied]
        );
        let second = writer.clone();
        assert_eq!(
            second
                .apply(vec![Mutation::UpsertSettings { seq: 1, settings }])
                .await
                .unwrap(),
            vec![ApplyOutcome::AlreadyCurrent]
        );
        writer.flush().await.unwrap();
        writer.close().await.unwrap();
        writer.close().await.unwrap();
        assert!(writer.flush().await.is_err());
        let (writer, loaded) = PersistenceWriter::open(directory.clone()).await.unwrap();
        assert_eq!(loaded.settings.theme, "light");
        assert_eq!(loaded.max_mutation_seq, 1);
        writer.close().await.unwrap();
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[tokio::test]
    async fn sql_failure_has_error_ack_and_can_be_retried_after_repair() {
        let directory = directory();
        let (writer, _) = PersistenceWriter::open(directory.clone()).await.unwrap();
        let connection = rusqlite::Connection::open(directory.join("fluxdl.sqlite3")).unwrap();
        connection.execute_batch("CREATE TRIGGER reject_settings BEFORE INSERT ON app_settings BEGIN SELECT RAISE(ABORT,'fixture failure'); END;").unwrap();
        let change = Mutation::UpsertSettings {
            seq: 1,
            settings: Settings {
                theme: "light".into(),
                ..Settings::default()
            },
        };
        let error = writer.apply(vec![change.clone()]).await.unwrap_err();
        assert_eq!(error.code, ErrorCode::PersistenceRejected);
        assert_eq!(error.phase, ErrorPhase::Persist);
        connection
            .execute_batch("DROP TRIGGER reject_settings")
            .unwrap();
        drop(connection);
        assert_eq!(
            writer.apply(vec![change]).await.unwrap(),
            vec![ApplyOutcome::Applied]
        );
        writer.flush().await.unwrap();
        writer.close().await.unwrap();
        let (repo, loaded) = Repository::open(&directory).unwrap();
        assert_eq!(loaded.settings.theme, "light");
        drop(repo);
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[tokio::test]
    async fn bounded_queue_preserves_ack_order_and_stale_write_is_ignored() {
        let directory = directory();
        let (writer, _) = PersistenceWriter::open(directory.clone()).await.unwrap();
        let newer = writer
            .apply(vec![Mutation::UpsertSettings {
                seq: 2,
                settings: Settings {
                    theme: "light".into(),
                    ..Settings::default()
                },
            }])
            .await
            .unwrap();
        let older = writer
            .apply(vec![Mutation::UpsertSettings {
                seq: 1,
                settings: Settings::default(),
            }])
            .await
            .unwrap();
        assert_eq!(newer, vec![ApplyOutcome::Applied]);
        assert_eq!(older, vec![ApplyOutcome::Stale]);
        writer.flush().await.unwrap();
        writer.close().await.unwrap();
        let (repo, loaded) = Repository::open(&directory).unwrap();
        assert_eq!(loaded.settings.theme, "light");
        drop(repo);
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[tokio::test]
    async fn receipt_failure_distinguishes_rejected_from_unconfirmed_without_losing_source_type() {
        let directory = directory();
        let (writer, _) = PersistenceWriter::open(directory.clone()).await.unwrap();
        let change = Mutation::UpsertSettings {
            seq: 1,
            settings: Settings::default(),
        };
        writer.apply(vec![change.clone()]).await.unwrap();
        let connection = rusqlite::Connection::open(directory.join("fluxdl.sqlite3")).unwrap();
        let changed = Settings {
            theme: "light".into(),
            ..Settings::default()
        };
        connection
            .execute(
                "UPDATE app_settings SET document=?1 WHERE id=1",
                [serde_json::to_string(&changed).unwrap()],
            )
            .unwrap();
        drop(connection);
        match writer.reconcile(vec![change.clone()]).await.unwrap_err() {
            WriteFailure::Unconfirmed(error) => {
                assert_eq!(error.code, ErrorCode::PersistenceUnconfirmed);
                assert_eq!(error.phase, ErrorPhase::Persist);
            }
            other => panic!("receipt outcome was incorrectly rejected: {other}"),
        }
        writer.close().await.unwrap();
        match writer.apply_checked(vec![change]).await.unwrap_err() {
            WriteFailure::Rejected(error) => assert_eq!(error.code, ErrorCode::PersistenceRejected),
            other => panic!("closed writer accepted an unknown outcome: {other}"),
        }
        drop(writer);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
