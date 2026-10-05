//! One node as a localized lookup sees it at a read revision: the node, its
//! overlays in the requested locale's fallback chain, and what THE selector
//! says about it there. Candidate verification, visibility and the segment a
//! canonical localized path uses all come from here, so the index path and the
//! fallback path cannot disagree.

use crate::indexing::localized_node_names::{node_name_in, NameIn, NODE_NAME_POINTER};
use crate::localized_name::config::NameConfig;
use crate::localized_name::keys::NameScope;
use crate::localized_name::sync::parent_path_of;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use raisin_models::translations::LocaleOverlay;
use rocksdb::DB;
use std::collections::BTreeMap;

/// A live node and its chain overlays as of the read revision.
#[derive(Clone)]
pub(crate) struct NodeView {
    /// The node record WITHOUT its property map (`node_head_at`): a lookup
    /// verifies ids, names and paths, never properties.
    pub(crate) node: Node,
    /// The revision of the node's record the read saw.
    pub(crate) revision: HLC,
    overlays: BTreeMap<String, Option<LocaleOverlay>>,
}

impl NodeView {
    /// Load `node_id` as of `bound` with its overlays in `chain`. `None` when
    /// the node does not exist or is deleted there.
    pub(crate) fn load(
        db: &DB,
        scope: NameScope<'_>,
        node_id: &str,
        chain: &[String],
        bound: Option<&HLC>,
    ) -> Result<Option<Self>> {
        Self::load_in(
            &mut crate::mvcc_read::DbRead(db),
            scope,
            node_id,
            chain,
            bound,
        )
    }

    /// [`Self::load`] through a read source — a lookup reads every segment
    /// through ONE set of iterators pinned to one snapshot (plan Phase 13d).
    /// Each overlay is decoded down to what a view consults: whether it
    /// hides the node, and its translated name.
    pub(crate) fn load_in(
        src: &mut impl crate::mvcc_read::VersionedRead,
        scope: NameScope<'_>,
        node_id: &str,
        chain: &[String],
        bound: Option<&HLC>,
    ) -> Result<Option<Self>> {
        let Some((revision, Some(node))) =
            crate::localized_name::reads::node_head_at_in(src, scope, node_id, bound)?
        else {
            return Ok(None);
        };
        let names_only = |value: &[u8]| {
            crate::translation_read::decode_overlay_keeping(value, NODE_NAME_POINTER)
        };
        let mut overlays = BTreeMap::new();
        for locale in chain {
            let overlay = crate::translation_read::read_version_in(
                src,
                (
                    scope.tenant_id,
                    scope.repo_id,
                    scope.branch,
                    scope.workspace,
                ),
                node_id,
                locale,
                bound,
                &names_only,
            )?
            .and_then(|version| version.overlay);
            overlays.insert(locale.clone(), overlay);
        }
        Ok(Some(Self {
            node,
            revision,
            overlays,
        }))
    }

    /// A view assembled by the caller: a node and its chain overlays as an
    /// uncommitted transaction will leave them (sibling uniqueness, plan
    /// Phase 13c), judged by the same methods as a loaded view.
    pub(crate) fn from_parts(
        node: Node,
        revision: HLC,
        overlays: BTreeMap<String, Option<LocaleOverlay>>,
    ) -> Self {
        Self {
            node,
            revision,
            overlays,
        }
    }

    /// The selector's answer in one chain locale.
    pub(crate) fn name_in(&self, locale: &str, cfg: &NameConfig) -> NameIn {
        let overlay = self.overlays.get(locale).and_then(|o| o.as_ref());
        node_name_in(overlay, locale, cfg)
    }

    /// Visible in the requested locale: hidden NOWHERE in its fallback chain
    /// (the translation resolver's rule).
    pub(crate) fn visible(&self) -> bool {
        !self
            .overlays
            .values()
            .any(|o| matches!(o, Some(LocaleOverlay::Hidden)))
    }

    /// Its URL segment in the requested locale: the first chain locale with a
    /// name of its own, else its canonical name.
    pub(crate) fn segment(&self, chain: &[String], cfg: &NameConfig) -> String {
        for locale in chain {
            if let NameIn::Name(name) = self.name_in(locale, cfg) {
                return name;
            }
        }
        self.canonical_name().to_string()
    }

    pub(crate) fn canonical_name(&self) -> &str {
        self.node
            .path
            .rsplit('/')
            .next()
            .unwrap_or(self.node.name.as_str())
    }

    /// Whether this node is a child of `parent_path` with `name` in `locale`
    /// — the verification of an index claim (and the fallback's test).
    pub(crate) fn answers(
        &self,
        parent_path: &str,
        locale: &str,
        name: &str,
        cfg: &NameConfig,
    ) -> bool {
        parent_path_of(&self.node.path) == parent_path
            && matches!(self.name_in(locale, cfg), NameIn::Name(s) if s == name)
    }
}

/// `parent` + `/` + `segment` (`/` + segment at the root).
pub(crate) fn join_path(parent: &str, segment: &str) -> String {
    if parent == "/" {
        format!("/{segment}")
    } else {
        format!("{parent}/{segment}")
    }
}
