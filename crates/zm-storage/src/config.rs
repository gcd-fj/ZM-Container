use crate::private_file::atomic_write;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};
use uuid::Uuid;
use zm_core::{Result, ZmError};

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AccountConfig {
    pub id: Uuid,
    pub account: String,
    pub display_name: String,
    pub uid: Option<u64>,
    pub credential_id: String,
    #[serde(default = "default_remember_password")]
    pub remember_password: bool,
}

fn default_remember_password() -> bool {
    true
}

impl AccountConfig {
    pub fn new(account: impl Into<String>) -> Self {
        let account = account.into();
        let id = Uuid::new_v4();
        Self {
            id,
            display_name: account.clone(),
            account,
            uid: None,
            credential_id: id.to_string(),
            remember_password: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppConfig {
    pub schema_version: u32,
    pub accounts: Vec<AccountConfig>,
    pub last_account: Option<Uuid>,
    pub volume: f32,
}
impl Default for AppConfig {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            accounts: vec![],
            last_account: None,
            volume: 1.0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ConfigStore {
    path: PathBuf,
    legacy_path: Option<PathBuf>,
}
impl ConfigStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            legacy_path: None,
        }
    }
    pub fn with_legacy_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.legacy_path = Some(path.into());
        self
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn load(&self) -> Result<AppConfig> {
        if let Some(config) = Self::read(&self.path)? {
            return Ok(config);
        }
        if let Some(legacy) = &self.legacy_path
            && let Some(config) = Self::read(legacy)?
        {
            let raw =
                toml::to_string_pretty(&config).map_err(|e| ZmError::Config(e.to_string()))?;
            // Keep the old file. Never replace a config created concurrently by another instance.
            match atomic_write(&self.path, raw.as_bytes(), false) {
                Ok(()) => return Ok(config),
                Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                    return Self::new(&self.path).load();
                }
                Err(error) => return Err(ZmError::io(&self.path, error)),
            }
        }
        Ok(AppConfig::default())
    }

    fn read(path: &Path) -> Result<Option<AppConfig>> {
        let raw = match fs::read_to_string(path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(ZmError::io(path, error)),
        };
        let config: AppConfig = toml::from_str(&raw).map_err(|e| ZmError::Config(e.to_string()))?;
        Self::validate(&config)?;
        Ok(Some(config))
    }

    fn validate(config: &AppConfig) -> Result<()> {
        if config.schema_version != SCHEMA_VERSION {
            return Err(ZmError::Config(format!(
                "不支持的配置版本 {}",
                config.schema_version
            )));
        }
        if !config.volume.is_finite() || !(0.0..=1.0).contains(&config.volume) {
            return Err(ZmError::Config("音量必须在 0 到 1 之间".into()));
        }
        Ok(())
    }
    pub fn save(&self, config: &AppConfig) -> Result<()> {
        Self::validate(config)?;
        let raw = toml::to_string_pretty(config).map_err(|e| ZmError::Config(e.to_string()))?;
        atomic_write(&self.path, raw.as_bytes(), true).map_err(|e| ZmError::io(&self.path, e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn config_never_serializes_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::new(dir.path().join("config.toml"));
        store.save(&AppConfig::default()).unwrap();
        let raw = fs::read_to_string(store.path()).unwrap();
        assert!(!raw.contains("password") && !raw.contains("token") && !raw.contains("cookie"));
    }
    #[test]
    fn account_names_containing_secret_words_are_valid() {
        let dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::new(dir.path().join("config.toml"));
        let mut config = AppConfig::default();
        config
            .accounts
            .push(AccountConfig::new("cookie_token_password"));
        store.save(&config).unwrap();
        assert_eq!(store.load().unwrap(), config);
        config.volume = 0.5;
        store.save(&config).unwrap();
        assert_eq!(store.load().unwrap(), config);
    }
    #[test]
    fn invalid_config_is_reported_without_overwriting() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "schema_version = 999").unwrap();
        let store = ConfigStore::new(&path);
        assert!(store.load().is_err());
        assert_eq!(
            std::fs::read_to_string(path).unwrap(),
            "schema_version = 999"
        );
    }

    #[test]
    fn legacy_accounts_settings_and_ids_migrate_once_without_deleting_the_source() {
        let dir = tempfile::tempdir().unwrap();
        let legacy_path = dir.path().join("zm-linux/config.toml");
        let path = dir.path().join("zm/config.toml");
        let mut config = AppConfig {
            volume: 0.4,
            ..Default::default()
        };
        let mut account = AccountConfig::new("old-account");
        account.remember_password = false;
        config.last_account = Some(account.id);
        config.accounts.push(account);
        ConfigStore::new(&legacy_path).save(&config).unwrap();
        let original = fs::read(&legacy_path).unwrap();
        let store = ConfigStore::new(&path).with_legacy_path(&legacy_path);
        assert_eq!(store.load().unwrap(), config);
        assert_eq!(ConfigStore::new(&path).load().unwrap(), config);
        assert_eq!(fs::read(&legacy_path).unwrap(), original);
        config.volume = 0.7;
        store.save(&config).unwrap();
        assert_eq!(store.load().unwrap(), config);
        assert_eq!(fs::read(&legacy_path).unwrap(), original);
    }

    #[test]
    fn missing_legacy_config_is_a_fresh_install_but_corrupt_files_are_not() {
        let dir = tempfile::tempdir().unwrap();
        let legacy_path = dir.path().join("old.toml");
        let path = dir.path().join("zm/config.toml");
        let store = ConfigStore::new(&path).with_legacy_path(&legacy_path);
        assert_eq!(store.load().unwrap(), AppConfig::default());
        fs::write(&legacy_path, "broken legacy").unwrap();
        assert!(store.load().is_err());
        assert!(!path.exists());
        assert_eq!(fs::read_to_string(&legacy_path).unwrap(), "broken legacy");
        store.save(&AppConfig::default()).unwrap();
        assert_eq!(store.load().unwrap(), AppConfig::default());
        fs::write(&path, "broken new").unwrap();
        ConfigStore::new(&legacy_path)
            .save(&AppConfig::default())
            .unwrap();
        assert!(store.load().is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "broken new");
    }
}
