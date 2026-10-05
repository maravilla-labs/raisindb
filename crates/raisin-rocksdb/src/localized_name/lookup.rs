//! Localized path lookup: `/produits/chaise` in `fr` -> the node.
//!
//! Walks the segments from the workspace root and stops at the first one that
//! does not resolve. Per segment, under the parent resolved so far:
//!
//! 1. each locale of `get_fallback_chain(L)` (bar the default language) in
//!    order: the index's claims on `(locale, parent, segment)`, newest first,
//!    each VERIFIED through the selector at the read revision (`view.rs`) —
//!    or, while the index is not `Ready` for this read, every child of the
//!    parent tested the same way (the row-level fallback: always correct,
//!    O(children));
//! 2. if none matches, the canonical name through `PATH_INDEX`;
//! 3. a node hidden anywhere in the chain is not there.
//!
//! The answer carries the canonical path and the canonical LOCALIZED path
//! (each node's own segment in `L`); `redirect` is set when the requested
//! path is not the canonical localized one (a canonical name used where the
//! node has a name of its own, or a fallback locale's name), so a caller can
//! answer 301. RLS is the caller's (`NodeService`): missing, forbidden and
//! hidden must all look the same there.

mod paths;
pub(crate) mod view;

use super::config::{self, NameConfig};
use super::keys::{NameScope, ROOT_PARENT};
use super::state::{self, Availability};
use crate::mvcc_read::VersionedRead;
use crate::repositories::nodes::NodeRepositoryImpl;
use raisin_error::Result;
use raisin_hlc::HLC;
use rocksdb::{SnapshotWithThreadMode, DB};
use view::{join_path, NodeView};

/// How a lookup was answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServedBy {
    /// The locale is the default language: the path is canonical.
    DefaultLanguage,
    /// Through the index.
    Index,
    /// Through the row-level fallback (index not ready for this read).
    Fallback,
}

/// A resolved localized path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution {
    pub node_id: String,
    pub canonical_path: String,
    pub canonical_localized_path: String,
    /// The requested path is not the canonical localized path (answer 301).
    pub redirect: bool,
    pub served_by: ServedBy,
}

/// The lookup over one storage.
pub struct LocalizedLookup<'a> {
    nodes: &'a NodeRepositoryImpl,
}

impl<'a> LocalizedLookup<'a> {
    pub fn new(nodes: &'a NodeRepositoryImpl) -> Self {
        Self { nodes }
    }

    /// Whether the index may answer reads of this workspace at `bound`, as
    /// the database stands now.
    pub fn availability(
        &self,
        scope: NameScope<'_>,
        cfg: &NameConfig,
        bound: Option<&HLC>,
    ) -> Result<Availability> {
        let db = self.nodes.db_handle();
        Self::availability_in(&mut crate::mvcc_read::DbRead(db), scope, cfg, bound)
    }

    /// [`Self::availability`] through `src`'s view.
    fn availability_in(
        src: &mut impl VersionedRead,
        scope: NameScope<'_>,
        cfg: &NameConfig,
        bound: Option<&HLC>,
    ) -> Result<Availability> {
        if !super::enabled() {
            return Ok(Availability::NotBuilt);
        }
        let record = state::read_in(
            src,
            scope.tenant_id,
            scope.repo_id,
            scope.branch,
            scope.workspace,
        )?;
        Ok(state::availability(
            record.as_ref(),
            &cfg.fingerprint(),
            bound,
        ))
    }

    /// Resolve `path` in `locale` at `bound` (`None`: HEAD). `Ok(None)`: no
    /// such localized path (or the repository is unknown here).
    pub fn resolve(
        &self,
        scope: NameScope<'_>,
        locale: &str,
        path: &str,
        bound: Option<&HLC>,
    ) -> Result<Option<Resolution>> {
        let snapshot = self.nodes.db_handle().snapshot();
        self.resolve_in(&snapshot, scope, locale, path, bound)
    }

