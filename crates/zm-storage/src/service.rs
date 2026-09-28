//! Ordered credential operations. Commands are queued at the UI call site, not
//! when a background task happens to be polled, so delete cannot race a save.
use crate::{CredentialStore, FileCredentialStore, SessionCredentialStore};
use std::{path::PathBuf, sync::Arc};
use tokio::sync::{mpsc, oneshot};
use zm_core::Result;

type Reply = oneshot::Sender<Result<Option<String>>>;
pub type CredentialReply = oneshot::Receiver<Result<Option<String>>>;
enum Command {
    Flush {
        reply: Reply,
    },
    Load {
        id: String,
        account: String,
        remember: bool,
        reply: Reply,
    },
    Save {
        id: String,
        account: String,
        password: String,
        remember: bool,
        reply: Reply,
    },
    Delete {
        id: String,
        account: String,
        keep_in_memory: bool,
        reply: Reply,
    },
}

pub struct CredentialService {
    tx: mpsc::UnboundedSender<Command>,
}
impl CredentialService {
    pub fn new(runtime: &tokio::runtime::Handle, path: impl Into<PathBuf>) -> Self {
        Self::with_store(runtime, Arc::new(FileCredentialStore::new(path)))
    }
    fn with_store(runtime: &tokio::runtime::Handle, persistent: Arc<dyn CredentialStore>) -> Self {
        let (tx, mut rx) = mpsc::unbounded_channel();
        runtime.spawn(async move {
            let memory = SessionCredentialStore::default();
            while let Some(command) = rx.recv().await {
                match command {
                    Command::Flush { reply } => {
                        let _ = reply.send(Ok(None));
                    }
                    Command::Load {
                        id,
                        account,
                        remember,
                        reply,
                    } => {
                        let result = match memory.load(&id, &account).await {
                            Ok(Some(password)) => Ok(Some(password)),
                            _ if !remember => Ok(None),
                            _ => persistent.load(&id, &account).await,
                        };
                        if let Ok(Some(password)) = &result {
                            let _ = memory.save(&id, &account, password).await;
                        }
                        let _ = reply.send(result);
                    }
                    Command::Save {
                        id,
                        account,
                        password,
                        remember,
                        reply,
                    } => {
                        let result = match memory.save(&id, &account, &password).await {
                            Err(error) => Err(error),
                            Ok(()) => {
                                if remember {
                                    persistent.save(&id, &account, &password).await
                                } else {
                                    persistent.delete(&id, &account).await
                                }
                            }
                        };
                        let _ = reply.send(result.map(|()| None));
                    }
                    Command::Delete {
                        id,
                        account,
                        keep_in_memory,
                        reply,
                    } => {
                        if !keep_in_memory {
                            let _ = memory.delete(&id, &account).await;
                        }
                        let result = persistent.delete(&id, &account).await;
                        let _ = reply.send(result.map(|()| None));
                    }
                }
            }
        });
        Self { tx }
    }
    pub fn load(&self, id: &str, account: &str, remember: bool) -> CredentialReply {
        let (reply, rx) = oneshot::channel();
        let _ = self.tx.send(Command::Load {
            id: id.into(),
            account: account.into(),
            remember,
            reply,
        });
        rx
    }
    /// A barrier for all operations already enqueued, used before runtime shutdown.
    pub fn flush(&self) -> CredentialReply {
        let (reply, rx) = oneshot::channel();
        let _ = self.tx.send(Command::Flush { reply });
        rx
    }
    pub fn save(&self, id: &str, account: &str, password: &str, remember: bool) -> CredentialReply {
        let (reply, rx) = oneshot::channel();
        let _ = self.tx.send(Command::Save {
            id: id.into(),
            account: account.into(),
            password: password.into(),
            remember,
            reply,
        });
        rx
    }
    pub fn delete(&self, id: &str, account: &str) -> CredentialReply {
        self.remove(id, account, false)
    }
    /// Stop remembering a password while retaining it for this running session.
    pub fn forget(&self, id: &str, account: &str) -> CredentialReply {
        self.remove(id, account, true)
    }
    fn remove(&self, id: &str, account: &str, keep_in_memory: bool) -> CredentialReply {
        let (reply, rx) = oneshot::channel();
        let _ = self.tx.send(Command::Delete {
            id: id.into(),
            account: account.into(),
            keep_in_memory,
            reply,
        });
        rx
    }
}

