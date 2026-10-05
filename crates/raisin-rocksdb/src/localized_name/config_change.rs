//! Writing a repository record: when the localized-name fingerprint changes,
//! every build state record of the repository goes `NotBuilt` IN THE SAME
//! WRITE BATCH, and builds are requested.
//!
//! Used by the origin (`RepositoryManagementRepository::update_repository_config`)
//! and by the replication apply arm of `UpdateRepository`, so a default
//! language changed on a peer can never
//! leave this node's index `Ready` under the old configuration. (The
//! fingerprint check at read time would already refuse such a record; the
//! flip makes the state honest for the console and the automatic builder,
//! and the generation it raises stops a build already running from stamping
//! `Ready`.)

use super::config::NameConfig;
use super::state;
use crate::{cf, cf_handle};
use raisin_context::RepositoryConfig;
use raisin_error::{Error, Result};
use rocksdb::{WriteBatch, DB};

/// Put the repository record (`key` -> `value`, already encoded) in `cf::REGISTRY`.
/// `previous`: the configuration it replaces (`None`: a new repository).
/// Returns whether the localized-name fingerprint changed.
pub fn write_repository_record(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    key: &[u8],
    value: &[u8],
    previous: Option<&RepositoryConfig>,
    current: &RepositoryConfig,
) -> Result<bool> {
    let changed = previous.is_some_and(|previous| {
        NameConfig::from_repository(previous).fingerprint()
            != NameConfig::from_repository(current).fingerprint()
    });
    let mut batch = WriteBatch::default();
    batch.put_cf(cf_handle(db, cf::REGISTRY)?, key, value);
    if changed {
        let _guard = state::transitions();
        let flipped = state::stage_not_built_for_repo(db, &mut batch, tenant_id, repo_id)?;
        db.write(batch).map_err(|e| Error::storage(e.to_string()))?;
        tracing::info!(
            tenant_id,
            repo_id,
            flipped,
            "localized name configuration changed: index state NotBuilt until rebuilt"
        );
    } else {
        db.write(batch).map_err(|e| Error::storage(e.to_string()))?;
    }
    if changed {
        super::auto::request_build(tenant_id, repo_id, "");
    }
    Ok(changed)
}
