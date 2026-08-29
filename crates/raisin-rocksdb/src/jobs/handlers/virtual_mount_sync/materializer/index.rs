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
    ///
    /// A remap on a mount whose [`Self::read_local_wins`] is set still keeps
    /// pending local edits: its contract is "re-apply the mapper to unchanged
    /// remote data", not "resolve conflicts in remote's favour", so the
    /// preserve step in `stage_op` deliberately does not branch on this flag.
    pub force_rewrite: bool,
    /// Whether an incoming remote item may overwrite a locally-diverged watched
    /// field. `false` for every policy except an explicit, well-formed
    /// `local_wins` — `remote_wins`, `error`, `resolver_function`, unset, and
    /// unparseable values all read as `false`, because the read path must never
    /// invent a merge rule the write path would refuse.
    ///
    /// A bool rather than the `ConflictPolicy` enum on purpose: the
    /// materializer has no business knowing about resolver plumbing, and the
    /// enum stays `pub(crate)` inside `write::conflict` where refusals are
    /// handled loudly.
    pub read_local_wins: bool,
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
    /// Node types this mount treats as OUTBOX COMMANDS
    /// (`write_config.command_node_types`).
    ///
    /// The read path needs it because a command node is both a command and a
    /// synced item, and an upsert rebuilds the property map from mapper output.
    /// The submit lifecycle — `status`, `attempt_id`, `sent_at` — is written by
    /// the engine and appears nowhere in that output, so the first sync after a
    /// send erased it. See `COMMAND_KEYS` in `stage.rs`.
    ///
    /// Empty for every mount that is not an outbox, which is all of them by
    /// default, and the carry is skipped entirely in that case.
    pub command_node_types: Vec<String>,
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
    /// Whether this node is an OUTBOX COMMAND rather than a mirror of a remote
    /// item — its `node_type` is one the mount declares in `command_node_types`.
    ///
    /// Read by reconcile: a command is AUTHORED LOCALLY, so "not seen upstream"
    /// is its normal condition and must never mean "deleted upstream". Some
    /// providers answer a send with no id at all (Graph's `sendMail` is a 202
    /// with an empty body), leaving the node stamped `cmd:{node_id}` — an id no
    /// listing can ever return, so an unguarded reconcile would delete every
    /// command it had just sent.
    pub is_command: bool,
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
    /// The node's `__external_id`, carried here so [`SyncIndex::external_id_at`]
    /// is a lookup rather than a scan of `by_external`.
    ///
    /// Lives INSIDE this entry rather than in a second path→external-id map on
    /// purpose: a parallel map would be a fourth structure to keep in step
    /// across `from_nodes`/`adopt`/`record_upsert`/`record_move`/`record_delete`,
    /// and `record_move`'s path swap — which moves this entry wholesale — is
    /// exactly where it would rot.
    ///
    /// `None` for every node this mount does not own and for the ancestor
    /// folders recorded with no id, which is what keeps `external_id_at` from
    /// answering for foreign content.
    pub external_id: Option<String>,
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
/// * `children_by_parent` — the same external-id namespace as `by_external`,
///   pre-split on [`CHILD_ID_SEP`], so [`Self::child_external_ids`] answers
///   without scanning every key (see that method).
#[derive(Debug, Clone, Default)]
pub struct SyncIndex {
    by_external: HashMap<String, VirtualNodeRef>,
    by_path: HashMap<String, PathEntry>,
    /// Parent external id → the namespaced ids of its subordinate nodes.
    /// Derived wholly from `by_external`'s keys; maintained in the same four
    /// places `by_external` is.
    children_by_parent: HashMap<String, Vec<String>>,
}

