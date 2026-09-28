use async_trait::async_trait;
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};
use zm_core::Result;

#[async_trait]
pub trait CredentialStore: Send + Sync {
    async fn save(&self, id: &str, account: &str, password: &str) -> Result<()>;
    async fn load(&self, id: &str, account: &str) -> Result<Option<String>>;
    async fn delete(&self, id: &str, account: &str) -> Result<()>;
}

#[derive(Default, Clone)]
pub struct SessionCredentialStore {
    values: Arc<RwLock<HashMap<(String, String), String>>>,
}
#[async_trait]
impl CredentialStore for SessionCredentialStore {
    async fn save(&self, id: &str, account: &str, password: &str) -> Result<()> {
        self.values
            .write()
            .unwrap()
            .insert((id.into(), account.into()), password.into());
        Ok(())
    }
    async fn load(&self, id: &str, account: &str) -> Result<Option<String>> {
        Ok(self
            .values
            .read()
            .unwrap()
            .get(&(id.into(), account.into()))
            .cloned())
    }
    async fn delete(&self, id: &str, account: &str) -> Result<()> {
        self.values
            .write()
            .unwrap()
            .remove(&(id.into(), account.into()));
        Ok(())
    }
}
