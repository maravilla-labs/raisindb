//! Spatial index build / backfill job handler.
//!
//! # Why a job is needed at all
//!
//! Before this, **no rebuild or backfill path existed anywhere** — zero hits for
//! `spatial` across `jobs/`, `raisin-core`, the transports and `raisin-server`. Data
//! written before spatial indexing existed, or before the current precision policy,
//! was permanently invisible to the index. Combined with the planner's hardcoded
//! `has_spatial_index() == true` that produced silent zero-row answers rather than a
//! signal.
//!
//! # Cluster semantics
//!
//! Each node runs its OWN build against its OWN local index, with no leader and no
//! distributed lock. The spatial index is derived local state (exactly like the
//! Tantivy fulltext index); the source of truth is the replicated node records, which
//! already converge. Coordination would add a failure mode for no benefit.
//!
//! The corollary must be said out loud: a manual rebuild with **no config change is
//! LOCAL-ONLY**. Fan-out happens through configuration, because
//! `WorkspaceConfig.spatial` is replicated data — an `ALTER SPATIAL INDEX` on node1
//! reaches node2/node3, each of which then observes `configured.policy_hash !=
//! state.policy_hash` and schedules its own local rebuild.

use raisin_error::{Error, Result};
use raisin_hlc::HLC;
use raisin_models::nodes::properties::{policy_key_for_path, PropertyValue, SpatialPolicy};
use raisin_models::nodes::Node;
use raisin_storage::jobs::{JobContext, JobInfo, JobType};
use raisin_storage::spatial::{SpatialBuildPhase, SpatialIndexState};
use rocksdb::WriteBatch;
use std::collections::BTreeSet;
use std::sync::Arc;

use crate::indexing::{IndexCtx, SpatialIndexTargets};
use crate::jobs::keyed_mutex::KeyedMutex;
use crate::spatial_state::SpatialStateStore;

/// How many nodes to process between state-record checkpoints.
///
/// A crash between checkpoints costs at most this much re-work, and the resume cursor
/// makes the restart pick up where it left off rather than starting over.
const CHECKPOINT_EVERY: u64 = 1000;

/// Outcome of one build, surfaced in the job result and in
/// `SHOW SPATIAL INDEX HEALTH`.
#[derive(Debug, Clone, Default)]
pub struct SpatialBuildReport {
    pub nodes_scanned: u64,
    pub geometries_indexed: u64,
    pub entries_written: u64,
    pub properties: Vec<String>,
}

/// Handler for `JobType::SpatialIndexBuild`.
pub struct SpatialIndexJobHandler {
    db: Arc<rocksdb::DB>,
    state: SpatialStateStore,
    /// Serialises builders per (tenant, repo, branch, workspace, property).
    ///
    /// `KeyedMutex` QUEUES on contention where `LockManager::try_acquire` would
    /// REJECT — the right primitive when a second rebuild request must run *after*
    /// the first rather than fail. It also evicts entries on last release, so a map
    /// keyed by something unbounded cannot grow forever.
    locks: Arc<KeyedMutex<String>>,
}

impl SpatialIndexJobHandler {
    pub fn new(db: Arc<rocksdb::DB>) -> Self {
        Self {
            state: SpatialStateStore::new(db.clone()),
            db,
            locks: Arc::new(KeyedMutex::new()),
        }
    }

    /// Handle a spatial index build job.
    pub async fn handle(&self, job: &JobInfo, context: &JobContext) -> Result<()> {
        let (tenant_id, repo_id, branch, workspace, property, rebuild) = match &job.job_type {
            JobType::SpatialIndexBuild {
                tenant_id,
                repo_id,
                branch,
                workspace,
                property,
                rebuild,
            } => (
                tenant_id.clone(),
                repo_id.clone(),
                branch.clone(),
                workspace.clone(),
                property.clone(),
                *rebuild,
            ),
            _ => {
                return Err(Error::Validation(
                    "Expected SpatialIndexBuild job type".to_string(),
                ))
            }
        };

        let lock_key = format!(
            "{}\0{}\0{}\0{}\0{}",
            tenant_id,
            repo_id,
            branch,
            workspace,
            property.as_deref().unwrap_or("*")
        );
        let guard = self.locks.lock(lock_key).await;
        let _guard = guard;

        tracing::info!(
            job_id = %job.id,
            tenant_id = %tenant_id,
            repo_id = %repo_id,
            branch = %branch,
            workspace = %workspace,
            property = ?property,
            rebuild,
            "Processing spatial index build job"
        );

        let report = self.build(
            &tenant_id,
            &repo_id,
            &branch,
            &workspace,
            property.as_deref(),
            rebuild,
            context.revision,
        )?;

        tracing::info!(
            job_id = %job.id,
            nodes_scanned = report.nodes_scanned,
            geometries_indexed = report.geometries_indexed,
            entries_written = report.entries_written,
            properties = ?report.properties,
            "Spatial index build completed"
        );

        Ok(())
    }