impl SyncIndex {
    /// Build both views from a listing of the target workspace.
    pub fn from_nodes(
        nodes: Vec<Node>,
        mount_id: &str,
        mount_path: &str,
        watched_fields: &[String],
        command_node_types: &[String],
    ) -> Self {
        let mut idx = Self::default();
        for node in nodes {
            if !under(mount_path, &node.path) {
                continue;
            }
            let mount_owned = node_mount_id(&node) == Some(mount_id);
            let etag = node_str_prop(&node, "__etag");
            // Only a node this mount owns may answer `external_id_at`; a foreign
            // node's id is deliberately not recorded here.
            let external_id = if mount_owned {
                node_external_id(&node).map(str::to_string)
            } else {
                None
            };
            idx.by_path.insert(
                node.path.clone(),
                PathEntry {
                    id: Some(node.id.clone()),
                    mount_owned,
                    etag: etag.clone(),
                    external_id: external_id.clone(),
                },
            );
            if !mount_owned {
                continue;
            }
            if let Some(ext) = external_id {
                let write_view = write_view_of(&node, watched_fields);
                let is_command = command_node_types.iter().any(|t| t == &node.node_type);
                idx.index_child(&ext);
                idx.by_external.insert(
                    ext.clone(),
                    VirtualNodeRef {
                        id: node.id.clone(),
                        path: node.path.clone(),
                        external_id: ext,
                        is_command,
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

    /// Record a node this RUN has just adopted an external id for.
    ///
    /// The index is read once at the start of a run, but the outbox drain runs
    /// BEFORE the walk in that same run (`phases.rs`) and stamps
    /// `__external_id` on the command node it just sent — straight to the
    /// materializer, bypassing this index. The walk then looked the item up,
    /// missed, fell back to a path match, found nothing at the provider-derived
    /// path and CREATED A SECOND NODE: one at `/checkout/order-123` and another
    /// at `/checkout/cs_test_…`, both claiming the same `__external_id`, with
    /// the original silently renamed on top.
    ///
    /// That mattered more than a stray node. Path is the natural way to address
    /// anything in RaisinDB, and the duplicate made a command node's path
    /// unreliable the moment it succeeded — for the one node type an
    /// application authors itself. The `by_external` hit already keeps the
    /// existing path (see the upsert's first branch), so paths were always meant
    /// to be stable here; this closes the one window where the index could not
    /// answer.
    pub fn adopt(
        &mut self,
        node_id: &str,
        path: &str,
        external_id: &str,
        etag: Option<String>,
        is_command: bool,
    ) {
        self.by_path.insert(
            path.to_string(),
            PathEntry {
                id: Some(node_id.to_string()),
                mount_owned: true,
                etag: etag.clone(),
                external_id: Some(external_id.to_string()),
            },
        );
        self.index_child(external_id);
        self.by_external.insert(
            external_id.to_string(),
            VirtualNodeRef {
                id: node_id.to_string(),
                path: path.to_string(),
                external_id: external_id.to_string(),
                is_command,
                etag,
                // Unset on purpose: the TTL cleanup reads `__synced_at` from the
                // stored node, and this run has not written one yet.
                synced_secs: None,
                write_view: None,
                pushed_state: None,
            },
        );
    }

    /// Every mount-owned virtual node, for reconcile and TTL cleanup.
    ///
    /// Clones the whole map; take [`Self::virtual_nodes_iter`] unless the caller
    /// needs owned values (`write::candidates` consumes them).
    pub fn virtual_nodes(&self) -> Vec<VirtualNodeRef> {
        self.by_external.values().cloned().collect()
    }

    /// The same nodes, borrowed.
    ///
    /// [`Self::virtual_nodes`] deep-clones every entry — five `String`s per
    /// node, plus a boxed write view holding a `serde_json` map per watched
    /// field on a writeback mount — and reconcile and the TTL sweep read two
    /// fields per node and normally delete none of them. Borrowing takes that
    /// from n clones to the k ids actually acted on.
    ///
    /// Both of those callers stage their deletes through the batcher, which
    /// borrows it mutably, so they must collect the ids they want BEFORE the
    /// loop that deletes them; the owning version above exists for exactly that
    /// reason and is still what `write::mod` needs.
    pub fn virtual_nodes_iter(&self) -> impl Iterator<Item = &VirtualNodeRef> {
        self.by_external.values()
    }

    /// The provider id of the mount-owned node at `path`, if there is one.
    ///
    /// This is how a locally-created node finds the FOLDER it belongs in. The
    /// engine otherwise tells an adapter only the mount's own remote root, which
    /// is right for a calendar (one container) and wrong for a drive, where a
    /// file uploaded into `Gründung` must be created inside that folder rather
    /// than at the top of the library.
    /// One hash lookup, not a scan. This runs once per create candidate inside
    /// the drain's provider-call loop, so the old `by_external.values().find()`
    /// cost the whole node map per candidate: n candidates x m nodes -> m.
    pub fn external_id_at(&self, path: &str) -> Option<&str> {
        let entry = self.by_path.get(path)?;
        // A path occupied by user content, or by an ancestor folder this run
        // auto-created, has no id to give — and answering for one would file a
        // new remote object inside a stranger's folder.
        if !entry.mount_owned {
            return None;
        }
        entry.external_id.as_deref()
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
    /// Served from a map keyed by the parent id rather than by scanning every
    /// key for a prefix: n keys compared per call -> 1 lookup. That matters
    /// because `item.rs` calls this for EVERY etag-skipped item, i.e. for every
    /// item of an unchanged re-walk — the path its own comment calls "the
    /// entire cost of the run" — so the scan made that path quadratic in the
    /// size of the mount.
    ///
    /// The returned order is insertion order rather than the old (arbitrary)
    /// hash order. Neither caller depends on it: one reports the ids as seen,
    /// the other deletes each one.
    pub fn child_external_ids(&self, parent_external_id: &str) -> Vec<String> {
        self.children_by_parent
            .get(parent_external_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Record `external_id` under its parent, if it is a namespaced child id.
    ///
    /// Idempotent: `record_upsert` re-inserts ids already in `by_external` on a
    /// re-sync, and a duplicate here would have the delete loop visit the same
    /// child twice.
    fn index_child(&mut self, external_id: &str) {
        let Some((parent, _)) = external_id.split_once(super::super::config::CHILD_ID_SEP) else {
            return;
        };
        let siblings = self
            .children_by_parent
            .entry(parent.to_string())
            .or_default();
        if !siblings.iter().any(|c| c == external_id) {
            siblings.push(external_id.to_string());
        }
    }

    /// Drop `external_id` from its parent's child list, and the parent's entry
    /// with it once empty — an entry that outlived its children would report
    /// deleted nodes as live.
    fn unindex_child(&mut self, external_id: &str) {
        let Some((parent, _)) = external_id.split_once(super::super::config::CHILD_ID_SEP) else {
            return;
        };
        let Some(siblings) = self.children_by_parent.get_mut(parent) else {
            return;
        };
        siblings.retain(|c| c != external_id);
        if siblings.is_empty() {
            self.children_by_parent.remove(parent);
        }
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
    pub(crate) fn record_upsert(&mut self, node_ref: VirtualNodeRef) {
        for ancestor in ancestor_paths(&node_ref.path) {
            self.by_path.entry(ancestor).or_insert(PathEntry {
                id: None,
                mount_owned: false,
                etag: None,
                external_id: None,
            });
        }
        self.by_path.insert(
            node_ref.path.clone(),
            PathEntry {
                id: Some(node_ref.id.clone()),
                mount_owned: true,
                etag: node_ref.etag.clone(),
                external_id: Some(node_ref.external_id.clone()),
            },
        );
        self.index_child(&node_ref.external_id);
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
                    external_id: Some(external_id.to_string()),
                }),
            );
        }
    }

    /// Drop a node this run just deleted.
    pub(super) fn record_delete(&mut self, external_id: &str) {
        if let Some(existing) = self.by_external.remove(external_id) {
            self.by_path.remove(&existing.path);
            self.unindex_child(external_id);
        }
    }
}
