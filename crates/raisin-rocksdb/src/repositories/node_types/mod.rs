//! NodeType repository implementation

mod crud;

use crate::repositories::{BranchRepositoryImpl, RevisionRepositoryImpl};
use crate::{cf, cf_handle, keys};
use chrono::Utc;
use raisin_error::{Error as RaisinError, Result};
use raisin_events::EventBus;
use raisin_hlc::HLC;
use raisin_models::nodes::types::NodeType;
use raisin_models::tree::ChangeOperation;
use raisin_storage::{
    BranchRepository, CommitMetadata, NodeTypeChangeInfo, RevisionMeta, RevisionRepository,
};
use rocksdb::{WriteBatch, DB};
use std::sync::Arc;

pub(super) const TOMBSTONE: &[u8] = b"TOMBSTONE";

#[derive(Clone)]
pub struct NodeTypeRepositoryImpl {
    pub(super) db: Arc<DB>,
    pub(super) revision_repo: Arc<RevisionRepositoryImpl>,
    pub(super) branch_repo: Arc<BranchRepositoryImpl>,
    pub(super) operation_capture: Option<Arc<crate::OperationCapture>>,
    /// Event bus for local `Event::Schema` emission. Optional for the same
    /// reason `operation_capture` is: repositories built for tests, embedded
    /// uses and background jobs pass none and stay silent.
    pub(super) event_bus: Option<Arc<dyn EventBus>>,
}

impl NodeTypeRepositoryImpl {
    pub fn new(
        db: Arc<DB>,
        revision_repo: Arc<RevisionRepositoryImpl>,
        branch_repo: Arc<BranchRepositoryImpl>,
    ) -> Self {
        Self {
            db,
            revision_repo,
            branch_repo,
            operation_capture: None,
            event_bus: None,
        }
    }

    pub fn new_with_capture(
        db: Arc<DB>,
        revision_repo: Arc<RevisionRepositoryImpl>,
        branch_repo: Arc<BranchRepositoryImpl>,
        operation_capture: Arc<crate::OperationCapture>,
    ) -> Self {
        Self {
            db,
            revision_repo,
            branch_repo,
            operation_capture: Some(operation_capture),
            event_bus: None,
        }
    }

    /// Attach the event bus so local schema writes publish `Event::Schema`.
    ///
    /// Without it the schema-stats cache only self-heals on its 5-minute TTL,
    /// and never gets the correctness signal on the originating node.
    pub fn with_event_bus(mut self, event_bus: Arc<dyn EventBus>) -> Self {
        self.event_bus = Some(event_bus);
        self
    }

    pub(super) fn decode_revision(key: &[u8]) -> Result<HLC> {
        // The HLC is always the last 16 bytes of the key
        // We can't split by null bytes because the HLC encoding itself may contain null bytes
        if key.len() < 16 {
            return Err(RaisinError::storage(format!(
                "Invalid nodetype key: too short ({} bytes, need at least 16 for HLC)",
                key.len()
            )));
        }

        let rev_bytes = &key[key.len() - 16..];

        keys::decode_descending_revision(rev_bytes)
            .map_err(|e| RaisinError::storage(format!("Failed to decode nodetype revision: {}", e)))
    }

    pub(super) fn deserialize_node_type(bytes: &[u8]) -> Result<NodeType> {
        tracing::debug!(
            "Attempting to deserialize NodeType, bytes length: {}, first 50 bytes: {:?}",
            bytes.len(),
            &bytes[..std::cmp::min(50, bytes.len())]
        );

        rmp_serde::from_slice::<NodeType>(bytes).map_err(|e| {
            tracing::error!(
                "Deserialization failed: {}, bytes length: {}, full bytes dump: {:?}",
                e,
                bytes.len(),
                bytes
            );
            RaisinError::storage(format!("Deserialization error for NodeType: {}", e))
        })
    }

