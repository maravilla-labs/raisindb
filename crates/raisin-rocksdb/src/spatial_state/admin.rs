//! The RocksDB implementation of [`SpatialIndexAdmin`].
//!
//! Three jobs, in order of how much they are trusted:
//!
//! 1. **State** — read the local `SpatialIndexState` records. Cheap, but it only
//!    reports what the index *claims*.
//! 2. **Census** — count the keys that are actually there. This is the part that
//!    makes `SHOW SPATIAL INDEX HEALTH` a statement about reality rather than
//!    about intent, and it is deliberately an O(entries-in-property) scan: an
//!    admin statement is allowed to scan, a query is not.
//! 3. **Queue a build** — hand the work to the job system, which serialises per
//!    (workspace, property) and is idempotent on its dedup key.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use raisin_error::{Error, Result};
use raisin_storage::spatial::SpatialIndexState;
use raisin_storage::spatial_admin::{SpatialEntryCensus, SpatialIndexAdmin};
use rocksdb::DB;

use crate::indexing::SPATIAL_TOMBSTONE;
use crate::repositories::spatial_index::SpatialEntry;
use crate::spatial_state::SpatialStateStore;
use crate::{cf, cf_handle, keys};

/// Admin handle over one node's spatial index.
///
/// Holds the job-enqueue closure rather than the whole `RocksDBStorage`, because
/// `RocksDBStorage` is not object-safe to hand out from a `&self` accessor and the
/// admin surface needs exactly two capabilities: read the CFs, and queue a job.
pub struct SpatialAdminStore {
    db: Arc<DB>,
    state: SpatialStateStore,
    /// Enqueues a `JobType::SpatialIndexBuild`. `None` when the job system has not
    /// been initialised, in which case a rebuild request errors loudly rather than
    /// silently doing nothing — the `BulkSql` debug-stub incident is the precedent.
    enqueue: Option<Arc<dyn BuildEnqueuer>>,
}

/// Indirection so this module does not need to know about `JobRegistry`.
#[async_trait]
pub trait BuildEnqueuer: Send + Sync {
    async fn enqueue(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        property: Option<&str>,
        rebuild: bool,
    ) -> Result<String>;
}

impl SpatialAdminStore {
    pub fn new(db: Arc<DB>, enqueue: Option<Arc<dyn BuildEnqueuer>>) -> Self {
        Self {
            state: SpatialStateStore::new(db.clone()),
            db,
            enqueue,
        }
    }

    /// Census one property by walking its key range.
    ///
    /// Resolution rule, matching the query scan exactly: keys sort
    /// `{geohash}\0{~revision}\0{node_id}`, and `~revision` is a *descending*
    /// encoding, so within a cell the newest revision for a node is the first one
    /// seen. The first entry observed for a `(cell, node)` pair therefore decides
    /// whether that pair is live or tombstoned, and every later entry for the same
    /// pair is a superseded revision. Counting raw keys instead would report
    /// history as if it were live data.
    fn census_property(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        property: &str,
    ) -> Result<SpatialEntryCensus> {
        let cf = cf_handle(self.db.as_ref(), cf::SPATIAL_INDEX)?;
        let prefix =
            keys::spatial_index_property_prefix(tenant_id, repo_id, branch, workspace, property);

        let mut census = SpatialEntryCensus {
            property: property.to_string(),
            ..Default::default()
        };
        let mut seen: HashSet<(String, String)> = HashSet::new();
        let mut nodes: HashSet<String> = HashSet::new();
        let mut per_precision: BTreeMap<usize, u64> = BTreeMap::new();
        let mut hashes: BTreeSet<u64> = BTreeSet::new();

        for item in self.db.prefix_iterator_cf(cf, &prefix) {
            let (key, value) =
                item.map_err(|e| Error::storage(format!("Failed to scan spatial index: {}", e)))?;
            if !key.starts_with(&prefix) {
                break;
            }
            census.raw_keys_scanned += 1;

            // Never split a spatial key on `\0`: the descending HLC contains
            // embedded nulls, so `parts[6]` is not reliably the geohash.
            let Some(parsed) = keys::parse_spatial_index_key(&key) else {
                continue;
            };
            if parsed.property_name != property {
                continue;
            }

            if !seen.insert((parsed.geohash.to_string(), parsed.node_id.to_string())) {
                continue; // superseded revision of a pair already decided
            }

            if value.as_ref() == SPATIAL_TOMBSTONE {
                census.tombstoned_entries += 1;
                continue;
            }

            census.live_entries += 1;
            *per_precision.entry(parsed.precision()).or_insert(0) += 1;
            nodes.insert(parsed.node_id.to_string());

            match SpatialEntry::decode(&value) {
                Some(entry) => {
                    if entry.is_legacy() {
                        census.legacy_entries += 1;
                    }
                    hashes.insert(entry.policy_hash);
                }
                // An undecodable value still occupies a key, so it counts as live;
                // reporting it as absent would understate what a rebuild must fix.
                None => census.legacy_entries += 1,
            }
        }

        census.distinct_nodes = nodes.len() as u64;
        census.live_per_precision = per_precision.into_iter().collect();
        census.policy_hashes = hashes.into_iter().collect();
        Ok(census)
    }

    /// Every property that has at least one key in the workspace's spatial range.
    ///
    /// Read off the keys rather than off the state records, so a property whose
    /// state record is missing (the pre-existing-data case) is still censused.
    fn properties_with_entries(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
    ) -> Result<BTreeSet<String>> {
        let cf = cf_handle(self.db.as_ref(), cf::SPATIAL_INDEX)?;
        let prefix = keys::spatial_index_workspace_prefix(tenant_id, repo_id, branch, workspace);

        let mut out = BTreeSet::new();
        for item in self.db.prefix_iterator_cf(cf, &prefix) {
            let (key, _) =
                item.map_err(|e| Error::storage(format!("Failed to scan spatial index: {}", e)))?;
            if !key.starts_with(&prefix) {
                break;
            }
            if let Some(parsed) = keys::parse_spatial_index_key(&key) {
                out.insert(parsed.property_name.to_string());
            }
        }
        Ok(out)
    }
}

#[async_trait]
impl SpatialIndexAdmin for SpatialAdminStore {
    fn states(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
    ) -> Result<Vec<(String, SpatialIndexState)>> {
        self.state
            .list_for_workspace(tenant_id, repo_id, branch, workspace)
    }

    fn census(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        property: Option<&str>,
    ) -> Result<Vec<SpatialEntryCensus>> {
        let properties: BTreeSet<String> = match property {
            Some(p) => std::iter::once(p.to_string()).collect(),
            None => {
                let mut all =
                    self.properties_with_entries(tenant_id, repo_id, branch, workspace)?;
                // A property with a state record but no entries yet is worth
                // reporting as zero rather than omitting.
                for (name, _) in self
                    .state
                    .list_for_workspace(tenant_id, repo_id, branch, workspace)?
                {
                    all.insert(name);
                }
                all
            }
        };

        properties
            .into_iter()
            .map(|p| self.census_property(tenant_id, repo_id, branch, workspace, &p))
            .collect()
    }

    async fn queue_build(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        property: Option<&str>,
        rebuild: bool,
    ) -> Result<String> {
        let enqueue = self.enqueue.as_ref().ok_or_else(|| {
            Error::Validation(
                "Spatial index rebuild requested but the job system is not initialised on this node"
                    .to_string(),
            )
        })?;
        enqueue
            .enqueue(tenant_id, repo_id, branch, workspace, property, rebuild)
            .await
    }
}
