//! Changing a repository's default language.
//!
//! Base (untranslated) content is stored in the repository's default language
//! and every other language is a translation overlay on top of it. Full-text
//! search files base content under the default language, so a change has three
//! consequences this module provides the storage side of:
//!
//! 1. **Collision check.** Overlays that already exist in the NEW default
//!    language would sit on top of base content in the same language: the
//!    resolver skips overlays for the default language, so they would silently
//!    stop applying. [`count_translation_overlays`] counts them, and the change
//!    is refused while there are any.
//! 2. **Re-index.** Base content has to move from the old language's full-text
//!    documents to the new one's. [`enqueue_fulltext_rebuild_all_branches`]
//!    queues a `FulltextRebuild` for every branch; the rebuild reads the
//!    repository config and indexes base content under the new default.
//! 3. **Replication.** The config change itself replicates as the ordinary
//!    `UpdateRepository` operation. A peer's full-text index is its own,
//!    though, so the applicator publishes a repository `Updated` event marked
//!    with [`META_DEFAULT_LANGUAGE_CHANGED`] when an applied update changes the
//!    default, and [`DefaultLanguageReindexHandler`] queues the same rebuilds
//!    on the peer.
//!
//! Vector embeddings need nothing: the embedding pipeline embeds base content
//! with no language attached, and the HNSW index is keyed by tenant, repository,
//! branch and workspace only, so a default-language change leaves every stored
//! vector valid.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use raisin_error::{Error, Result};
use raisin_events::{Event, EventHandler, RepositoryEventKind};
use raisin_models::translations::LocaleOverlay;
use raisin_storage::jobs::{JobContext, JobId, JobType};
use serde::Serialize;

use crate::management::helpers::list_branches;
use crate::storage::RocksDBStorage;
use crate::{cf, cf_handle};

/// Repository event metadata key: `true` when an update changed the default language.
pub const META_DEFAULT_LANGUAGE_CHANGED: &str = "default_language_changed";
/// Repository event metadata key: the default language before the change.
pub const META_PREVIOUS_DEFAULT_LANGUAGE: &str = "previous_default_language";
/// Repository event metadata key: the default language after the change.
pub const META_DEFAULT_LANGUAGE: &str = "default_language";
/// Job context metadata key naming why a full-text rebuild was queued.
pub const META_REINDEX_REASON: &str = "reason";
/// [`META_REINDEX_REASON`] value for a rebuild queued by a default-language change.
pub const REINDEX_REASON_DEFAULT_LANGUAGE: &str = "default_language_changed";

const TOMBSTONE: &[u8] = b"T";

/// Live translation overlays of one language across a repository.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct OverlayCount {
    /// Node-level overlays (one per node, branch and workspace).
    pub node_overlays: u64,
    /// Block-level overlays (one per block, node, branch and workspace).
    pub block_overlays: u64,
}

impl OverlayCount {
    pub fn total(&self) -> u64 {
        self.node_overlays + self.block_overlays
    }
}

/// Count the live translation overlays in `locale` on every branch of the repository.
///
/// "Live" is the newest revision of an overlay: not deleted (a tombstone),
/// not ended by a delete of its node, and not deleted as a translation (an
/// empty overlay). A `Hidden` overlay counts — it is a real per-locale
/// decision on top of base content.
///
/// Reads the data column families rather than `TRANSLATION_INDEX`, which is
/// append-only and would keep counting overlays that have since been deleted.
pub async fn count_translation_overlays(
    storage: &RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
    locale: &str,
) -> Result<OverlayCount> {
    let branches: HashSet<String> = list_branches(storage, tenant_id, repo_id)
        .await?
        .into_iter()
        .collect();
    let prefix = format!("{tenant_id}\0{repo_id}\0").into_bytes();

    // translation_data: {tenant}\0{repo}\0{branch}\0{ws}\0translations\0{node}\0{locale}\0{~rev}
    let node_overlays = count_live_in_cf(
        storage,
        cf::TRANSLATION_DATA,
        &prefix,
        "translations",
        7,
        &branches,
        locale,
    )?;
    // block_translations: {tenant}\0{repo}\0{branch}\0{ws}\0block_trans\0{node}\0{block}\0{locale}\0{~rev}
    let block_overlays = count_live_in_cf(
        storage,
        cf::BLOCK_TRANSLATIONS,
        &prefix,
        "block_trans",
        8,
        &branches,
        locale,
    )?;

    Ok(OverlayCount {
        node_overlays,
        block_overlays,
    })
}

