//! `__node_name` / `__localized_path` for many nodes of one
//! `(scope, locale, bound)` — THE implementation of both: the single-node
//! calls ([`LocalizedLookup::localized_path`], [`LocalizedLookup::node_name`])
//! are a fresh session asked once.
//!
//! A localized path is the segment of every ancestor, each resolved through
//! `PATH_INDEX` and a [`NodeView`] (node head, chain overlays, a walk of the
//! node's history per live overlay). A tree read in `fr` asked that for every
//! row from the root down, so a page of siblings re-resolved the same
//! ancestors once per row, and a row naming both columns loaded its own view
//! three times. The session remembers each canonical path's segment (or that
//! the node there is missing or hidden), so every ancestor is resolved once
//! per session — which is sound only because the bound is fixed for the
//! session's life (`LocalizedNameSource::session`).

use super::view::{join_path, NodeView};
use super::LocalizedLookup;
use crate::localized_name::config::{self, NameConfig};
use crate::localized_name::keys::NameScope;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_storage::localized::{KnownNode, LocalizedNameSession, LocalizedNames};
use rocksdb::DB;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// See the module docs.
pub(crate) struct NameSession {
    db: Arc<DB>,
    tenant_id: String,
    repo_id: String,
    branch: String,
    workspace: String,
    bound: Option<HLC>,
    /// `None`: the repository is unknown here — every answer is `None`.
    cfg: Option<NameConfig>,
    chain: Vec<String>,
    /// Canonical path -> its segment in the locale; `None` when no visible
    /// node is there (which makes every path through it `None`).
    segments: Mutex<HashMap<String, Option<String>>>,
}

impl NameSession {
    pub(crate) fn open(
        db: Arc<DB>,
        scope: NameScope<'_>,
        locale: &str,
        bound: Option<&HLC>,
    ) -> Result<Self> {
        let cfg = config::load(&db, scope.tenant_id, scope.repo_id)?;
        let chain = cfg
            .as_ref()
            .map(|cfg| cfg.fallback_chain(locale))
            .unwrap_or_default();
        Ok(Self {
            db,
            tenant_id: scope.tenant_id.to_string(),
            repo_id: scope.repo_id.to_string(),
            branch: scope.branch.to_string(),
            workspace: scope.workspace.to_string(),
            bound: bound.copied(),
            cfg,
            chain,
            segments: Mutex::new(HashMap::new()),
        })
    }

