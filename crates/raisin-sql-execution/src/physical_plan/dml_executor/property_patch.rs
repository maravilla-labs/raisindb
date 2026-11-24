// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! `UPDATE ... SET properties = properties || {...}` as a PARTIAL update.
//!
//! Evaluated against a row read before the write, the merge rewrites the
//! whole property map, and a write of another property of the same node
//! that commits in between is lost. Recognised here, the statement becomes a
//! [`PropertyPatch`] of only the keys on the right-hand side, which the
//! storage merges into the node's state as stored when the write lands
//! (`TransactionalContext::patch_node_properties`).
//!
//! `SET properties = $1` (no `properties ||`) still replaces the whole map:
//! that is the explicit way to replace it.

use raisin_error::Error;
use raisin_models::nodes::properties::PropertyValue;
use raisin_sql::analyzer::{BinaryOperator, Expr, TypedExpr};
use raisin_storage::node_patch::PropertyPatch;

use super::helpers::literal_to_property_value;
use crate::physical_plan::executor::Row;

/// The patch, when every assignment is `properties = properties || <expr>`
/// and `<expr>` reads no column; `None` for any other statement.
pub(super) fn property_merge_patch(
    assignments: &[(String, TypedExpr)],
) -> Result<Option<PropertyPatch>, Error> {
    if assignments.is_empty() {
        return Ok(None);
    }
    let mut patch = PropertyPatch::default();
    for (column, expr) in assignments {
        if !column.eq_ignore_ascii_case("properties") {
            return Ok(None);
        }
        let Expr::BinaryOp {
            left,
            op: BinaryOperator::JsonConcat,
            right,
        } = &expr.expr
        else {
            return Ok(None);
        };
        if !is_properties_column(left) || reads_a_column(right) {
            return Ok(None);
        }
        let Ok(value) = crate::physical_plan::eval::core::eval_expr(right, &Row::new()) else {
            return Ok(None);
        };
        let PropertyValue::Object(map) = literal_to_property_value(&value)? else {
            return Ok(None);
        };
        // A nested `properties` key is flattened by the full-map path; keep
        // that behaviour there.
        if map.contains_key("properties") {
            return Ok(None);
        }
        patch.then(PropertyPatch::setting(map.into_iter().collect()));
    }
    Ok(Some(patch))
}

fn is_properties_column(expr: &TypedExpr) -> bool {
    matches!(&expr.expr, Expr::Column { column, .. } if column.eq_ignore_ascii_case("properties"))
}

fn reads_a_column(expr: &TypedExpr) -> bool {
    if matches!(expr.expr, Expr::Column { .. }) {
        return true;
    }
    let mut found = false;
    expr.for_each_child(&mut |child| {
        if !found && reads_a_column(child) {
            found = true;
        }
    });
    found
}
