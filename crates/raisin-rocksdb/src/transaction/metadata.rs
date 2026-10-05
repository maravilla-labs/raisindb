//! Internal metadata structures for transaction state management

use raisin_hlc::HLC;
use raisin_models::auth::AuthContext;
use raisin_models::{nodes::Node, translations::LocaleOverlay};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;

/// Transaction metadata
///
/// Uses `Arc<String>` for frequently cloned fields to optimize hot path performance.
/// When metadata is extracted during commit, Arc clones are very cheap (just increment refcount)
/// compared to full String allocations.
#[derive(Debug, Clone, Default)]
pub(crate) struct TransactionMetadata {
    pub(crate) tenant_id: Arc<String>,
    pub(crate) repo_id: Arc<String>,
    pub(crate) branch: Option<Arc<String>>,
    pub(crate) actor: Option<Arc<String>>,
    pub(crate) message: Option<Arc<String>>,
    pub(crate) is_manual_version: bool,
    pub(crate) manual_version_node_id: Option<Arc<String>>,
    /// Whether this is a system commit (background job, migration, etc.)
    pub(crate) is_system: bool,
    /// Whether this commit is engine bookkeeping (state blobs, counters).
    /// Skips TreeSnapshot + trigger fan-out; see `TransactionalContext::set_bookkeeping`.
    pub(crate) bookkeeping: bool,
    /// A `put_node` in this transaction rewrote a node IN PLACE (a
    /// `versionable=false` update at the node's existing revision): the commit
    /// holds `in_place_write_guard` around its write (see
    /// `repositories/nodes/crud/indexing/in_place_guard.rs`).
    pub(crate) in_place_record_write: bool,
    /// The single HLC timestamp used for ALL operations in this transaction
    /// This ensures atomicity - all nodes in a transaction share the same revision
    pub(crate) transaction_revision: Option<HLC>,
    /// Authentication context for permission checks
    /// When set, RLS and field-level security will be enforced
    pub(crate) auth_context: Option<Arc<AuthContext>>,
}

/// Read-your-writes cache for transactions
#[derive(Debug, Default)]
pub(crate) struct ReadCache {
    /// Cached nodes: (workspace, node_id) -> Node
    pub(crate) nodes: HashMap<(String, String), Option<Node>>,
    /// Cached paths: (workspace, path) -> node_id
    pub(crate) paths: HashMap<(String, String), Option<String>>,
    /// Committed nodes a `move_node_tree` in this transaction moved, as they
    /// stand after it: (workspace, node_id) -> Node with its new path.
    ///
    /// Not in `nodes`, because `nodes` holds what this transaction WROTE and
    /// is served without an RLS check; these were read from committed state
    /// and must still pass one. `get_node` serves them in place of the
    /// committed (pre-move) record — without this, a write after the move
    /// read the old path and stored it back at the move's revision.
    pub(crate) moved_nodes: HashMap<(String, String), Node>,
    /// Cached translations: (workspace, node_id, locale) -> LocaleOverlay.
    /// Ordered, so one node's overlays are a range ([`Self::translations_of`])
    /// rather than a scan of every overlay the transaction staged — which
    /// made a large transaction's uniqueness checks quadratic.
    pub(crate) translations: BTreeMap<(String, String, String), Option<LocaleOverlay>>,
    /// Cached BLOCK translations: (workspace, node_id, block_uuid, locale) -> LocaleOverlay.
    /// Separate from `translations` because block overlays have their own key
    /// space — same node, different block, different record.
    pub(crate) block_translations: HashMap<(String, String, String, String), Option<LocaleOverlay>>,
    /// Last assigned order label per (workspace, parent_id) within this transaction.
    /// Prevents sibling nodes in the same batch from getting identical fractional indexes.
    pub(crate) last_order_labels: HashMap<(String, String), String>,
    /// (workspace, node_id) -> what the node's FIRST property-index write in
    /// this transaction assumed about its stored versions, re-checked under
    /// the commit lock (`indexing::StagedDeltaCheck`). Recorded only while
    /// `index.skip_unchanged` is on.
    pub(crate) delta_checks: HashMap<(String, String), crate::indexing::StagedDeltaCheck>,
    /// (workspace, locale, translated node name) -> the nodes whose overlays
    /// staged in this transaction set that name: HINTS for sibling
    /// uniqueness (plan Phase 13c), written only by [`Self::put_translation`].
    /// Never pruned when a later overlay replaces the name — every hint is
    /// judged against the transaction's final view.
    pub(crate) node_name_hints: HashMap<(String, String, String), BTreeSet<String>>,
}

impl ReadCache {
    /// The overlays this transaction staged for `node_id` in `workspace`, as
    /// `(locale, overlay)`.
    pub(crate) fn translations_of<'a>(
        &'a self,
        workspace: &str,
        node_id: &str,
    ) -> impl Iterator<Item = (&'a String, &'a Option<LocaleOverlay>)> + 'a {
        let (ws, id) = (workspace.to_string(), node_id.to_string());
        self.translations
            .range((ws.clone(), id.clone(), String::new())..)
            .take_while(move |((w, n, _), _)| *w == ws && *n == id)
            .map(|((_, _, locale), overlay)| (locale, overlay))
    }

    /// Record an overlay this transaction staged (`None`: deleted), with its
    /// `node_name_hints`. (A node delete only ever clears an entry in place,
    /// which adds no name, so it needs no hint.)
    pub(crate) fn put_translation(
        &mut self,
        workspace: &str,
        node_id: &str,
        locale: &str,
        overlay: Option<LocaleOverlay>,
    ) {
        use crate::indexing::localized_node_names::{overlay_node_name, NameIn};
        if let Some(NameIn::Name(name)) = overlay.as_ref().map(overlay_node_name) {
            self.node_name_hints
                .entry((workspace.to_string(), locale.to_string(), name))
                .or_default()
                .insert(node_id.to_string());
        }
        self.translations.insert(
            (
                workspace.to_string(),
                node_id.to_string(),
                locale.to_string(),
            ),
            overlay,
        );
    }
}

/// Conflict detection tracking
#[derive(Debug, Default)]
pub(crate) struct ConflictTracker {
    /// Set of keys read during this transaction
    pub(crate) read_set: HashSet<Vec<u8>>,
    /// Set of keys written during this transaction
    pub(crate) write_set: HashSet<Vec<u8>>,
}
