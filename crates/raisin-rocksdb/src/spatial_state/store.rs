//! Read/write of [`SpatialIndexState`] in the `INDEX_STATUS` column family.

use super::resolve::{
    availability_in_rebuild_window, initial_state, needs_configured_check, resolve_write_policy,
    WritePolicyResolution,
};
use crate::{cf, cf_handle};
use raisin_error::{Error, Result};
use raisin_models::nodes::properties::{policy_key_for_path, SpatialPolicy};
use raisin_storage::spatial::{
    SpatialAvailability, SpatialBuildPhase, SpatialIndexState, SpatialStateSource,
};
use rocksdb::{WriteBatch, DB};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// Build the `INDEX_STATUS` key for one (workspace, property) spatial index.
///
/// Shape mirrors the existing `prop_index\0{tenant}\0{repo}\0{branch}\0{ws}`
/// watermark key, with the property appended.
///
/// # Nested paths collapse here, and ONLY here
///
/// `property` may be any geometry property path the walker produces
/// (`location`, `venue.geo`, `stops.3.geo`). It is normalised through
/// [`policy_key_for_path`] before being embedded, so every element of an array
/// shares ONE state record under `stops[].geo` — otherwise an array of stops
/// would mint an unbounded number of state records that no operator could
/// configure or rebuild.
///
/// Every reader and writer of the record goes through this function, including
/// the query planner's `spatial_availability`. That is what stops the planner
/// from asking about `stops.3.geo`, missing the record written under
/// `stops[].geo`, and concluding the field is unindexed forever — correct
/// results, permanently bad performance, no error anywhere.
///
/// **The index ENTRY key is unaffected**: it keeps the concrete path
/// (`keys::spatial_index_key_versioned`), so a query naming `stops.3.geo` scans
/// exactly that element's cells.
pub fn spatial_state_key(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    property: &str,
) -> Vec<u8> {
    format!(
        "spatial_index\0{}\0{}\0{}\0{}\0{}",
        tenant_id,
        repo_id,
        branch,
        workspace,
        policy_key_for_path(property)
    )
    .into_bytes()
}

/// Read a state record from a borrowed database handle.
///
/// The uncached primitive. Exists because the tombstone path holds a `&DB` rather
/// than an `Arc<DB>`, and threading ownership through every caller of the deletion
/// machinery to satisfy a cache would be a large diff for no benefit — a delete is
/// already doing far more work than one point lookup.
pub fn read_state(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    property: &str,
) -> Result<Option<SpatialIndexState>> {
    let cf = cf_handle(db, cf::INDEX_STATUS)?;
    let key = spatial_state_key(tenant_id, repo_id, branch, workspace, property);
    match db
        .get_cf(cf, &key)
        .map_err(|e| Error::storage(format!("Failed to read spatial index state: {}", e)))?
    {
        Some(bytes) => Ok(rmp_serde::from_slice::<SpatialIndexState>(&bytes).ok()),
        None => Ok(None),
    }
}

/// Resolve the policy an existing index was built under, or the default.
///
/// Never fails: an unreadable record falls back to the default policy, and because
/// tombstoning widens to every precision in
/// [`raisin_models::nodes::properties::PRECISION_RANGE`] a wrong guess here cannot
/// strand a live stale entry.
pub fn policy_for_property(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    property: &str,
) -> SpatialPolicy {
    match read_state(db, tenant_id, repo_id, branch, workspace, property) {
        Ok(Some(state)) => state.policy(),
        _ => SpatialPolicy::default(),
    }
}

/// The policy and precision set a **delete** must tombstone one property path at.
///
/// The delete path holds only a `&DB` (see [`read_state`] for why), so it cannot
/// go through [`SpatialStateStore`]. It still needs the same bound the update path
/// gets from [`crate::indexing::NodeSpatialPolicies::tombstone_precisions`]:
/// `configured ∪ indexed`, which is exactly the set any respecting writer can have
/// produced.
///
/// Falls back to `None` precisions — meaning "tombstone at every precision" — when
/// there is no state record, because then nothing is known about what is
/// physically present and under-tombstoning would leave a deleted node matching
/// `ST_DWITHIN` forever.
pub fn tombstone_policy_for_property(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    property: &str,
) -> (SpatialPolicy, Option<Vec<usize>>) {
    let configured =
        super::configured::configured_policy(db, tenant_id, repo_id, workspace, property);
    let state = read_state(db, tenant_id, repo_id, branch, workspace, property)
        .ok()
        .flatten();
    let has_state = state.is_some();
    let resolution = resolve_write_policy(configured, state.as_ref());
    if !has_state {
        return (resolution.write, None);
    }
    let mut union = resolution.write.precisions.clone();
    union.extend_from_slice(&resolution.configured.precisions);
    let precisions = raisin_models::nodes::properties::sorted_precisions(union);
    (resolution.write, Some(precisions))
}

