use super::*;

impl ZmApp {
    pub(super) fn update_remember_password(&mut self) {
        let AccountMode::Saved(id) = self.account_mode else {
            return;
        };
        let Some(index) = self.config.accounts.iter().position(|entry| entry.id == id) else {
            return;
        };
        let previous = self.config.accounts[index].remember_password;
        self.config.accounts[index].remember_password = self.save_password;
        if let Err(error) = self.save_config() {
            self.config.accounts[index].remember_password = previous;
            self.save_password = previous;
            self.status = format!("保存密码偏好失败：{error}");
            return;
        }
        let account = &self.config.accounts[index];
        let reply = if !self.save_password {
            self.credentials
                .forget(&account.credential_id, &account.account)
        } else if !self.password.is_empty() {
            self.credentials.save(
                &account.credential_id,
                &account.account,
                &self.password,
                true,
            )
        } else {
            return;
        };
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            if let Err(error) = receive_credential(reply).await {
                let _ = tx.send(AppMessage::Notice(format!("本地密码文件更新失败：{error}")));
            }
        });
    }

    pub(super) fn add_managed_account(&mut self) {
        let account_name = self.manager_account.trim().to_owned();
        if account_name.is_empty() || self.manager_password.is_empty() {
            self.status = "请输入新用户的用户名和密码".into();
            return;
        }
        if self
            .config
            .accounts
            .iter()
            .any(|account| account.account == account_name)
        {
            self.status = "该用户已存在，可直接点击切换".into();
            return;
        }

        self.launch.cancel();
        self.captcha_id = None;
        self.captcha_url = None;
        self.captcha_texture = None;
        self.captcha_value.clear();
        let mut account = AccountConfig::new(&account_name);
        account.remember_password = self.manager_save_password;
        let password = self.manager_password.clone();
        let remember_password = self.manager_save_password;
        let previous_last_account = self.config.last_account;
        self.config.accounts.push(account.clone());
        self.config.last_account = Some(account.id);
        if let Err(error) = self.save_config() {
            self.config.accounts.retain(|entry| entry.id != account.id);
            self.config.last_account = previous_last_account;
            self.status = format!("添加用户失败：{error}");
            return;
        }

        let reply = self.credentials.save(
            &account.credential_id,
            &account.account,
            &password,
            remember_password,
        );
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            if let Err(error) = receive_credential(reply).await {
                let _ = tx.send(AppMessage::Notice(format!("本地密码文件更新失败：{error}")));
            }
        });

        self.credential_request_id = self.credential_request_id.wrapping_add(1);
        self.account_mode = AccountMode::Saved(account.id);
        self.account = account_name;
        self.password.clone_from(&self.manager_password);
        self.credential_state = CredentialState::Available;
        self.save_password = remember_password;
        self.manager_account.clear();
        self.manager_password.clear();
        self.account_picker_open = false;
        self.status = "新用户已添加并切换".into();
    }

    pub(super) fn delete_managed_account(&mut self, id: Uuid) {
        let Some(index) = self.config.accounts.iter().position(|entry| entry.id == id) else {
            return;
        };
        let removed = self.config.accounts.remove(index);
        let previous_last_account = self.config.last_account;
        if self.config.last_account == Some(id) {
            self.config.last_account = None;
        }
        if let Err(error) = self.save_config() {
            self.config.accounts.insert(index, removed);
            self.config.last_account = previous_last_account;
            self.status = format!("删除用户失败：{error}");
            return;
        }
        let reply = self
            .credentials
            .delete(&removed.credential_id, &removed.account);
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            if let Err(error) = receive_credential(reply).await {
                let _ = tx.send(AppMessage::Notice(format!(
                    "账号记录已删除，但本地密码删除失败：{error}"
                )));
            }
        });
        if self.account_mode == AccountMode::Saved(id) {
            self.select_account(AccountMode::New);
            self.account_picker_open = true;
        }
        self.status = "用户已删除".into();
    }
}
