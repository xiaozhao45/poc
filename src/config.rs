//! 用户信息：本仓库 meta → $GIT_AUTHOR_* → Git 全局（gix-config，含 include 展开）。

use rust_i18n::t;

use crate::db::Store;
use crate::err::{PocError, Res};
use crate::object::Author;

pub const USER_NAME: &str = "user.name";
pub const USER_EMAIL: &str = "user.email";

/// 用户信息即 op 的 author 字段（object::Author）。
pub type Identity = Author;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Repo,
    Env,
    GitGlobal,
}

impl Source {
    pub fn label(self) -> String {
        match self {
            Source::Repo => t!("config.source_repo").to_string(),
            Source::Env => t!("config.source_env").to_string(),
            Source::GitGlobal => t!("config.source_git").to_string(),
        }
    }
}

pub fn valid_config_key(key: &str) -> bool {
    matches!(key, USER_NAME | USER_EMAIL)
}

pub fn git_global_identity() -> Option<Identity> {
    let f = gix_config::File::from_globals().ok()?;
    let name = f.string("user.name")?;
    let email = f.string("user.email").unwrap_or_default();
    let name = String::from_utf8(name.to_vec()).ok()?;
    if name.is_empty() {
        None
    } else {
        Some(Identity {
            name,
            email: String::from_utf8(email.to_vec()).unwrap_or_default(),
        })
    }
}

/// 单键解析：本仓库 meta → $GIT_AUTHOR_* → Git 全局。
pub fn resolve_key(store: &Store, key: &str) -> Res<Option<(String, Source)>> {
    if let Some(v) = store.meta_get(key)?
        && !v.trim().is_empty()
    {
        return Ok(Some((v, Source::Repo)));
    }
    let env_key = match key {
        USER_NAME => "GIT_AUTHOR_NAME",
        USER_EMAIL => "GIT_AUTHOR_EMAIL",
        _ => return Ok(None),
    };
    if let Ok(v) = std::env::var(env_key)
        && !v.trim().is_empty()
    {
        return Ok(Some((v, Source::Env)));
    }
    if let Some(id) = git_global_identity() {
        let v = match key {
            USER_NAME => id.name,
            USER_EMAIL => id.email,
            _ => String::new(),
        };
        if !v.trim().is_empty() {
            return Ok(Some((v, Source::GitGlobal)));
        }
    }
    Ok(None)
}

/// name 必有、email 可空。返回（身份, name 的来源）。
pub fn resolve_identity(store: &Store) -> Res<(Identity, Source)> {
    let (name, src) = resolve_key(store, USER_NAME)?
        .ok_or_else(|| PocError::Config(t!("config.err_missing_name").to_string()))?;
    let email = resolve_key(store, USER_EMAIL)?
        .map(|(v, _)| v)
        .unwrap_or_default();
    Ok((Identity { name, email }, src))
}