/// Count the overlays of one column family whose NEWEST revision is live.
///
/// `fields` is the number of NUL-terminated text fields before the revision
/// (the revision is binary and may itself contain NUL bytes, so the key is only
/// split that far). The locale is the last of them. Revisions are encoded
/// descending, so the first key of each overlay is its newest revision.
fn count_live_in_cf(
    storage: &RocksDBStorage,
    cf_name: &str,
    prefix: &[u8],
    entity: &str,
    fields: usize,
    branches: &HashSet<String>,
    locale: &str,
) -> Result<u64> {
    let db = storage.db();
    let cf = cf_handle(db, cf_name)?;
    let mut count = 0u64;
    let mut current_overlay: Option<Vec<u8>> = None;

    for item in crate::prefix_scan(db, cf, prefix) {
        let (key, value) = item.map_err(|e| Error::storage(e.to_string()))?;
        if !key.starts_with(prefix) {
            break;
        }
        let Some(overlay_len) = nth_nul_end(&key, fields) else {
            continue;
        };
        let overlay_key = &key[..overlay_len];
        if current_overlay.as_deref() == Some(overlay_key) {
            continue; // an older revision of an overlay already decided
        }
        current_overlay = Some(overlay_key.to_vec());

        let parts: Vec<&[u8]> = overlay_key[..overlay_len - 1].split(|b| *b == 0).collect();
        if parts.len() != fields
            || parts[4] != entity.as_bytes()
            || parts[fields - 1] != locale.as_bytes()
        {
            continue;
        }
        if !branches.contains(&*String::from_utf8_lossy(parts[2])) {
            continue;
        }
        if is_live_overlay(&value) && !ended_by_node_delete(db, &parts, &key)? {
            count += 1;
        }
    }
    Ok(count)
}

/// A live overlay version (node or block) a delete of its node has ended —
/// the one reader's rule (`translation_read::ended_by_node_delete`), so a
/// deleted node's translations are not counted. The node id is segment 5 in
/// both column families (`…\0translations\0{node}` / `…\0block_trans\0{node}`).
fn ended_by_node_delete(db: &rocksdb::DB, parts: &[&[u8]], key: &[u8]) -> Result<bool> {
    let text = |i: usize| String::from_utf8_lossy(parts[i]).into_owned();
    let Ok(revision) = crate::keys::extract_revision_from_key(key) else {
        return Ok(false);
    };
    let (tenant, repo, branch, workspace) = (text(0), text(1), text(2), text(3));
    crate::translation_read::ended_by_node_delete(
        db,
        (&tenant, &repo, &branch, &workspace),
        &text(5),
        &revision,
        None,
    )
}

/// Length of the key prefix ending with (and including) its `n`th NUL byte.
fn nth_nul_end(key: &[u8], n: usize) -> Option<usize> {
    key.iter()
        .enumerate()
        .filter(|(_, b)| **b == 0)
        .nth(n - 1)
        .map(|(i, _)| i + 1)
}

fn is_live_overlay(value: &[u8]) -> bool {
    if value == TOMBSTONE {
        return false;
    }
    match serde_json::from_slice::<LocaleOverlay>(value) {
        Ok(overlay) => overlay.is_hidden() || !overlay.is_empty(),
        // Unreadable: count it. Refusing a change is recoverable; silently
        // shadowing base content is not.
        Err(_) => true,
    }
}

/// Queue one full-text maintenance job for a branch and return its id.
///
/// The job context is written BEFORE the job is registered under the same id,
/// so the worker that picks it up can never find it contextless. `max_retries`
/// is 0: a rebuild is an operator-level decision, not something to repeat
/// silently. The worker (`FulltextJobHandler`) does the work.
pub async fn enqueue_fulltext_job(
    storage: &RocksDBStorage,
    job_type: JobType,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    metadata: HashMap<String, serde_json::Value>,
) -> Result<JobId> {
    let context = JobContext {
        tenant_id: tenant_id.to_string(),
        repo_id: repo_id.to_string(),
        branch: branch.to_string(),
        // Full-text maintenance is branch-wide: it walks every workspace.
        workspace_id: String::new(),
        revision: raisin_hlc::HLC::new(0, 0),
        metadata,
    };

    let job_id = JobId::new();
    storage
        .job_data_store()
        .put(&job_id, &context)
        .map_err(|e| Error::storage(format!("Failed to store job context: {e}")))?;
    storage
        .job_registry()
        .register_job_with_id(
            job_id.clone(),
            job_type,
            tenant_id.to_string(),
            None,
            None,
            Some(0),
        )
        .await
        .map_err(|e| Error::storage(format!("Failed to register job: {e}")))?;
    Ok(job_id)
}

