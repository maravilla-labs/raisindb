// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Virtual localized URL columns (plan Phase 12): `__node_name` and
//! `__localized_path`, from the storage's localized name lookup — the same
//! selector the index, the HTTP and WS lookups use. Opt-in: populated only
//! when NAMED in the projection, for the row's effective locale.

use crate::physical_plan::executor::{ExecutionContext, Row};
use raisin_error::Error;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_models::translations::LocaleOverlay;
use raisin_storage::localized::{KnownNode, LocalizedNames};
use raisin_storage::Storage;

/// `known_path`: the row's canonical localized path when the scan already
/// resolved it (`LocalizedPathLookup`); computed otherwise.
#[allow(clippy::too_many_arguments)]
pub(super) async fn insert_localized_fields<S: Storage>(
    row: &mut Row,
    node: &Node,
    qualifier: &str,
    workspace: &str,
    ctx: &ExecutionContext<S>,
    locale: &str,
    projection: &Option<Vec<String>>,
    known_path: Option<&str>,
    known_names: Option<&LocalizedNames>,
) -> Result<(), Error> {
    let named = |col: &str| {
        projection
            .as_ref()
            .is_some_and(|p| p.iter().any(|c| c == col))
    };
    let (want_name, want_path) = (named("__node_name"), named("__localized_path"));
    if !want_name && !want_path {
        return Ok(());
    }
    // A path the scan already resolved is not asked again.
    let ask_path = want_path && known_path.is_none();
    let names = if let Some(known) = known_names {
        known.clone()
    } else if want_name || ask_path {
        match ctx.localized_name_session(workspace, locale).await? {
            Some(session) => session
                .names(&[node.id.as_str()], want_name, ask_path)?
                .pop()
                .unwrap_or_default(),
            None => LocalizedNames::default(),
        }
    } else {
        LocalizedNames::default()
    };
    let text = |v: Option<String>| v.map_or(PropertyValue::Null, PropertyValue::String);
    if want_name {
        row.insert(format!("{qualifier}.__node_name"), text(names.node_name));
    }
    if want_path {
        let path = match known_path {
            Some(known) => Some(known.to_string()),
            None => names.localized_path,
        };
        row.insert(format!("{qualifier}.__localized_path"), text(path));
    }
    Ok(())
}

/// Which of `__node_name` / `__localized_path` the projection names.
pub(crate) fn wants_localized_names(projection: &Option<Vec<String>>) -> (bool, bool) {
    let named = |col: &str| {
        projection
            .as_ref()
            .is_some_and(|p| p.iter().any(|c| c == col))
    };
    (named("__node_name"), named("__localized_path"))
}

/// The localized name columns of a whole page of a scan in one read, aligned
/// with `nodes` — handed to [`super::node_to_row`] through
/// `OrderContext::names`. `None` when the projection names neither.
///
/// `known`: the fallback chain and each node's node-level overlays in it, as
/// translation resolution of these rows read them at the statement snapshot
/// — then the name reader does not read them again.
pub(crate) async fn page_names<S: Storage>(
    ctx: &ExecutionContext<S>,
    workspace: &str,
    locale: &str,
    projection: &Option<Vec<String>>,
    nodes: &[&Node],
    known: Option<(&[String], &[Vec<Option<LocaleOverlay>>])>,
) -> Result<Option<Vec<LocalizedNames>>, Error> {
    let (want_name, want_path) = wants_localized_names(projection);
    if !want_name && !want_path {
        return Ok(None);
    }
    let Some(session) = ctx.localized_name_session(workspace, locale).await? else {
        return Ok(Some(vec![LocalizedNames::default(); nodes.len()]));
    };
    let known = known.filter(|(_, overlays)| overlays.len() == nodes.len());
    // The read is synchronous: an unbounded subtree is not one call.
    let mut out = Vec::with_capacity(nodes.len());
    for (at, chunk) in nodes.chunks(256).enumerate() {
        match known {
            Some((chain, overlays)) => {
                let rows: Vec<KnownNode<'_>> = chunk
                    .iter()
                    .zip(&overlays[at * 256..])
                    .map(|(node, overlays)| KnownNode {
                        node,
                        chain,
                        overlays,
                    })
                    .collect();
                out.extend(session.names_known(&rows, want_name, want_path)?);
            }
            None => {
                let ids: Vec<&str> = chunk.iter().map(|n| n.id.as_str()).collect();
                out.extend(session.names(&ids, want_name, want_path)?);
            }
        }
        tokio::task::yield_now().await;
    }
    Ok(Some(out))
}
