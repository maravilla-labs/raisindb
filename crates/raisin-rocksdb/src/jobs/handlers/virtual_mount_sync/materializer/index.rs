//! The mount's slice of the target workspace, read ONCE per sync run.
//!
//! See the module doc on [`super`] for why this exists: every lookup the upsert
//! path needs is served from these two maps instead of re-listing the workspace
//! per item.

use raisin_models::nodes::Node;
use std::collections::HashMap;

use super::node_paths::{
    ancestor_paths, node_external_id, node_mount_id, node_str_prop, node_synced_secs, under,
};
use super::write_view::{carried_pushed_state, write_view_of, WriteView};

/// Identifies one mount within a repo/branch/workspace.
#[derive(Debug, Clone)]
pub struct MountScope {
    pub tenant: String,
    pub repo: String,
    pub branch: String,
    /// Target workspace the mount materializes into.
    pub workspace: String,
    pub mount_id: String,
    /// Path prefix inside the target workspace (e.g. `/documents/shared`).
    pub mount_path: String,
    /// Re-materialize every item even when its etag is unchanged, and move it if
    /// the mapper/template now resolves a different path ("remap").
    ///
    /// Ordinary syncs must NOT do this: the etag skip-write is what stops a
    /// re-sync creating a revision per unchanged item and re-firing every
    /// downstream trigger. But that same skip returns before the mapper's output
    /// is applied, so a changed mapper — a new node type, a renamed property, a
    /// new folder hierarchy — is invisible to everything already synced. Remap
    /// is the deliberate, operator-triggered exception.
    pub force_rewrite: bool,
    /// Node properties this mount may push outward — the `state_only`
    /// allow-list, taken from `write_config.mutable_fields`.
    ///
    /// Empty for every mount that has not configured writeback, which is all of
    /// them by default; the index then carries no [`WriteView`] at all, so a
    /// read-only mount pays nothing for the write path existing.
    ///
    /// Deliberately the MOUNT's declared list rather than the effective one:
    /// the index is loaded before the adapter's `capabilities` are probed, and
    /// what gets STAMPED must not depend on what the adapter happens to answer
    /// this run — a mount whose adapter probe fails once would otherwise stop
    /// seeding `__pushed_state` and push a batch of stale flags when it
    /// recovered. The adapter's `mutable_fields` narrows what is actually SENT
    /// (see `write::plan`), not what is recorded.
    pub watched_fields: Vec<String>,
}

/// Reserved virtual metadata stamped on every synced node.
#[derive(Debug, Clone)]
pub struct VirtualMeta {
    pub mount_id: String,
    pub external_id: String,
    pub etag: Option<String>,
    /// ISO 8601 timestamp of this sync write.
    pub synced_at: String,
}

/// Lightweight reference to an existing mount-owned node.
#[derive(Debug, Clone)]
pub struct VirtualNodeRef {
    pub id: String,
    pub path: String,
    pub external_id: String,
    pub etag: Option<String>,
    /// `__synced_at` as unix epoch seconds (used by ephemeral TTL cleanup).
    /// Tolerates both `String` (ISO 8601) and `Date` stored representations.
    pub synced_secs: Option<i64>,
    /// Present only when the mount watches fields (see
    /// [`MountScope::watched_fields`]), and boxed so a read-only mount's index
    /// entry grows by one pointer rather than by two maps per node.
    pub write_view: Option<Box<WriteView>>,
    /// The node's stored `__pushed_state`, carried ONLY when [`Self::write_view`]
    /// is absent — see [`carried_pushed_state`]. Read through
    /// [`Self::pushed_state`], never directly.
    pub pushed_state: Option<serde_json::Map<String, serde_json::Value>>,
}

impl VirtualNodeRef {
    /// The writeback baseline stored on this node, whichever half of the entry
    /// is carrying it.
    ///
    /// The upsert path re-states a node's whole property map, so it must put
    /// this back when it has nothing newer to say — otherwise flipping a mount's
    /// `mode` away from `state_only` (to `off`, to debug) silently strips the
    /// baseline from every node the next delta touches, and re-enabling
    /// writeback then finds a mailbox of nodes with no record of what was pushed.
    pub fn pushed_state(&self) -> Option<&serde_json::Map<String, serde_json::Value>> {
        match self.write_view.as_ref().and_then(|v| v.pushed.as_ref()) {
            Some(pushed) => Some(pushed),
            None => self.pushed_state.as_ref(),
        }
    }
}

/// One node occupying a path under the mount.
#[derive(Debug, Clone)]
pub struct PathEntry {
    /// `None` when we know the path is occupied but never read the node's id —
    /// an ancestor folder auto-created by `upsert_deep_node` during this run.
    pub id: Option<String>,
    /// Whether the node carries THIS mount's `__mount_id`. Nodes that do not are
    /// user content and must never be clobbered.
    pub mount_owned: bool,
    pub etag: Option<String>,
}