/// A full-text rebuild queued for one branch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReindexJob {
    pub branch: String,
    pub job_id: String,
}

/// Queue a full-text REBUILD of every branch of the repository.
///
/// Used after a default-language change, so base content moves from the old
/// language's documents to the new one's. A rebuild (not a reconcile) is what
/// removes the old documents: it recreates the branch's index directory.
pub async fn enqueue_fulltext_rebuild_all_branches(
    storage: &RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
) -> Result<Vec<ReindexJob>> {
    let mut branches = list_branches(storage, tenant_id, repo_id).await?;
    branches.sort();
    branches.dedup();

    let mut jobs = Vec::with_capacity(branches.len());
    for branch in branches {
        let mut metadata = HashMap::new();
        metadata.insert(
            META_REINDEX_REASON.to_string(),
            serde_json::json!(REINDEX_REASON_DEFAULT_LANGUAGE),
        );
        let job_id = enqueue_fulltext_job(
            storage,
            JobType::FulltextRebuild,
            tenant_id,
            repo_id,
            &branch,
            metadata,
        )
        .await?;
        jobs.push(ReindexJob {
            branch,
            job_id: job_id.0,
        });
    }
    Ok(jobs)
}

/// Metadata for the repository `Updated` event of a default-language change.
pub fn default_language_changed_metadata(
    previous: &str,
    current: &str,
) -> HashMap<String, serde_json::Value> {
    let mut metadata = HashMap::new();
    metadata.insert(
        META_DEFAULT_LANGUAGE_CHANGED.to_string(),
        serde_json::Value::Bool(true),
    );
    metadata.insert(
        META_PREVIOUS_DEFAULT_LANGUAGE.to_string(),
        serde_json::json!(previous),
    );
    metadata.insert(
        META_DEFAULT_LANGUAGE.to_string(),
        serde_json::json!(current),
    );
    metadata.insert("source".to_string(), serde_json::json!("replication"));
    metadata
}

/// Rebuilds the full-text index on a replication peer after a replicated
/// default-language change.
///
/// The node that took the change queues its own rebuilds and reports them to
/// the caller; this handler covers the peers, which learn about the change only
/// through the applied `UpdateRepository` operation (see the module docs).
pub struct DefaultLanguageReindexHandler {
    storage: Arc<RocksDBStorage>,
}

impl DefaultLanguageReindexHandler {
    pub fn new(storage: Arc<RocksDBStorage>) -> Self {
        Self { storage }
    }
}

impl EventHandler for DefaultLanguageReindexHandler {
    fn name(&self) -> &str {
        "default_language_reindex"
    }

    fn handle<'a>(
        &'a self,
        event: &'a Event,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let Event::Repository(repo_event) = event else {
                return Ok(());
            };
            if repo_event.kind != RepositoryEventKind::Updated {
                return Ok(());
            }
            let changed = repo_event
                .metadata
                .as_ref()
                .and_then(|m| m.get(META_DEFAULT_LANGUAGE_CHANGED))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if !changed {
                return Ok(());
            }
            let (tenant, repo) = (&repo_event.tenant_id, &repo_event.repository_id);
            match enqueue_fulltext_rebuild_all_branches(&self.storage, tenant, repo).await {
                Ok(jobs) => tracing::info!(
                    %tenant,
                    %repo,
                    jobs = jobs.len(),
                    "Replicated default-language change: queued full-text rebuilds"
                ),
                Err(e) => tracing::error!(
                    %tenant,
                    %repo,
                    error = %e,
                    "Replicated default-language change: could not queue full-text rebuilds"
                ),
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nth_nul_end_stops_before_a_binary_revision() {
        let mut key = b"t\0r\0main\0ws\0translations\0n1\0de\0".to_vec();
        key.extend_from_slice(&[0, 0, 7, 0]);
        let end = nth_nul_end(&key, 7).unwrap();
        assert_eq!(&key[..end], b"t\0r\0main\0ws\0translations\0n1\0de\0");
    }

    #[test]
    fn deleted_and_tombstoned_overlays_are_not_live() {
        assert!(!is_live_overlay(TOMBSTONE));
        let empty = serde_json::to_vec(&LocaleOverlay::properties(HashMap::new())).unwrap();
        assert!(!is_live_overlay(&empty));
        let hidden = serde_json::to_vec(&LocaleOverlay::hidden()).unwrap();
        assert!(is_live_overlay(&hidden));
    }
}