    /// [`Self::resolve`] against a snapshot the caller holds.
    ///
    /// EVERY read of the lookup goes through it (plan Phase 13d, one iterator
    /// per column family): the repository config, the index's build state,
    /// each segment's claims, node heads, overlays, path index entries and
    /// the fallback's child lists. The state record is what vouches for the
    /// claims — a build commits its last claims and only then stamps `Ready`
    /// — so reading it from a later view than the claims answers a false miss
    /// for a node the index already claims to cover.
    pub fn resolve_in(
        &self,
        snapshot: &SnapshotWithThreadMode<'_, DB>,
        scope: NameScope<'_>,
        locale: &str,
        path: &str,
        bound: Option<&HLC>,
    ) -> Result<Option<Resolution>> {
        let mut src = crate::mvcc_read::SnapshotRead::new(self.nodes.db_handle(), snapshot);
        let src = &mut src;
        let Some(cfg) = config::load_in(src, scope.tenant_id, scope.repo_id)? else {
            return Ok(None);
        };
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        if segments.is_empty() {
            return Ok(None);
        }
        let requested = format!("/{}", segments.join("/"));
        let chain: Vec<String> = cfg.fallback_chain(locale);
        if locale == cfg.default_language {
            return self.resolve_canonical(src, scope, &chain, &requested, bound);
        }
        let availability = Self::availability_in(src, scope, &cfg, bound)?;
        let use_index = availability.is_ready();
        if !use_index && !matches!(availability, Availability::BelowBuild { .. }) {
            super::auto::request_build(scope.tenant_id, scope.repo_id, scope.branch);
        }
        let name_locales: Vec<&String> = chain
            .iter()
            .filter(|l| **l != cfg.default_language)
            .collect();

        let (mut parent_id, mut parent_path) = (ROOT_PARENT.to_string(), "/".to_string());
        let mut localized = Vec::with_capacity(segments.len());
        let mut found: Option<NodeView> = None;
        for segment in &segments {
            let view = match self.by_name(
                src,
                scope,
                &cfg,
                &chain,
                &name_locales,
                (&parent_id, &parent_path),
                segment,
                bound,
                use_index,
            )? {
                Some(view) => view,
                None => {
                    let child_path = join_path(&parent_path, segment);
                    match self.by_path(src, scope, &chain, &child_path, bound)? {
                        Some(view) => view,
                        None => return Ok(None),
                    }
                }
            };
            if !view.visible() {
                return Ok(None);
            }
            localized.push(view.segment(&chain, &cfg));
            parent_id = view.node.id.clone();
            parent_path = view.node.path.clone();
            found = Some(view);
        }
        let Some(view) = found else { return Ok(None) };
        let canonical_localized_path = format!("/{}", localized.join("/"));
        Ok(Some(Resolution {
            node_id: view.node.id.clone(),
            canonical_path: view.node.path.clone(),
            redirect: canonical_localized_path != requested,
            canonical_localized_path,
            served_by: if use_index {
                ServedBy::Index
            } else {
                ServedBy::Fallback
            },
        }))
    }

    /// The node at a canonical path.
    fn by_path(
        &self,
        src: &mut impl VersionedRead,
        scope: NameScope<'_>,
        chain: &[String],
        path: &str,
        bound: Option<&HLC>,
    ) -> Result<Option<NodeView>> {
        let entry = crate::mvcc_read::path_index_entry_in(
            src,
            scope.tenant_id,
            scope.repo_id,
            scope.branch,
            scope.workspace,
            path,
            bound,
        )?;
        let Some((_, Some(id))) = entry else {
            return Ok(None);
        };
        Ok(NodeView::load_in(src, scope, &id, chain, bound)?.filter(|v| v.node.path == path))
    }

    /// The child of the parent whose own name is `segment` in the first chain
    /// locale that has one.
    #[allow(clippy::too_many_arguments)]
    fn by_name(
        &self,
        src: &mut impl VersionedRead,
        scope: NameScope<'_>,
        cfg: &NameConfig,
        chain: &[String],
        name_locales: &[&String],
        (parent_id, parent_path): (&str, &str),
        segment: &str,
        bound: Option<&HLC>,
        use_index: bool,
    ) -> Result<Option<NodeView>> {
        if use_index {
            for locale in name_locales {
                let claims = super::rows::claims_in(src, scope, locale, parent_id, segment, bound)?;
                for (_, node_id) in claims {
                    if let Some(view) = NodeView::load_in(src, scope, &node_id, chain, bound)? {
                        // A claimant hidden in the requested locale is not
                        // there; a visible sibling may still answer.
                        if view.visible() && view.answers(parent_path, locale, segment, cfg) {
                            return Ok(Some(view));
                        }
                    }
                }
            }
            return Ok(None);
        }
        // Row-level fallback: every child, through the same selector.
        let children = self.nodes.ordered_child_ids_at(
            scope.tenant_id,
            scope.repo_id,
            scope.branch,
            scope.workspace,
            parent_id,
            bound,
            src.snapshot(),
        )?;
        let mut views = Vec::with_capacity(children.len());
        for child in children {
            if let Some(view) = NodeView::load_in(src, scope, &child, chain, bound)? {
                views.push(view);
            }
        }
        // Newest record first, as the index orders claims.
        views.sort_by(|a, b| {
            b.revision
                .cmp(&a.revision)
                .then_with(|| a.node.id.cmp(&b.node.id))
        });
        for locale in name_locales {
            if let Some(i) = views
                .iter()
                .position(|v| v.visible() && v.answers(parent_path, locale, segment, cfg))
            {
                return Ok(Some(views.swap_remove(i)));
            }
        }
        Ok(None)
    }
}
