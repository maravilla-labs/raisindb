//! Localized URL lookup (plan Phase 12): the storage-level contract behind
//! `NodeService::resolve_localized_path` and every surface that calls it.

use raisin_error::Result;
use raisin_hlc::HLC;
use std::sync::Arc;

use crate::scope::StorageScope;

/// How a lookup was answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalizedServedBy {
    /// The locale is the default language: the path is canonical.
    DefaultLanguage,
    /// Through the localized name index.
    Index,
    /// Through the row-level fallback (the index is not ready for this read).
    Fallback,
}

/// A resolved localized path. RLS is NOT applied here: the caller reads the
/// node through its own permission-checked path.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LocalizedResolution {
    pub node_id: String,
    pub canonical_path: String,
    pub canonical_localized_path: String,
    /// The requested path is not the canonical localized one (answer 301).
    pub redirect: bool,
    pub served_by: LocalizedServedBy,
}

/// The localized name index, as a backend exposes it.
pub trait LocalizedNameSource: Send + Sync {
    /// Resolve `path` in `locale` at `max_revision` (`None`: HEAD).
    fn resolve(
        &self,
        scope: StorageScope<'_>,
        locale: &str,
        path: &str,
        max_revision: Option<&HLC>,
    ) -> Result<Option<LocalizedResolution>>;

    /// The canonical localized path of a node in `locale` — `None` when the
    /// node, or an ancestor, is missing or hidden there.
    fn localized_path(
        &self,
        scope: StorageScope<'_>,
        node_id: &str,
        locale: &str,
        max_revision: Option<&HLC>,
    ) -> Result<Option<String>>;

    /// The node's OWN translated name in `locale` (the first locale of its
    /// fallback chain that gives it one) — `None` when it has none there, or is
    /// missing or hidden.
    fn node_name(
        &self,
        scope: StorageScope<'_>,
        node_id: &str,
        locale: &str,
        max_revision: Option<&HLC>,
    ) -> Result<Option<String>>;

    /// A reader of many nodes' [`Self::node_name`] / [`Self::localized_path`]
    /// in ONE `(scope, locale, max_revision)`, which remembers what it read:
    /// a localized path is the segments of every ancestor, and the rows of a
    /// tree read share their ancestors. Answers exactly what the single
    /// calls answer.
    ///
    /// Only for a reader whose revision is FIXED for the session's life (a
    /// SQL statement's snapshot): what it remembers is true at that revision.
    fn session(
        &self,
        scope: StorageScope<'_>,
        locale: &str,
        max_revision: Option<&HLC>,
    ) -> Result<Arc<dyn LocalizedNameSession>>;
}

/// See [`LocalizedNameSource::session`].
pub trait LocalizedNameSession: Send + Sync {
    /// For each of `node_ids`, its own name (when `want_name`) and its
    /// canonical localized path (when `want_path`) — `None` where the single
    /// calls answer `None`, and for what was not asked.
    fn names(
        &self,
        node_ids: &[&str],
        want_name: bool,
        want_path: bool,
    ) -> Result<Vec<LocalizedNames>>;

    /// [`Self::names`] for nodes the caller already READ at the session's
    /// revision, with their node-level overlays in a fallback chain (what
    /// translation resolution read for the same rows). Where `chain` is the
    /// session's own chain, those are exactly what the session would read
    /// for the node itself, so it does not read them again; otherwise it
    /// reads as [`Self::names`] does.
    fn names_known(
        &self,
        known: &[KnownNode<'_>],
        want_name: bool,
        want_path: bool,
    ) -> Result<Vec<LocalizedNames>>;
}

/// A node as a caller of [`LocalizedNameSession::names_known`] already holds it.
#[derive(Debug, Clone, Copy)]
pub struct KnownNode<'a> {
    /// The node's record at the session's revision (its id, name and path
    /// are what is used).
    pub node: &'a raisin_models::nodes::Node,
    /// The fallback chain `overlays` is aligned with.
    pub chain: &'a [String],
    /// The node's live node-level overlay in each chain locale.
    pub overlays: &'a [Option<raisin_models::translations::LocaleOverlay>],
}

/// One node's answer from [`LocalizedNameSession::names`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LocalizedNames {
    pub node_name: Option<String>,
    pub localized_path: Option<String>,
}
