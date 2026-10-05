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
use raisin_storage::{Storage, StorageScope};

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
    let source = ctx.storage.localized_names();
    let scope = StorageScope::new(&ctx.tenant_id, &ctx.repo_id, &ctx.branch, workspace);
    let snapshot = ctx.statement_snapshot().await?;
    let text = |v: Option<String>| v.map_or(PropertyValue::Null, PropertyValue::String);
    if want_name {
        let name = match &source {
            Some(s) => s.node_name(scope, &node.id, locale, Some(&snapshot))?,
            None => None,
        };
        row.insert(format!("{qualifier}.__node_name"), text(name));
    }
    if want_path {
        let path = match (known_path, &source) {
            (Some(known), _) => Some(known.to_string()),
            (None, Some(s)) => s.localized_path(scope, &node.id, locale, Some(&snapshot))?,
            (None, None) => None,
        };
        row.insert(format!("{qualifier}.__localized_path"), text(path));
    }
    Ok(())
}