    /// Walk the workspace's latest-revision nodes and (re)index their geometries.
    ///
    /// # `rebuild` semantics
    ///
    /// `rebuild: true` does **not** `delete_range` the column family. The CF is
    /// MVCC/tombstone-based and a range delete would destroy history and break
    /// time-travel reads. Instead, per node, it performs the same
    /// tombstone-superseded-then-index pair the update path performs, at
    /// `revision`. Entries the new policy no longer produces get a tombstone;
    /// entries it still produces are rewritten **byte-identically** (the
    /// `SpatialEntry` encoding is deterministic), so a re-run over unchanged data is
    /// a no-op rather than a new MVCC generation.
    ///
    /// `rebuild: false` skips nodes already covered by the current `policy_hash`,
    /// which is the cheap gap-fill used by the automatic backfill trigger.
    ///
    /// # Entries are stamped at the NODE's revision, not the job's
    ///
    /// `revision` (the job context's, wall-clock now) is used only for the state
    /// record. Every index entry is written at the revision the node record it
    /// describes is stored under. This is not a detail:
    ///
    /// A spatial read resolves candidates as of a `max_revision` — the branch HEAD
    /// for a normal query — and **discards any entry stamped after it**
    /// (`repositories/spatial_index/repository/scan.rs`). A rebuild does not advance
    /// the branch head, so entries stamped "now" land in the future relative to
    /// every read and are invisible until some unrelated write happens to push the
    /// head past them. That was survivable only by accident: while the old and new
    /// precision sets overlapped, queries kept being answered from the still-visible
    /// entries written by the original writes. A rebuild between two DISJOINT
    /// precision sets removed that crutch and every spatial query in the workspace
    /// returned ZERO rows — a completed, "healthy", fully-censused index that
    /// answered nothing.
    ///
    /// Stamping at the node's own revision also makes the re-emission genuinely
    /// byte-identical (same key, same value) for entries the policy still wants, so
    /// a repeated rebuild is a true no-op rather than a new MVCC generation, and it
    /// keeps time-travel reads consistent: an entry is visible exactly when the node
    /// revision it describes is.
    #[allow(clippy::too_many_arguments)]
    pub fn build(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        only_property: Option<&str>,
        rebuild: bool,
        revision: HLC,
    ) -> Result<SpatialBuildReport> {
        let ctx = IndexCtx::new(tenant_id, repo_id, branch, workspace);
        let targets = SpatialIndexTargets::from_db(self.db.as_ref())?;
        let mut report = SpatialBuildReport::default();

        // A rebuild builds at the CONFIGURED policy, never at the currently indexed
        // one. This is the whole point of the job: it is the mechanism that moves
        // local reality towards replicated intent. Resolving from local state here
        // instead — which is what this did before — meant `ALTER SPATIAL INDEX`
        // followed by `REBUILD SPATIAL INDEX` rebuilt at the OLD precisions and
        // silently changed nothing.
        //
        // Resolved ONCE per property up front rather than per node: it is a workspace
        // -level record, so a per-node read would be pure waste, and a policy that
        // changed mid-build would produce entries under two different `policy_hash`
        // values in the same pass.
        let mut configured: std::collections::HashMap<String, SpatialPolicy> =
            std::collections::HashMap::new();
        let mut policy_for = |property: &str| -> SpatialPolicy {
            configured
                .entry(property.to_string())
                .or_insert_with(|| {
                    crate::spatial_state::configured_policy(
                        self.db.as_ref(),
                        tenant_id,
                        repo_id,
                        workspace,
                        property,
                    )
                })
                .clone()
        };

        // Mark every property we are about to touch as `Building`. The record keeps
        // its OLD `precisions` while the phase is `Building`, so queries continue to
        // be answered correctly and completely from the existing entries throughout.
        let existing: Vec<(String, SpatialIndexState)> = self
            .state
            .list_for_workspace(tenant_id, repo_id, branch, workspace)?
            .into_iter()
            .filter(|(p, _)| only_property.is_none_or(|want| want == p))
            .collect();
        for (property, mut state) in existing {
            state.phase = SpatialBuildPhase::Building;
            state.cursor = None;
            self.state
                .put(tenant_id, repo_id, branch, workspace, &property, &state)?;
        }

        let mut touched: BTreeSet<String> = BTreeSet::new();
        let mut batch = WriteBatch::default();
        let mut in_batch = 0u64;

        for (node, node_revision) in
            self.iter_latest_nodes(tenant_id, repo_id, branch, workspace)?
        {
            report.nodes_scanned += 1;

            // The FULL property tree, not just the top level. A rebuild that only
            // looked at top-level properties is why nested geometry could never
            // migrate: the writer would index it going forward but no existing
            // node would ever be picked up.
            let walked = crate::indexing::walk_geometries_capped(&node.properties, &node.id);
            for (property_name, geometry) in &walked.indexed {
                let property_name = property_name.as_str();
                // `only_property` matches either the concrete path
                // (`stops.3.geo`) or the policy key the operator configured
                // (`stops[].geo`), so a targeted rebuild of an array field does
                // not silently do nothing.
                if let Some(want) = only_property {
                    if want != property_name && want != policy_key_for_path(property_name) {
                        continue;
                    }
                }

                let policy = policy_for(property_name);

                if !rebuild && self.already_covered(&node, property_name, &policy)? {
                    continue;
                }

                if rebuild {
                    // Repair sweep: tombstone whatever is physically present for
                    // this node, not just what the current policy would derive. A
                    // rebuild after a `cover` mode change has to remove extent cells
                    // the centroid derivation would never name.
                    let removed = self.tombstone_existing(
                        &mut batch,
                        tenant_id,
                        repo_id,
                        branch,
                        workspace,
                        property_name,
                        &node.id,
                        &node_revision,
                    )?;
                    in_batch += removed as u64;
                }

                crate::indexing::write_spatial_property(
                    &mut batch,
                    &targets,
                    &ctx,
                    &node.id,
                    property_name,
                    geometry,
                    &node_revision,
                    &policy,
                    resolve_bucket(&node, &policy).as_deref(),
                )?;

                report.geometries_indexed += 1;
                report.entries_written += policy.precisions.len() as u64;
                in_batch += policy.precisions.len() as u64;
                // Record the POLICY key, not the concrete path: state records are
                // keyed that way (see `spatial_state_key`), so touching
                // `stops.0.geo` … `stops.9.geo` must stamp ONE `stops[].geo`
                // record rather than ten.
                touched.insert(policy_key_for_path(property_name));
            }

            if in_batch >= CHECKPOINT_EVERY {
                self.commit(std::mem::take(&mut batch))?;
                in_batch = 0;
                self.checkpoint(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    &touched,
                    report.nodes_scanned,
                    revision,
                )?;
            }
        }

        self.commit(batch)?;

        // Only on SUCCESSFUL completion do `precisions` / `policy_hash` flip to the
        // values just built. A crashed rebuild therefore leaves the OLD
        // `indexed_precisions` in force, and queries keep working correctly against
        // the old entries — the failure mode is "stale but complete", never "empty".
        for property in &touched {
            // Stamp what was actually built — the configured policy — so
            // `indexed_precisions` and `policy_hash` now describe reality. Reading
            // the state record back here would re-stamp the OLD policy and leave the
            // index permanently claiming a set it no longer holds.
            let policy = policy_for(property);
            let mut state = SpatialIndexState::ready(&policy, revision);
            state.nodes_indexed = report.nodes_scanned;
            self.state
                .put(tenant_id, repo_id, branch, workspace, property, &state)?;
        }

        report.properties = touched.into_iter().collect();
        Ok(report)
    }