/// Cached reader/writer for spatial index state records.
///
/// Reads go through an in-process cache because the query planner consults
/// availability on **every** spatial query; a RocksDB point lookup per query
/// would be a needless tax. The cache is invalidated on write, and it is keyed by
/// the full state key so multi-tenant isolation is preserved.
#[derive(Clone)]
pub struct SpatialStateStore {
    db: Arc<DB>,
    cache: Arc<RwLock<HashMap<Vec<u8>, Option<Arc<SpatialIndexState>>>>>,
}

impl SpatialStateStore {
    pub fn new(db: Arc<DB>) -> Self {
        Self {
            db,
            cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// The **configured** policy for a property — replicated intent, read from the
    /// workspace record.
    ///
    /// Exposed here so the synchronous write path can fall back to declared intent
    /// when no local state record exists yet, instead of silently indexing a
    /// brand-new workspace at the server defaults and ignoring the configuration
    /// the operator already set. One point lookup; not cached, because this store
    /// is constructed per write and a cache would only add a staleness window.
    pub fn configured_policy(
        &self,
        tenant_id: &str,
        repo_id: &str,
        workspace: &str,
        property: &str,
    ) -> SpatialPolicy {
        super::configured::configured_policy(
            self.db.as_ref(),
            tenant_id,
            repo_id,
            workspace,
            property,
        )
    }

    /// Resolve what a write to this (workspace, property) must emit, and whether
    /// a build is owed.
    ///
    /// **Configured intent is the source of truth here**; the local state record
    /// contributes only the precisions that already have entries (so the union
    /// keeps them findable) and the build phase. Resolving the write policy from
    /// the state record instead — which is what this used to do — made an
    /// operator's precision change take effect on exactly nothing.
    ///
    /// Costs one point lookup on the workspace record plus one on the state
    /// record, per geometry property, per write.
    pub fn write_policy(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        property: &str,
    ) -> WritePolicyResolution {
        let configured = self.configured_policy(tenant_id, repo_id, workspace, property);
        let state = self
            .get(tenant_id, repo_id, branch, workspace, property)
            .ok()
            .flatten();
        resolve_write_policy(configured, state.as_deref())
    }

    /// Read the state record, consulting the cache first.
    pub fn get(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        property: &str,
    ) -> Result<Option<Arc<SpatialIndexState>>> {
        let key = spatial_state_key(tenant_id, repo_id, branch, workspace, property);

        if let Ok(cache) = self.cache.read() {
            if let Some(hit) = cache.get(&key) {
                return Ok(hit.clone());
            }
        }

        let cf = cf_handle(&self.db, cf::INDEX_STATUS)?;
        let loaded = match self
            .db
            .get_cf(cf, &key)
            .map_err(|e| Error::storage(format!("Failed to read spatial index state: {}", e)))?
        {
            Some(bytes) => match rmp_serde::from_slice::<SpatialIndexState>(&bytes) {
                Ok(state) => Some(Arc::new(state)),
                Err(e) => {
                    // Corrupt record: report it rather than pretending the index
                    // is fine. `availability()` will surface `Unusable`.
                    tracing::warn!(
                        workspace = %workspace,
                        property = %property,
                        error = %e,
                        "Corrupt spatial index state record"
                    );
                    None
                }
            },
            None => None,
        };

        if let Ok(mut cache) = self.cache.write() {
            cache.insert(key, loaded.clone());
        }
        Ok(loaded)
    }

    /// Stage a state record write into `batch`, so the record lands atomically
    /// with the index entries it describes.
    ///
    /// The cache entry is invalidated immediately. That is deliberately
    /// pessimistic: if the batch is later dropped the next read simply re-reads
    /// the (unchanged) record from disk.
    pub fn put_to_batch(
        &self,
        batch: &mut WriteBatch,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        property: &str,
        state: &SpatialIndexState,
    ) -> Result<()> {
        let cf = cf_handle(&self.db, cf::INDEX_STATUS)?;
        let key = spatial_state_key(tenant_id, repo_id, branch, workspace, property);
        let bytes = rmp_serde::to_vec(state).map_err(|e| {
            Error::storage(format!("Failed to serialize spatial index state: {}", e))
        })?;
        batch.put_cf(cf, &key, bytes);
        self.invalidate(&key);
        Ok(())
    }

    /// Write a state record immediately (used by the reindex job's checkpoints).
    pub fn put(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        property: &str,
        state: &SpatialIndexState,
    ) -> Result<()> {
        let mut batch = WriteBatch::default();
        self.put_to_batch(
            &mut batch, tenant_id, repo_id, branch, workspace, property, state,
        )?;
        self.db
            .write(batch)
            .map_err(|e| Error::storage(format!("Failed to write spatial index state: {}", e)))
    }

    /// Ensure a `Ready` record exists for a property that is being written for the
    /// first time, staging it into the caller's batch.
    ///
    /// This is what preserves **zero-opt-in automatic indexing**: the first write
    /// of a geometry to a brand-new workspace creates the record in the same batch,
    /// so the very next query sees `Ready` and takes the index. No admin action,
    /// no config, no migration step.
    ///
    /// Returns the policy the write must emit, which is
    /// `configured ∪ indexed` while the two disagree so the row stays findable
    /// under both the old and the new configuration until a rebuild catches up.
    /// The union rule itself lives in [`super::resolve::union_policy`], shared
    /// with [`Self::write_policy`], so the record this creates and the cells the
    /// write emits cannot disagree.
    ///
    /// A record that already exists is left **untouched**, including when the
    /// configured policy has moved on: only the `SpatialIndexBuild` job may flip
    /// `precisions` / `policy_hash`, because only it has actually rewritten the
    /// entries those fields describe.
    #[allow(clippy::too_many_arguments)]
    pub fn ensure_for_write(
        &self,
        batch: &mut WriteBatch,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        property: &str,
        configured: &SpatialPolicy,
        revision: raisin_hlc::HLC,
    ) -> Result<SpatialPolicy> {
        let existing = self.get(tenant_id, repo_id, branch, workspace, property)?;
        let resolution = resolve_write_policy(configured.clone(), existing.as_deref());

        if existing.is_none() {
            let state = initial_state(&resolution.configured, revision);
            self.put_to_batch(
                batch, tenant_id, repo_id, branch, workspace, property, &state,
            )?;
        }

        Ok(resolution.write)
    }

    /// Mark a property's index as needing a build, without touching entries.
    pub fn mark_not_built(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        property: &str,
    ) -> Result<()> {
        let mut state = match self.get(tenant_id, repo_id, branch, workspace, property)? {
            Some(s) => (*s).clone(),
            None => return Ok(()),
        };
        state.phase = SpatialBuildPhase::NotBuilt;
        self.put(tenant_id, repo_id, branch, workspace, property, &state)
    }

    /// List every spatial index state record for a workspace.
    ///
    /// Used by `SHOW SPATIAL INDEX HEALTH` and by the reindex job when no explicit
    /// property was named.
    pub fn list_for_workspace(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
    ) -> Result<Vec<(String, SpatialIndexState)>> {
        let cf = cf_handle(&self.db, cf::INDEX_STATUS)?;
        let prefix = format!(
            "spatial_index\0{}\0{}\0{}\0{}\0",
            tenant_id, repo_id, branch, workspace
        )
        .into_bytes();

        let mut out = Vec::new();
        for item in self.db.prefix_iterator_cf(cf, &prefix) {
            let (key, value) =
                item.map_err(|e| Error::storage(format!("Failed to list spatial state: {}", e)))?;
            if !key.starts_with(&prefix) {
                break;
            }
            let property = String::from_utf8_lossy(&key[prefix.len()..]).to_string();
            if let Ok(state) = rmp_serde::from_slice::<SpatialIndexState>(&value) {
                out.push((property, state));
            }
        }
        Ok(out)
    }

    fn invalidate(&self, key: &[u8]) {
        if let Ok(mut cache) = self.cache.write() {
            cache.remove(key);
        }
    }
}

impl SpatialStateSource for SpatialStateStore {
    /// # Behaviour while a policy change is in flight
    ///
    /// The raw record's [`SpatialIndexState::availability`] describes what the
    /// index held when it was last built. That is not enough on its own once the
    /// CONFIGURED policy has moved on, because a rebuild tombstones and re-emits
    /// node by node — so mid-build the entry set is a mixture of both policies.
    /// [`availability_in_rebuild_window`] narrows the answer to the precisions
    /// that are provably complete, and returns
    /// [`SpatialAvailability::Unusable`] (planner falls back to a scan with the
    /// spatial predicate retained) when none are. See that function for the full
    /// decision.
    ///
    /// The extra workspace-record lookup that resolving configured intent costs
    /// is taken only when it could change the answer
    /// ([`super::resolve::needs_configured_check`]), so the steady-state query
    /// path is unchanged: one cached state read and nothing else.
    fn spatial_availability(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        property: &str,
    ) -> SpatialAvailability {
        match self.get(tenant_id, repo_id, branch, workspace, property) {
            Ok(Some(state)) => {
                if !needs_configured_check(&state) {
                    return state.availability();
                }
                let configured = self.configured_policy(tenant_id, repo_id, workspace, property);
                availability_in_rebuild_window(&state, &configured)
            }
            // No record: fail CLOSED. Never assume the index works.
            Ok(None) => SpatialAvailability::NotBuilt,
            Err(e) => {
                SpatialAvailability::Unusable(format!("could not read spatial index state: {}", e))
            }
        }
    }
}
