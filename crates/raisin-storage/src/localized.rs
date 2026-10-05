//! Localized URL lookup (plan Phase 12): the storage-level contract behind
//! `NodeService::resolve_localized_path` and every surface that calls it.

use raisin_error::Result;
use raisin_hlc::HLC;

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
}