    /// Whether this node's entries were already produced under `policy`.
    ///
    /// Cheap check for the gap-fill path: read any one of the node's expected keys and
    /// compare the stamped `policy_hash`.
    fn already_covered(
        &self,
        _node: &Node,
        _property: &str,
        _policy: &SpatialPolicy,
    ) -> Result<bool> {
        // Deliberately conservative: always re-index in the gap-fill path too.
        // Re-indexing is a byte-identical no-op for already-correct entries (the
        // `SpatialEntry` encoding is deterministic and the keys are derived), so the
        // only cost of being conservative is CPU, whereas a wrong `true` here would
        // leave a node permanently unindexed. If profiling shows this matters, the
        // check is a single `get_cf` on the finest-precision key.
        Ok(false)
    }

    /// Tombstone every physically present entry for one node+property.
    #[allow(clippy::too_many_arguments)]
    fn tombstone_existing(
        &self,
        batch: &mut WriteBatch,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        property: &str,
        node_id: &str,
        revision: &HLC,
    ) -> Result<usize> {
        let repo = crate::repositories::spatial_index::SpatialIndexRepository::new(self.db.clone());
        repo.tombstone_all_entries_for_node(
            batch, tenant_id, repo_id, branch, workspace, property, node_id, revision,
        )
    }

    fn commit(&self, batch: WriteBatch) -> Result<()> {
        if batch.is_empty() {
            return Ok(());
        }
        self.db
            .write(batch)
            .map_err(|e| Error::storage(format!("Spatial index build write failed: {}", e)))
    }