/// Everything the upsert path needs to locate an item, read ONCE per sync run.
///
/// Two views, because the single filtered list the engine used to derive is not
/// enough:
///
/// * `by_external` — mount-owned nodes that carry an `__external_id`. Replaces
///   the linear scan that matched an item to its node, and is what makes a
///   provider-side rename update the existing node instead of duplicating it.
/// * `by_path` — EVERY node under the mount path, mount-owned or not. The
///   foreign-node guard (never overwrite user content sitting at a target path)
///   and the path-fallback etag check both operate on nodes that `by_external`
///   deliberately excludes, so filtering this map by `__mount_id` would silently
///   drop both protections.
#[derive(Debug, Clone, Default)]
pub struct SyncIndex {
    by_external: HashMap<String, VirtualNodeRef>,
    by_path: HashMap<String, PathEntry>,
}

impl SyncIndex {
    /// Build both views from a listing of the target workspace.
    pub fn from_nodes(
        nodes: Vec<Node>,
        mount_id: &str,
        mount_path: &str,
        watched_fields: &[String],
    ) -> Self {
        let mut idx = Self::default();
        for node in nodes {
            if !under(mount_path, &node.path) {
                continue;
            }
            let mount_owned = node_mount_id(&node) == Some(mount_id);
            let etag = node_str_prop(&node, "__etag");
            idx.by_path.insert(
                node.path.clone(),
                PathEntry {
                    id: Some(node.id.clone()),
                    mount_owned,
                    etag: etag.clone(),
                },
            );
            if !mount_owned {
                continue;
            }
            if let Some(ext) = node_external_id(&node) {
                let write_view = write_view_of(&node, watched_fields);
                idx.by_external.insert(
                    ext.to_string(),
                    VirtualNodeRef {
                        id: node.id.clone(),
                        path: node.path.clone(),
                        external_id: ext.to_string(),
                        etag,
                        synced_secs: node_synced_secs(&node),
                        pushed_state: carried_pushed_state(&node, &write_view),
                        write_view,
                    },
                );
            }
        }
        idx
    }

    /// Every mount-owned virtual node, for reconcile and TTL cleanup.
    pub fn virtual_nodes(&self) -> Vec<VirtualNodeRef> {
        self.by_external.values().cloned().collect()
    }

    /// Number of mount-owned virtual nodes.
    pub fn virtual_len(&self) -> usize {
        self.by_external.len()
    }

    pub(super) fn by_external(&self, external_id: &str) -> Option<&VirtualNodeRef> {
        self.by_external.get(external_id)
    }

    /// The stored etag of an already-synced item, for the pre-mapping skip check.
    pub fn etag_for(&self, external_id: &str) -> Option<&str> {
        self.by_external.get(external_id)?.etag.as_deref()
    }

    /// Namespaced external ids of the subordinate nodes materialized under
    /// `parent_external_id` (mail attachments).
    ///
    /// Keyed off the external-id namespace rather than the node PATH on
    /// purpose. A path lookup would answer "whatever currently sits beneath this
    /// node", which after a `path_template` change or an operator's manual move
    /// can include foreign content — and both callers act destructively on the
    /// answer: one reports it as seen (suppressing a reconcile delete), the
    /// other deletes it outright. The namespace says "this mount created this
    /// node AS a child of that item", which is the actual question.
    pub fn child_external_ids(&self, parent_external_id: &str) -> Vec<String> {
        let prefix = format!("{parent_external_id}{}", super::super::config::CHILD_ID_SEP);
        self.by_external
            .keys()
            .filter(|k| k.starts_with(&prefix))
            .cloned()
            .collect()
    }

    // Visible to the whole sync engine, not just the materializer: the
    // never-clobber-user-content guard is asserted from the engine's tests.
    pub(in crate::jobs::handlers::virtual_mount_sync) fn at_path(
        &self,
        path: &str,
    ) -> Option<&PathEntry> {
        self.by_path.get(path)
    }

    /// Record a node this run just wrote, plus the ancestor folders
    /// `upsert_deep_node` guarantees now exist.
    ///
    /// Both must be recorded or a later item in the SAME run re-derives a stale
    /// answer: without the node, a duplicate `external_id` creates a second node;
    /// without the ancestors, an item resolving exactly onto an auto-created
    /// folder path would treat that path as free.
    pub(super) fn record_upsert(&mut self, node_ref: VirtualNodeRef) {
        for ancestor in ancestor_paths(&node_ref.path) {
            self.by_path.entry(ancestor).or_insert(PathEntry {
                id: None,
                mount_owned: false,
                etag: None,
            });
        }
        self.by_path.insert(
            node_ref.path.clone(),
            PathEntry {
                id: Some(node_ref.id.clone()),
                mount_owned: true,
                etag: node_ref.etag.clone(),
            },
        );
        self.by_external
            .insert(node_ref.external_id.clone(), node_ref);
    }

    /// Record a relocation performed by the remap pre-pass.
    pub(super) fn record_move(&mut self, external_id: &str, new_path: &str) {
        if let Some(existing) = self.by_external.get_mut(external_id) {
            let old_path = std::mem::replace(&mut existing.path, new_path.to_string());
            let entry = self.by_path.remove(&old_path);
            self.by_path.insert(
                new_path.to_string(),
                entry.unwrap_or(PathEntry {
                    id: Some(existing.id.clone()),
                    mount_owned: true,
                    etag: existing.etag.clone(),
                }),
            );
        }
    }

    /// Drop a node this run just deleted.
    pub(super) fn record_delete(&mut self, external_id: &str) {
        if let Some(existing) = self.by_external.remove(external_id) {
            self.by_path.remove(&existing.path);
        }
    }
}
