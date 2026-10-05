//! The part of a repository's configuration the localized name index depends
//! on, and its fingerprint.
//!
//! Read straight from `cf::REGISTRY` on every use (one point read, no cache):
//! a stale cached default language is exactly the wrong-name bug the
//! fingerprinted state record exists to prevent, and the config already
//! replicates (`UpdateRepository`) and arrives in checkpoints.

use crate::{cf, keys};
use raisin_context::{RepositoryConfig, RepositoryInfo};
use raisin_error::Result;
use rocksdb::DB;
use sha2::{Digest, Sha256};

/// What the selector and the lookup need from `RepositoryConfig`.
#[derive(Debug, Clone, PartialEq)]
pub struct NameConfig {
    pub default_language: String,
    /// The supported locales, minus the default (sorted).
    pub locales: Vec<String>,
    pub enforce_unique: bool,
    config: RepositoryConfig,
}

impl NameConfig {
    pub fn from_repository(config: &RepositoryConfig) -> Self {
        let mut locales: Vec<String> = config
            .supported_languages
            .iter()
            .filter(|l| **l != config.default_language)
            .cloned()
            .collect();
        locales.sort();
        locales.dedup();
        Self {
            default_language: config.default_language.clone(),
            locales,
            enforce_unique: config.localized_names.enforce_unique,
            config: config.clone(),
        }
    }

    /// `get_fallback_chain(locale)` of the repository.
    pub fn fallback_chain(&self, locale: &str) -> Vec<String> {
        self.config.get_fallback_chain(locale)
    }

    /// Hash of everything the index CONTENT and the lookup depend on:
    /// `(default_language, supported languages, fallback chains)`. A state
    /// record built under another fingerprint is not ready.
    pub fn fingerprint(&self) -> String {
        let mut chains: Vec<(&String, &Vec<String>)> =
            self.config.locale_fallback_chains.iter().collect();
        chains.sort();
        let canonical = serde_json::json!({
            "default": self.default_language,
            "locales": self.locales,
            "chains": chains,
        });
        let digest = Sha256::digest(canonical.to_string().as_bytes());
        hex::encode(&digest[..12])
    }
}

/// The repository's configuration as stored on this node (`None`: no such
/// repository here).
pub fn load_repository_config(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
) -> Result<Option<RepositoryConfig>> {
    load_repository_config_in(&mut crate::mvcc_read::DbRead(db), tenant_id, repo_id)
}

/// [`load_repository_config`] through `src`'s view.
fn load_repository_config_in(
    src: &mut impl crate::mvcc_read::VersionedRead,
    tenant_id: &str,
    repo_id: &str,
) -> Result<Option<RepositoryConfig>> {
    let Some(bytes) = src.get(cf::REGISTRY, &keys::repository_key(tenant_id, repo_id))? else {
        return Ok(None);
    };
    match rmp_serde::from_slice::<RepositoryInfo>(&bytes) {
        Ok(info) => Ok(Some(info.config)),
        Err(e) => Err(raisin_error::Error::storage(format!(
            "unreadable repository record {tenant_id}/{repo_id}: {e}"
        ))),
    }
}

/// [`load_repository_config`] as a [`NameConfig`].
pub fn load(db: &DB, tenant_id: &str, repo_id: &str) -> Result<Option<NameConfig>> {
    load_in(&mut crate::mvcc_read::DbRead(db), tenant_id, repo_id)
}

/// [`load`] through `src`'s view — the lookup reads the config (and so the
/// fingerprint it checks the build state against) from its own snapshot.
pub(crate) fn load_in(
    src: &mut impl crate::mvcc_read::VersionedRead,
    tenant_id: &str,
    repo_id: &str,
) -> Result<Option<NameConfig>> {
    Ok(load_repository_config_in(src, tenant_id, repo_id)?
        .as_ref()
        .map(NameConfig::from_repository))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// A repository record written before `localized_names` existed decodes,
    /// with the default (positional msgpack: the missing trailing field takes
    /// its `#[serde(default)]`) — every existing database has such records.
    #[test]
    fn a_record_from_before_the_field_decodes() {
        #[derive(serde::Serialize)]
        struct OldConfig {
            default_branch: String,
            description: Option<String>,
            tags: HashMap<String, String>,
            default_language: String,
            supported_languages: Vec<String>,
            locale_fallback_chains: HashMap<String, Vec<String>>,
        }
        #[derive(serde::Serialize)]
        struct OldInfo {
            tenant_id: String,
            repo_id: String,
            created_at: chrono::DateTime<chrono::Utc>,
            branches: Vec<String>,
            config: OldConfig,
        }
        let old = OldInfo {
            tenant_id: "t".into(),
            repo_id: "r".into(),
            created_at: chrono::Utc::now(),
            branches: vec!["main".into()],
            config: OldConfig {
                default_branch: "main".into(),
                description: None,
                tags: HashMap::new(),
                default_language: "en".into(),
                supported_languages: vec!["en".into(), "fr".into()],
                locale_fallback_chains: HashMap::new(),
            },
        };
        let bytes = rmp_serde::to_vec(&old).unwrap();
        let info: RepositoryInfo = rmp_serde::from_slice(&bytes).unwrap();
        assert_eq!(info.config.supported_languages, vec!["en", "fr"]);
        assert_eq!(info.config.localized_names, Default::default());
        // And a new record round-trips with enforcement set.
        let mut config = info.config.clone();
        config.localized_names.enforce_unique = true;
        let info = RepositoryInfo { config, ..info };
        let back: RepositoryInfo =
            rmp_serde::from_slice(&rmp_serde::to_vec(&info).unwrap()).unwrap();
        assert_eq!(back, info);
    }

    #[test]
    fn the_fingerprint_moves_with_the_default_language() {
        let base = RepositoryConfig {
            supported_languages: vec!["en".into(), "fr".into()],
            ..RepositoryConfig::default()
        };
        let a = NameConfig::from_repository(&base).fingerprint();
        let mut other_default = base.clone();
        other_default.default_language = "fr".into();
        let c = NameConfig::from_repository(&other_default).fingerprint();
        assert_ne!(a, c);
        assert_eq!(a, NameConfig::from_repository(&base).fingerprint());
        // Uniqueness enforcement changes no entry: same fingerprint.
        let mut enforced = base;
        enforced.localized_names.enforce_unique = true;
        assert_eq!(a, NameConfig::from_repository(&enforced).fingerprint());
    }
}