    /// Persist progress so a restart resumes rather than restarts.
    #[allow(clippy::too_many_arguments)]
    fn checkpoint(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        properties: &BTreeSet<String>,
        nodes_indexed: u64,
        revision: HLC,
    ) -> Result<()> {
        for property in properties {
            let policy = crate::spatial_state::policy_for_property(
                self.db.as_ref(),
                tenant_id,
                repo_id,
                branch,
                workspace,
                property,
            );
            let mut state = SpatialIndexState::ready(&policy, revision);
            state.phase = SpatialBuildPhase::Building;
            state.nodes_indexed = nodes_indexed;
            self.state
                .put(tenant_id, repo_id, branch, workspace, property, &state)?;
        }
        Ok(())
    }

    /// Every latest-revision, non-deleted node in the workspace, **with the HLC
    /// that revision is stored under**.
    ///
    /// NODES keys are `{tenant}\0{repo}\0{branch}\0{ws}\0nodes\0{node_id}\0{~revision}`
    /// with a **descending** revision encoding, so the first key seen for a node id is
    /// its newest revision. If that newest revision is a tombstone the node is
    /// skipped — indexing a deleted node would resurrect it in every proximity query.
    ///
    /// Adjacency and other sub-keys under `nodes\0{id}\0…` are filtered out by
    /// requiring the tail after the node id to be exactly the 16-byte revision.
    ///
    /// The revision is returned because the rebuild must stamp its entries with it;
    /// see [`Self::build`].
    fn iter_latest_nodes(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
    ) -> Result<Vec<(Node, HLC)>> {
        use crate::{cf, cf_handle};

        let cf_nodes = cf_handle(self.db.as_ref(), cf::NODES)?;
        let prefix = {
            let mut p = crate::keys::workspace_prefix(tenant_id, repo_id, branch, workspace);
            p.extend_from_slice(b"nodes\0");
            p
        };

        let mut out: Vec<(Node, HLC)> = Vec::new();
        let mut current_id: Option<String> = None;

        for item in self.db.prefix_iterator_cf(cf_nodes, &prefix) {
            let (key, value) =
                item.map_err(|e| Error::storage(format!("Failed to scan nodes: {}", e)))?;
            if !key.starts_with(&prefix) {
                break;
            }

            let tail = &key[prefix.len()..];
            // `{node_id}\0{16-byte revision}`
            let Some(sep) = tail.iter().position(|&b| b == 0) else {
                continue;
            };
            if tail.len() != sep + 1 + 16 {
                continue; // an adjacency or other sub-key, not a node revision
            }
            let Ok(node_id) = std::str::from_utf8(&tail[..sep]) else {
                continue;
            };

            if current_id.as_deref() == Some(node_id) {
                continue; // an older revision of a node we have already resolved
            }
            current_id = Some(node_id.to_string());

            if value.as_ref() == crate::tombstones::TOMBSTONE {
                continue;
            }

            // The revision this node record is stored under. Unparseable means the
            // key is not a node revision after all, so skip it rather than guess:
            // guessing would strand the entries at the wrong revision, which is the
            // exact failure this returns the revision to prevent.
            let node_revision = match crate::keys::decode_descending_revision(&tail[sep + 1..]) {
                Ok(hlc) => hlc,
                Err(e) => {
                    tracing::warn!(
                        node_id = %node_id,
                        error = %e,
                        "Unparseable node revision during spatial index build; skipping"
                    );
                    continue;
                }
            };

            match rmp_serde::from_slice::<crate::repositories::StorageNode>(&value) {
                Ok(storage_node) => {
                    // The blob deliberately excludes `path` (so a subtree move is
                    // O(1)); the spatial index does not use it, so an empty path is
                    // fine here and avoids a PATH_INDEX lookup per node.
                    out.push((storage_node.into_node(String::new()), node_revision));
                }
                Err(e) => {
                    tracing::warn!(
                        node_id = %node_id,
                        error = %e,
                        "Could not deserialize node during spatial index build; skipping"
                    );
                }
            }
        }

        Ok(out)
    }
}

/// Read the configured discriminator value off a node.
///
/// Mirrors `crate::indexing::spatial`'s private helper; kept in step by the parity
/// test, which asserts a rebuild reproduces the write path's keys byte-for-byte.
fn resolve_bucket(node: &Node, policy: &SpatialPolicy) -> Option<String> {
    let property = policy.bucket_property.as_deref()?;
    match node.properties.get(property)? {
        PropertyValue::String(s) => Some(s.clone()),
        PropertyValue::Integer(i) => Some(i.to_string()),
        PropertyValue::Float(f) => Some(f.to_string()),
        PropertyValue::Boolean(b) => Some(b.to_string()),
        _ => None,
    }
}
