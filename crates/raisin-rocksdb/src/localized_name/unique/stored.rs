//! A node's effective names as STORED, so the uniqueness check refuses only
//! what a write changes (the way `UNIQUE_INDEX` checks only changed values).
//!
//! Replication apply, merge and cross-branch promotion may store a collision
//! (they are never refused — see the module doc of `unique`). Judged on every
//! locale, every later local write of either colliding node was refused — a
//! property-only update, another locale's overlay, a package re-install of
//! the record — until an operator renamed one of them.

use super::super::config::NameConfig;
use super::super::keys::NameScope;
use super::super::reads::{node_head_at, overlays_at, parent_path_of};
use super::super::sync::Overrides;
use crate::indexing::localized_node_names::{node_name_in, NameIn};
use raisin_error::Result;
use raisin_models::nodes::Node;
use raisin_models::translations::LocaleOverlay;
use rocksdb::DB;
use std::collections::{BTreeMap, BTreeSet};

/// `node`'s effective segment in `locale` with `overlays`: its translated
/// name there (`true`), else its canonical name (`false`); `None` when it is
/// hidden there. The ONE derivation the check and the stored view share.
pub(super) fn effective(
    overlays: &BTreeMap<String, LocaleOverlay>,
    locale: &str,
    cfg: &NameConfig,
    node: &Node,
) -> Option<(String, bool)> {
    match node_name_in(overlays.get(locale), locale, cfg) {
        NameIn::Hidden => None,
        NameIn::Name(name) => Some((name, true)),
        NameIn::None => Some((canonical(node).to_string(), false)),
    }
}

fn canonical(node: &Node) -> &str {
    node.path.rsplit('/').next().unwrap_or(node.name.as_str())
}

/// The newest stored version of a node: its parent path and its effective
/// name per locale (`None`: hidden there).
pub(super) struct StoredNames {
    parent_path: String,
    names: BTreeMap<String, Option<String>>,
}

impl StoredNames {
    /// `None` for a node not stored (a create, a copy).
    pub(super) fn load(
        db: &DB,
        scope: NameScope<'_>,
        cfg: &NameConfig,
        node_id: &str,
        locales: &BTreeSet<String>,
    ) -> Result<Option<Self>> {
        if node_id.is_empty() {
            return Ok(None);
        }
        let Some((_, Some(node))) = node_head_at(db, scope, node_id, None)? else {
            return Ok(None);
        };
        let overlays = overlays_at(
            db,
            scope,
            node_id,
            None,
            &Overrides::new(),
            &crate::mvcc_read::NEWEST,
        )?;
        let names = locales
            .iter()
            .map(|locale| {
                let name = effective(&overlays, locale, cfg, &node).map(|(name, _)| name);
                (locale.clone(), name)
            })
            .collect();
        Ok(Some(Self {
            parent_path: parent_path_of(&node.path),
            names,
        }))
    }

    /// Whether the node already stands under `parent_path` named `name` in
    /// `locale` — nothing this write introduces.
    pub(super) fn unchanged(&self, parent_path: &str, locale: &str, name: &str) -> bool {
        self.parent_path == parent_path
            && self
                .names
                .get(locale)
                .is_some_and(|stored| stored.as_deref() == Some(name))
    }
}
