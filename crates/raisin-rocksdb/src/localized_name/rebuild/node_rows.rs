//! One node's rows in a build (see `rebuild`), and the claim verification
//! the collision pass uses.

use super::LocalizedNameCounts;
use crate::indexing::localized_node_names::{localized_node_names, NameIn};
use crate::localized_name::config;
use crate::localized_name::keys::{self, NameScope};
use crate::localized_name::lookup::view::NodeView;
use crate::localized_name::plan::Prior;
use crate::localized_name::sync::{overlays_at, parent_key, plan_rows, Overrides};
use raisin_error::Result;
use raisin_hlc::HLC;
use rocksdb::DB;
use std::collections::BTreeMap;

/// The rows one node needs as of the pin (`None`: nothing to write).
pub(super) fn node_rows(
    db: &DB,
    scope: NameScope<'_>,
    node_id: &str,
    cfg: &config::NameConfig,
    pin: &HLC,
    parents: &mut BTreeMap<(String, String), Option<String>>,
    counts: &mut LocalizedNameCounts,
) -> Result<Option<Vec<(Vec<u8>, Vec<u8>)>>> {
    let Some(view) = NodeView::load(db, scope, node_id, &[], Some(pin))? else {
        return Ok(None);
    };
    counts.nodes += 1;
    let node = &view.node;
    let overlays = overlays_at(db, scope, node_id, Some(pin), &Overrides::new(), pin)?;
    let desired = localized_node_names(&overlays, cfg);
    // At or above every record of the node up to the pin (see the module
    // doc): a row an older configuration, a deleted overlay, a catch-up or a
    // peer left between the node's version and the pin must not outrank the
    // rebuilt state, nor be what the diff below compares against.
    let at = [
        Some(view.revision),
        crate::localized_name::catch_up::newest_translation_revision(
            db,
            scope,
            node_id,
            Some(pin),
        )?,
        newest_reverse_revision_at(db, scope, node_id, pin)?,
    ]
    .into_iter()
    .flatten()
    .max()
    .unwrap_or(view.revision);
    let parent = if desired.is_empty() {
        None
    } else {
        let parent_path = crate::localized_name::sync::parent_path_of(&node.path);
        let cache_key = (scope.workspace.to_string(), parent_path);
        let parent = match parents.get(&cache_key) {
            Some(p) => p.clone(),
            None => {
                let p = parent_key(db, scope, node, None, pin)?;
                parents.insert(cache_key, p.clone());
                p
            }
        };
        match parent {
            Some(parent) => Some(parent),
            // Unreachable at the pin (an orphan): the fallback cannot reach
            // it either, so there is nothing to index.
            None => return Ok(None),
        }
    };
    let planned = plan_rows(
        db,
        scope,
        node_id,
        parent.as_deref(),
        &desired,
        &at,
        &Prior::default(),
    )?;
    if planned.changed == 0 {
        return Ok(None);
    }
    counts.rewritten += 1;
    Ok(Some(planned.rows))
}

/// The newest reverse row revision of the node at or below `pin`, any locale.
fn newest_reverse_revision_at(
    db: &DB,
    scope: NameScope<'_>,
    node_id: &str,
    pin: &HLC,
) -> Result<Option<HLC>> {
    Ok(
        crate::localized_name::rows::reverse_rows(db, scope, node_id, Some(pin))?
            .into_values()
            .map(|(rev, _)| rev)
            .max(),
    )
}

/// Whether a stored claim still verifies (used by the collision pass).
pub(crate) fn claim_verifies(
    db: &DB,
    scope: NameScope<'_>,
    claim: &keys::ForwardKey,
    cfg: &config::NameConfig,
    pin: &HLC,
) -> Result<bool> {
    let chain = [claim.locale.clone()];
    let Some(view) = NodeView::load(db, scope, &claim.node_id, &chain, Some(pin))? else {
        return Ok(false);
    };
    if !matches!(view.name_in(&claim.locale, cfg), NameIn::Name(s) if s == claim.name) {
        return Ok(false);
    }
    Ok(parent_key(db, scope, &view.node, None, pin)?.as_deref() == Some(claim.parent_id.as_str()))
}