pub async fn receive_credential(
    reply: CredentialReply,
) -> std::result::Result<Option<String>, String> {
    reply
        .await
        .map_err(|_| "凭据服务已停止".to_owned())?
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn queued_delete_wins_even_when_save_reply_is_dropped() {
        let persistent = Arc::new(SessionCredentialStore::default());
        let service =
            CredentialService::with_store(&tokio::runtime::Handle::current(), persistent.clone());
        drop(service.save("id", "account", "password", true));
        receive_credential(service.delete("id", "account"))
            .await
            .unwrap();
        assert!(
            receive_credential(service.load("id", "account", true))
                .await
                .unwrap()
                .is_none()
        );
        assert!(persistent.load("id", "account").await.unwrap().is_none());
    }
    #[tokio::test]
    async fn memory_only_save_removes_old_persistent_password() {
        let persistent = Arc::new(SessionCredentialStore::default());
        persistent.save("id", "account", "old").await.unwrap();
        let service =
            CredentialService::with_store(&tokio::runtime::Handle::current(), persistent.clone());
        receive_credential(service.save("id", "account", "new", false))
            .await
            .unwrap();
        assert_eq!(
            receive_credential(service.load("id", "account", true))
                .await
                .unwrap()
                .as_deref(),
            Some("new")
        );
        assert!(persistent.load("id", "account").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn forget_keeps_current_session_but_not_a_restarted_service() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("credentials.toml");
        let runtime = tokio::runtime::Handle::current();
        let store = FileCredentialStore::new(&path);
        store.save("id", "account", "password").await.unwrap();
        let service = CredentialService::new(&runtime, &path);
        assert!(
            receive_credential(service.load("id", "account", true))
                .await
                .unwrap()
                .is_some()
        );
        receive_credential(service.forget("id", "account"))
            .await
            .unwrap();
        assert_eq!(
            receive_credential(service.load("id", "account", false))
                .await
                .unwrap()
                .as_deref(),
            Some("password")
        );
        let restarted = CredentialService::new(&runtime, &path);
        assert!(
            receive_credential(restarted.load("id", "account", true))
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn no_remember_never_loads_a_stale_disk_password() {
        let persistent = Arc::new(SessionCredentialStore::default());
        persistent.save("id", "account", "stale").await.unwrap();
        let service = CredentialService::with_store(&tokio::runtime::Handle::current(), persistent);
        assert!(
            receive_credential(service.load("id", "account", false))
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn flush_finishes_queued_disk_writes_even_with_dropped_replies() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("credentials.toml");
        let service = CredentialService::new(&tokio::runtime::Handle::current(), &path);
        drop(service.save("first", "account1", "old", true));
        drop(service.save("second", "account2", "retained", true));
        drop(service.delete("first", "account1"));
        receive_credential(service.flush()).await.unwrap();
        let store = FileCredentialStore::new(&path);
        assert!(store.load("first", "account1").await.unwrap().is_none());
        assert_eq!(
            store.load("second", "account2").await.unwrap().as_deref(),
            Some("retained")
        );
    }

    #[tokio::test]
    async fn failed_disk_write_keeps_new_password_in_memory_and_preserves_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("credentials.toml");
        std::fs::write(&path, "broken").unwrap();
        let service = CredentialService::new(&tokio::runtime::Handle::current(), &path);
        assert!(
            receive_credential(service.save("id", "account", "new", true))
                .await
                .is_err()
        );
        assert_eq!(
            receive_credential(service.load("id", "account", true))
                .await
                .unwrap()
                .as_deref(),
            Some("new")
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "broken");
    }
}