    pub(super) fn extract_name(key: &[u8]) -> Result<String> {
        let parts: Vec<&[u8]> = key.split(|&b| b == 0).collect();
        let name_bytes = parts
            .get(4)
            .ok_or_else(|| RaisinError::storage("Invalid nodetype key (missing nodetype name)"))?;
        String::from_utf8(name_bytes.to_vec())
            .map_err(|e| RaisinError::storage(format!("Invalid UTF-8 in nodetype name: {}", e)))
    }

    pub(super) async fn resolve_head_revision(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
    ) -> Result<Option<HLC>> {
        match self.branch_repo.get_head(tenant_id, repo_id, branch).await {
            Ok(head) => Ok(Some(head)),
            Err(RaisinError::NotFound(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub(super) async fn get_at_or_before(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        name: &str,
        target_revision: &HLC,
    ) -> Result<Option<NodeType>> {
        tracing::debug!(
            target: "rocksb::nodetype::lookup",
            "Scanning NodeType '{}' in {}/{}/{} at or before revision {}",
            name,
            tenant_id,
            repo_id,
            branch,
            target_revision
        );
        let cf = cf_handle(&self.db, cf::NODE_TYPES)?;
        let prefix = keys::nodetype_name_prefix(tenant_id, repo_id, branch, name);
        let prefix_clone = prefix.clone();
        let iter = self.db.prefix_iterator_cf(cf, prefix);

        for item in iter {
            let (key, value) = item.map_err(|e| RaisinError::storage(e.to_string()))?;
            if !key.starts_with(&prefix_clone) {
                tracing::debug!(
                    target: "rocksb::nodetype::lookup",
                    "Key no longer matches prefix for '{}', stopping scan",
                    name
                );
                break;
            }

            let revision = Self::decode_revision(&key)?;
            if &revision > target_revision {
                tracing::trace!(
                    target: "rocksb::nodetype::lookup",
                    "Skipping revision {} for '{}' (newer than target {})",
                    revision,
                    name,
                    target_revision
                );
                continue;
            }

            if value.as_ref() == TOMBSTONE {
                tracing::debug!(
                    target: "rocksb::nodetype::lookup",
                    "Found tombstone for '{}' at revision {}, returning None",
                    name,
                    revision
                );
                return Ok(None);
            }

            let nodetype = Self::deserialize_node_type(&value)?;
            tracing::debug!(
                target: "rocksb::nodetype::lookup",
                "Found NodeType '{}' at revision {}",
                nodetype.name,
                revision
            );
            return Ok(Some(nodetype));
        }

        tracing::debug!(
            target: "rocksb::nodetype::lookup",
            "No NodeType '{}' entries found at or before revision {}",
            name,
            target_revision
        );
        Ok(None)
    }

    pub(super) fn determine_parent(parent_head: Option<HLC>) -> Option<HLC> {
        parent_head
    }

    /// Warn about compound-index declarations that will never do what the
    /// author meant.
    ///
    /// These are WARNINGS, not errors, on purpose: this runs on the package
    /// install path, and a hard failure there aborts the whole schema batch and
    /// takes an unrelated install down with it. The cost of a bad declaration is
    /// a permanently unused (or permanently empty) index with correct query
    /// results — bad performance, never wrong rows — so refusing the write is
    /// the more damaging of the two options.
    ///
    /// The trap this exists for: the PLANNER rewrites a query's
    /// `ORDER BY created_at` to `__created_at`
    /// (`raisin-sql-execution/.../planner/compound_index.rs`) and then compares
    /// it against the declared property VERBATIM. Meanwhile the WRITER resolves
    /// `__created_at` from `node.created_at` but a bare `created_at` from
    /// `node.properties["created_at"]` — a different value entirely. So a
    /// declaration spelling the order column `created_at` can never claim the
    /// ORDER BY it was written for, and silently costs a Sort. Nothing else in
    /// the system says so.
    pub(super) fn warn_on_suspect_compound_indexes(node_type: &NodeType) {
        const ENGINE_OWNED: [&str; 4] = [
            "__node_type",
            "__parent_path",
            "__created_at",
            "__updated_at",
        ];

        let Some(indexes) = node_type.compound_indexes.as_ref() else {
            return;
        };

        let declared: std::collections::HashSet<&str> = node_type
            .properties
            .as_ref()
            .map(|props| props.iter().filter_map(|p| p.name.as_deref()).collect())
            .unwrap_or_default();

        for index in indexes {
            if index.has_order_column {
                if let Some(order_col) = index.columns.last() {
                    let bare = order_col.property.as_str();
                    if bare == "created_at" || bare == "updated_at" {
                        tracing::warn!(
                            target: "rocksb::nodetype::compound_index",
                            "NodeType '{}' compound index '{}' declares its ORDER column as '{}'. \
                             The planner normalises `ORDER BY {}` to '__{}', so this index can NEVER \
                             claim that ordering and every query pays a Sort. It also indexes \
                             properties['{}'] rather than the engine timestamp. Did you mean '__{}'?",
                            node_type.name, index.name, bare, bare, bare, bare, bare
                        );
                    }
                }
            }

            for column in &index.columns {
                let prop = column.property.as_str();
                if ENGINE_OWNED.contains(&prop) || declared.contains(prop) {
                    continue;
                }
                tracing::warn!(
                    target: "rocksb::nodetype::compound_index",
                    "NodeType '{}' compound index '{}' references column '{}', which is neither a \
                     declared property nor engine-owned. Every node missing that value is dropped \
                     from the index entirely, so the index may be silently empty.",
                    node_type.name, index.name, prop
                );
            }
        }
    }

    /// Drop the build state of any compound index whose declaration changed.
    ///
    /// The entries already in the keyspace were written under the OLD columns,
    /// and an index name addresses one workspace-global keyspace — so after a
    /// declaration change the stored bytes and the planner's idea of the key
    /// layout disagree. Marking the state stale makes the planner decline the
    /// index until a build re-establishes it, instead of reading the old
    /// entries through the new layout.
    ///
    /// Deliberately covers REMOVED declarations too: an index dropped from a
    /// NodeType stops being maintained immediately, so a state record still
    /// saying `Ready` would outlive the thing it describes.
    ///
    /// Best-effort — a failure here is logged, never fatal. Refusing a schema
    /// write because a status record could not be updated would take down
    /// package installs for a bookkeeping problem, and the fail-closed default
    /// already covers the case where the record is missing entirely.
    pub(super) fn invalidate_changed_compound_state(
        db: &Arc<DB>,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        existing: Option<&NodeType>,
        incoming: &NodeType,
    ) {
        use std::collections::HashMap;

        let hashes = |nt: Option<&NodeType>| -> HashMap<String, u64> {
            nt.and_then(|n| n.compound_indexes.as_ref())
                .map(|indexes| {
                    indexes
                        .iter()
                        .map(|i| (i.name.clone(), i.definition_hash()))
                        .collect()
                })
                .unwrap_or_default()
        };

        let before = hashes(existing);
        let after = hashes(Some(incoming));

        let mut stale: Vec<&String> = Vec::new();
        for (name, hash) in &after {
            if before.get(name) != Some(hash) {
                stale.push(name);
            }
        }
        for name in before.keys() {
            if !after.contains_key(name) {
                stale.push(name);
            }
        }
        if stale.is_empty() {
            return;
        }

        let store = crate::compound_state::CompoundStateStore::new(db.clone());
        // A NodeType is branch-scoped but compound state is per WORKSPACE, and
        // the declaration does not name one. Clearing across every workspace
        // that has a record is the conservative reading: a spurious extra
        // rebuild costs time, a missed one costs correctness.
        let workspaces = match Self::workspaces_with_compound_state(db, tenant_id, repo_id, branch)
        {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!(error = %e, "could not enumerate compound index state; skipping invalidation");
                return;
            }
        };
        for workspace in workspaces {
            for name in &stale {
                if let Err(e) = store.mark_not_built(tenant_id, repo_id, branch, &workspace, name) {
                    tracing::warn!(
                        index = %name,
                        workspace = %workspace,
                        error = %e,
                        "could not invalidate compound index build state"
                    );
                }
            }
        }
    }

    /// Workspaces on this branch that carry at least one compound state record.
    fn workspaces_with_compound_state(
        db: &Arc<DB>,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
    ) -> Result<Vec<String>> {
        let cf = cf_handle(db, cf::INDEX_STATUS)?;
        let prefix = format!("compound_index\0{tenant_id}\0{repo_id}\0{branch}\0").into_bytes();
        let mut out = std::collections::BTreeSet::new();
        for item in db.prefix_iterator_cf(cf, &prefix) {
            let (key, _) = item
                .map_err(|e| RaisinError::storage(format!("compound state scan failed: {}", e)))?;
            if !key.starts_with(&prefix) {
                break;
            }
            // Remainder is `{workspace}\0{index_name}`.
            let rest = &key[prefix.len()..];
            if let Some(sep) = rest.iter().position(|b| *b == 0) {
                if let Ok(ws) = std::str::from_utf8(&rest[..sep]) {
                    out.insert(ws.to_string());
                }
            }
        }
        Ok(out.into_iter().collect())
    }

    pub(super) fn apply_versioning(node_type: NodeType, existing: Option<&NodeType>) -> NodeType {
        let now = Utc::now();
        let mut enriched = node_type.clone();

        if enriched.id.is_none() {
            enriched.id = Some(nanoid::nanoid!(16));
        }

        if let Some(previous) = existing {
            let next_version = previous.version.unwrap_or(0) + 1;
            enriched.version = Some(next_version);
            enriched.created_at = previous.created_at;
            enriched.previous_version = previous.id.clone();
        } else {
            enriched.version = Some(1);
            if enriched.created_at.is_none() {
                enriched.created_at = Some(now);
            }
        }

        enriched.updated_at = Some(now);
        enriched
    }

    pub(super) fn build_revision_meta(
        revision: HLC,
        parent: Option<HLC>,
        branch: &str,
        commit: &CommitMetadata,
        operation: ChangeOperation,
        node_type_name: &str,
    ) -> RevisionMeta {
        RevisionMeta {
            revision,
            parent,
            merge_parent: None,
            branch: branch.to_string(),
            timestamp: Utc::now(),
            actor: commit.actor.clone(),
            message: commit.message.clone(),
            is_system: commit.is_system,
            changed_nodes: Vec::new(),
            changed_node_types: vec![NodeTypeChangeInfo {
                name: node_type_name.to_string(),
                operation,
            }],
            changed_archetypes: Vec::new(),
            changed_element_types: Vec::new(),
            operation: None,
        }
    }
}

/// Inheritance-aware resolution needs only "fetch a NodeType by name", and this
/// is the object-safe form of that. It exists so a caller BELOW the storage
/// facade — the node repository, which can never hold an `Arc<RocksDBStorage>`
/// without making a reference cycle — can drive the SAME `NodeTypeResolver`
/// rather than growing a second inheritance walk. See
/// `raisin_core::services::schema_lookup`.
#[async_trait::async_trait]
impl raisin_core::services::schema_lookup::NodeTypeLookup for NodeTypeRepositoryImpl {
    async fn get_node_type(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        name: &str,
        max_revision: Option<raisin_hlc::HLC>,
    ) -> raisin_error::Result<Option<raisin_models::nodes::types::NodeType>> {
        use raisin_storage::NodeTypeRepository;
        self.get(
            raisin_storage::scope::BranchScope::new(tenant_id, repo_id, branch),
            name,
            max_revision.as_ref(),
        )
        .await
    }
}
