use crate::{
    CredentialStore,
    private_file::{atomic_write, ensure_private_dir, lock_file},
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, io::ErrorKind, path::PathBuf};
use zm_core::{Result, ZmError};

const CREDENTIAL_SCHEMA_VERSION: u32 = 1;

// Deliberately no Debug: TOML serialization is the only persistent secret sink.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedCredential {
    account: String,
    password: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CredentialsFile {
    schema_version: u32,
    accounts: BTreeMap<String, SavedCredential>,
}

impl Default for CredentialsFile {
    fn default() -> Self {
        Self {
            schema_version: CREDENTIAL_SCHEMA_VERSION,
            accounts: BTreeMap::new(),
        }
    }
}

/// Portable, plaintext credentials. Unix access is restricted to the current user.
#[derive(Clone)]
pub struct FileCredentialStore {
    path: PathBuf,
}

impl FileCredentialStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    fn read(&self) -> Result<CredentialsFile> {
        match fs::symlink_metadata(&self.path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(ZmError::Credential("密码文件不能是符号链接".into()));
            }
            Ok(metadata) if !metadata.is_file() => {
                return Err(ZmError::Credential("密码文件路径不是普通文件".into()));
            }
            Ok(_) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return Ok(CredentialsFile::default());
            }
            Err(error) => return Err(ZmError::io(&self.path, error)),
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&self.path, fs::Permissions::from_mode(0o600))
                .map_err(|error| ZmError::io(&self.path, error))?;
        }
        let raw = fs::read_to_string(&self.path).map_err(|error| ZmError::io(&self.path, error))?;
        // TOML errors can include the source line, including a plaintext password.
        let document: CredentialsFile = toml::from_str(&raw).map_err(|_| {
            ZmError::Credential("密码文件格式无效，原文件已保留；请备份并修复该文件".into())
        })?;
        if document.schema_version != CREDENTIAL_SCHEMA_VERSION {
            return Err(ZmError::Credential(
                "不支持的密码文件版本，原文件已保留".into(),
            ));
        }
        Ok(document)
    }

    async fn access<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&mut CredentialsFile) -> (T, bool) + Send + 'static,
    ) -> Result<T> {
        let store = self.clone();
        tokio::task::spawn_blocking(move || {
            let parent = store
                .path
                .parent()
                .ok_or_else(|| ZmError::Credential("密码文件路径没有父目录".into()))?;
            ensure_private_dir(parent).map_err(|error| ZmError::io(parent, error))?;
            let lock_path = store.path.with_extension("lock");
            let _lock = lock_file(&lock_path).map_err(|error| ZmError::io(&lock_path, error))?;
            // Read under the lock each time so independent app instances cannot lose entries.
            let mut document = store.read()?;
            let (result, changed) = operation(&mut document);
            if changed {
                let raw = toml::to_string_pretty(&document)
                    .map_err(|_| ZmError::Credential("无法序列化密码文件".into()))?;
                atomic_write(&store.path, raw.as_bytes(), true)
                    .map_err(|error| ZmError::io(&store.path, error))?;
            }
            Ok(result)
        })
        .await
        .map_err(|_| ZmError::Credential("本地密码存储任务失败".into()))?
    }
}

#[async_trait]
impl CredentialStore for FileCredentialStore {
    async fn save(&self, id: &str, account: &str, password: &str) -> Result<()> {
        let (id, account, password) = (id.to_owned(), account.to_owned(), password.to_owned());
        self.access(move |document| {
            document
                .accounts
                .insert(id, SavedCredential { account, password });
            ((), true)
        })
        .await
    }

    async fn load(&self, id: &str, account: &str) -> Result<Option<String>> {
        let (id, account) = (id.to_owned(), account.to_owned());
        self.access(move |document| {
            let password = document
                .accounts
                .get(&id)
                .filter(|entry| entry.account == account && !entry.password.is_empty())
                .map(|entry| entry.password.clone());
            (password, false)
        })
        .await
    }