    fn scope(&self) -> NameScope<'_> {
        NameScope::new(
            &self.tenant_id,
            &self.repo_id,
            &self.branch,
            &self.workspace,
        )
    }

    /// The answer for one node through `src`.
    fn names_in(
        &self,
        src: &mut crate::mvcc_read::SnapshotRead<'_>,
        cfg: &NameConfig,
        node_id: &str,
        want_name: bool,
        want_path: bool,
    ) -> Result<LocalizedNames> {
        let scope = self.scope();
        let bound = self.bound.as_ref();
        match NodeView::load_in(src, scope, node_id, &self.chain, bound)? {
            Some(target) => self.names_of(src, cfg, &target, want_name, want_path),
            None => Ok(LocalizedNames::default()),
        }
    }

    /// The answer for a loaded (or caller-supplied) view of the node.
    fn names_of(
        &self,
        src: &mut crate::mvcc_read::SnapshotRead<'_>,
        cfg: &NameConfig,
        target: &NodeView,
        want_name: bool,
        want_path: bool,
    ) -> Result<LocalizedNames> {
        let mut out = LocalizedNames::default();
        if want_name && target.visible() {
            out.node_name = self
                .chain
                .iter()
                .find_map(|l| match target.name_in(l, cfg) {
                    crate::indexing::localized_node_names::NameIn::Name(s) => Some(s),
                    _ => None,
                });
        }
        if want_path {
            out.localized_path = self.localized_path_in(src, cfg, target)?;
        }
        Ok(out)
    }

    /// The view the session would load for `known`, from what the caller
    /// read at the same revision — `None` when its overlays are of another
    /// chain (then the session reads for itself).
    fn known_view(&self, known: &KnownNode<'_>) -> Option<NodeView> {
        if known.chain != self.chain.as_slice() || known.overlays.len() != self.chain.len() {
            return None;
        }
        // A view consults the node's id, name and path, never its
        // properties (`node_head_at` skips them for the same reason), and
        // its record revision only to order index claims — not here.
        let node = raisin_models::nodes::Node {
            id: known.node.id.clone(),
            name: known.node.name.clone(),
            path: known.node.path.clone(),
            ..Default::default()
        };
        let overlays = self
            .chain
            .iter()
            .cloned()
            .zip(known.overlays.iter().cloned())
            .collect();
        Some(NodeView::from_parts(node, HLC::new(0, 0), overlays))
    }

    /// The canonical localized path of `target`: each ancestor's segment from
    /// the root down, resolved by canonical path (`by_path`), remembered.
    fn localized_path_in(
        &self,
        src: &mut crate::mvcc_read::SnapshotRead<'_>,
        cfg: &NameConfig,
        target: &NodeView,
    ) -> Result<Option<String>> {
        let scope = self.scope();
        let bound = self.bound.as_ref();
        let mut segments = Vec::new();
        let mut path = "/".to_string();
        for name in target.node.path.split('/').filter(|s| !s.is_empty()) {
            path = join_path(&path, name);
            let known = self
                .segments
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&path)
                .cloned();
            let segment = match known {
                Some(segment) => segment,
                None => {
                    // The row's own segment reuses its view: `PATH_INDEX`
                    // naming the target means `by_path` would load exactly it.
                    let segment = LocalizedLookup::by_path_known(
                        src,
                        scope,
                        &self.chain,
                        &path,
                        bound,
                        Some(target),
                    )?
                    .filter(|view| view.visible())
                    .map(|view| view.segment(&self.chain, cfg));
                    self.segments
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .insert(path.clone(), segment.clone());
                    segment
                }
            };
            let Some(segment) = segment else {
                return Ok(None);
            };
            segments.push(segment);
        }
        Ok(Some(format!("/{}", segments.join("/"))))
    }
}

impl LocalizedNameSession for NameSession {
    fn names(
        &self,
        node_ids: &[&str],
        want_name: bool,
        want_path: bool,
    ) -> Result<Vec<LocalizedNames>> {
        let Some(cfg) = self.cfg.as_ref() else {
            return Ok(vec![LocalizedNames::default(); node_ids.len()]);
        };
        if !want_name && !want_path {
            return Ok(vec![LocalizedNames::default(); node_ids.len()]);
        }
        // One snapshot-pinned iterator per column family for the whole call
        // (plan Phase 13d).
        let snapshot = self.db.snapshot();
        let mut src = crate::mvcc_read::SnapshotRead::new(&self.db, &snapshot);
        node_ids
            .iter()
            .map(|id| self.names_in(&mut src, cfg, id, want_name, want_path))
            .collect()
    }

    fn names_known(
        &self,
        known: &[KnownNode<'_>],
        want_name: bool,
        want_path: bool,
    ) -> Result<Vec<LocalizedNames>> {
        let Some(cfg) = self.cfg.as_ref() else {
            return Ok(vec![LocalizedNames::default(); known.len()]);
        };
        if !want_name && !want_path {
            return Ok(vec![LocalizedNames::default(); known.len()]);
        }
        let snapshot = self.db.snapshot();
        let mut src = crate::mvcc_read::SnapshotRead::new(&self.db, &snapshot);
        known
            .iter()
            .map(|k| match self.known_view(k) {
                Some(view) => self.names_of(&mut src, cfg, &view, want_name, want_path),
                None => self.names_in(&mut src, cfg, &k.node.id, want_name, want_path),
            })
            .collect()
    }
}
