#[cfg(target_os = "linux")]
use std::env;
use std::path::PathBuf;
use zm_core::{Result, ZmError};

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub config_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub data_dir: PathBuf,
    pub log_dir: PathBuf,
}

impl AppPaths {
    pub fn discover() -> Result<Self> {
        #[cfg(target_os = "linux")]
        let (config, cache, data) = {
            let config = writable_xdg_dir("XDG_CONFIG_HOME", ".config")
                .ok_or_else(|| ZmError::Config("无法确定XDG配置目录".into()))?;
            let cache = writable_xdg_dir("XDG_CACHE_HOME", ".cache")
                .ok_or_else(|| ZmError::Config("无法确定XDG缓存目录".into()))?;
            let data = writable_xdg_dir("XDG_DATA_HOME", ".local/share")
                .ok_or_else(|| ZmError::Config("无法确定XDG数据目录".into()))?;
            (config, cache, data)
        };
        #[cfg(not(target_os = "linux"))]
        let (config, cache, data) = (
            dirs::config_dir().ok_or_else(|| ZmError::Config("无法确定配置目录".into()))?,
            dirs::cache_dir().ok_or_else(|| ZmError::Config("无法确定缓存目录".into()))?,
            dirs::data_local_dir().ok_or_else(|| ZmError::Config("无法确定数据目录".into()))?,
        );
        Ok(Self::from_roots(config, cache, data))
    }

    fn from_roots(config: PathBuf, cache: PathBuf, data: PathBuf) -> Self {
        Self {
            config_dir: config.join("zm"),
            cache_dir: cache.join("zm-linux"),
            data_dir: data.join("zm-linux"),
            log_dir: data.join("zm-linux/logs"),
        }
    }
    pub fn ensure(&self) -> Result<()> {
        crate::private_file::ensure_private_dir(&self.config_dir)
            .map_err(|error| ZmError::io(&self.config_dir, error))?;
        for path in [&self.cache_dir, &self.data_dir, &self.log_dir] {
            std::fs::create_dir_all(path).map_err(|e| ZmError::io(path, e))?;
        }
        Ok(())
    }
    pub fn config_file(&self) -> PathBuf {
        self.config_dir.join("config.toml")
    }

    pub fn credentials_file(&self) -> PathBuf {
        self.config_dir.join("credentials.toml")
    }

    pub fn legacy_config_file(&self) -> PathBuf {
        self.config_dir
            .with_file_name("zm-linux")
            .join("config.toml")
    }
}

/// AppImage 启动器可能把 XDG 变量临时指向只读的挂载目录。
/// 不在该目录持久化用户数据，遇到这种情况统一回退到 `$HOME`。
#[cfg(target_os = "linux")]
fn writable_xdg_dir(variable: &str, home_suffix: &str) -> Option<PathBuf> {
    let app_dir = env::var_os("APPDIR").map(PathBuf::from);
    let configured = env::var_os(variable)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .filter(|path| app_dir.as_ref().is_none_or(|root| !path.starts_with(root)));
    configured.or_else(|| dirs::home_dir().map(|home| home.join(home_suffix)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portable_config_uses_zm_and_preserves_game_data_locations() {
        let root = tempfile::tempdir().unwrap();
        let paths = AppPaths::from_roots(
            root.path().join("config"),
            root.path().join("cache"),
            root.path().join("data"),
        );
        assert_eq!(
            paths.config_file(),
            root.path().join("config/zm/config.toml")
        );
        assert_eq!(
            paths.credentials_file(),
            root.path().join("config/zm/credentials.toml")
        );
        assert_eq!(
            paths.legacy_config_file(),
            root.path().join("config/zm-linux/config.toml")
        );
        assert_eq!(paths.cache_dir, root.path().join("cache/zm-linux"));
        assert_eq!(paths.data_dir, root.path().join("data/zm-linux"));
        paths.ensure().unwrap();
        assert!(paths.log_dir.is_dir());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn xdg_path_is_absolute() {
        assert!(writable_xdg_dir("XDG_CONFIG_HOME", ".config").is_some_and(|p| p.is_absolute()));
    }
}