    async fn delete(&self, id: &str, account: &str) -> Result<()> {
        let (id, account) = (id.to_owned(), account.to_owned());
        self.access(move |document| {
            let matches = document
                .accounts
                .get(&id)
                .is_some_and(|entry| entry.account == account);
            if matches {
                document.accounts.remove(&id);
            }
            ((), matches)
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn passwords_survive_restart_update_and_delete_without_touching_other_accounts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("zm/credentials.toml");
        let store = FileCredentialStore::new(&path);
        let password = "中文密码\n\"\\🔐";
        store.save("id1", "账号一", password).await.unwrap();
        store.save("id2", "account2", "other").await.unwrap();
        let restarted = FileCredentialStore::new(&path);
        assert_eq!(
            restarted.load("id1", "账号一").await.unwrap().as_deref(),
            Some(password)
        );
        assert!(
            restarted
                .load("id1", "different-account")
                .await
                .unwrap()
                .is_none()
        );
        restarted.delete("id1", "different-account").await.unwrap();
        assert!(restarted.load("id1", "账号一").await.unwrap().is_some());
        restarted
            .save("id1", "账号一", "new-password")
            .await
            .unwrap();
        assert_eq!(
            store.load("id1", "账号一").await.unwrap().as_deref(),
            Some("new-password")
        );
        restarted.delete("id1", "账号一").await.unwrap();
        assert!(store.load("id1", "账号一").await.unwrap().is_none());
        assert_eq!(
            store.load("id2", "account2").await.unwrap().as_deref(),
            Some("other")
        );
        assert!(!fs::read_to_string(path).unwrap().contains("new-password"));
    }

    #[tokio::test]
    async fn missing_file_is_empty_and_read_delete_do_not_create_a_password_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("zm/credentials.toml");
        let store = FileCredentialStore::new(&path);
        assert!(store.load("id", "account").await.unwrap().is_none());
        store.delete("id", "account").await.unwrap();
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn invalid_files_are_preserved_and_errors_never_echo_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("credentials.toml");
        let store = FileCredentialStore::new(&path);
        for raw in [
            "schema_version = 1\n[accounts.id]\naccount = 'account'\npassword = exposed-secret",
            "schema_version = 999\n[accounts.id]\naccount = 'account'\npassword = 'exposed-secret'",
        ] {
            fs::write(&path, raw).unwrap();
            for result in [
                store.load("id", "account").await.map(|_| ()),
                store.save("id", "account", "new").await,
                store.delete("id", "account").await,
            ] {
                assert!(!result.unwrap_err().to_string().contains("exposed-secret"));
                assert_eq!(fs::read_to_string(&path).unwrap(), raw);
            }
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn independent_stores_do_not_lose_concurrent_updates() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("credentials.toml");
        let mut tasks = Vec::new();
        for index in 0..16 {
            let store = FileCredentialStore::new(&path);
            tasks.push(tokio::spawn(async move {
                store
                    .save(&index.to_string(), "account", "password")
                    .await
                    .unwrap();
            }));
        }
        for task in tasks {
            task.await.unwrap();
        }
        let store = FileCredentialStore::new(&path);
        for index in 0..16 {
            assert_eq!(
                store
                    .load(&index.to_string(), "account")
                    .await
                    .unwrap()
                    .as_deref(),
                Some("password")
            );
        }
    }

    #[tokio::test]
    async fn copying_both_toml_files_to_another_directory_preserves_accounts_and_passwords() {
        use crate::{AccountConfig, AppConfig, ConfigStore};
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let account = AccountConfig::new("portable-account");
        let config = AppConfig {
            accounts: vec![account.clone()],
            last_account: Some(account.id),
            ..Default::default()
        };
        ConfigStore::new(source.path().join("config.toml"))
            .save(&config)
            .unwrap();
        FileCredentialStore::new(source.path().join("credentials.toml"))
            .save(
                &account.credential_id,
                &account.account,
                "portable-password",
            )
            .await
            .unwrap();
        for name in ["config.toml", "credentials.toml"] {
            fs::copy(source.path().join(name), target.path().join(name)).unwrap();
        }
        let copied = ConfigStore::new(target.path().join("config.toml"))
            .load()
            .unwrap();
        assert_eq!(copied, config);
        let account = &copied.accounts[0];
        assert_eq!(
            FileCredentialStore::new(target.path().join("credentials.toml"))
                .load(&account.credential_id, &account.account)
                .await
                .unwrap()
                .as_deref(),
            Some("portable-password")
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn non_regular_password_paths_are_rejected_without_changing_the_target() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let dir = tempfile::tempdir().unwrap();
        let external = dir.path().join("external");
        let path = dir.path().join("credentials.toml");
        fs::write(&external, "keep-me").unwrap();
        symlink(&external, &path).unwrap();
        let store = FileCredentialStore::new(&path);
        assert!(store.save("id", "account", "password").await.is_err());
        assert_eq!(fs::read_to_string(&external).unwrap(), "keep-me");
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(store.load("id", "account").await.is_err());
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn password_directory_files_and_replacements_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let parent = dir.path().join("zm");
        let path = parent.join("credentials.toml");
        let store = FileCredentialStore::new(&path);
        for password in ["first", "updated"] {
            store.save("id", "account", password).await.unwrap();
            assert_eq!(
                fs::metadata(&parent).unwrap().permissions().mode() & 0o777,
                0o700
            );
            for file in [&path, &parent.join("credentials.lock")] {
                assert_eq!(
                    fs::metadata(file).unwrap().permissions().mode() & 0o777,
                    0o600
                );
            }
        }
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        store.load("id", "account").await.unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
