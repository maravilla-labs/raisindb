// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

use crate::ast::{FieldFilter, FieldOperators, Filter, NodeSearchQuery, SortOrder};
use raisin_models as models;
use serde_json::Value;

pub fn filter_nodes<'a>(
    nodes: impl IntoIterator<Item = &'a models::nodes::Node>,
    q: &NodeSearchQuery,
) -> Vec<&'a models::nodes::Node> {
    match (&q.and, &q.or, &q.not) {
        (None, None, None) => nodes.into_iter().collect(),
        _ => nodes
            .into_iter()
            .filter(|n| matches_filter(n, &q.and, &q.or, &q.not))
            .collect(),
    }
}

pub fn eval_query<'a>(
    nodes: impl IntoIterator<Item = &'a models::nodes::Node>,
    q: &NodeSearchQuery,
) -> Vec<&'a models::nodes::Node> {
    let mut v: Vec<&models::nodes::Node> = filter_nodes(nodes, q);
    if let Some(order) = &q.order_by {
        // support ordering by known top-level fields: path, id, name, node_type
        v.sort_by(|a, b| {
            let mut ord = std::cmp::Ordering::Equal;
            for (field, dir) in order.iter() {
                let o = match field.as_str() {
                    "path" => a.path.cmp(&b.path),
                    "id" => a.id.cmp(&b.id),
                    "name" => a.name.cmp(&b.name),
                    "node_type" => a.node_type.cmp(&b.node_type),
                    _ => std::cmp::Ordering::Equal,
                };
                ord = match dir {
                    SortOrder::Asc => o,
                    SortOrder::Desc => o.reverse(),
                };
                if ord != std::cmp::Ordering::Equal {
                    break;
                }
            }
            ord
        });
    }
    let total = v.len();
    let offset = q.offset.unwrap_or(0).min(total);
    let limit = q.limit.unwrap_or(usize::MAX);
    let end = offset.saturating_add(limit).min(total);
    v[offset..end].to_vec()
}

fn matches_filter(
    n: &models::nodes::Node,
    ands: &Option<Vec<Filter>>,
    ors: &Option<Vec<Filter>>,
    not: &Option<Box<Filter>>,
) -> bool {
    let and_ok = ands
        .as_ref()
        .is_none_or(|v| v.iter().all(|f| match_filter(n, f)));
    let or_ok = ors
        .as_ref()
        .is_none_or(|v| v.iter().any(|f| match_filter(n, f)));
    let not_ok = not.as_ref().is_none_or(|f| !match_filter(n, f));
    and_ok && or_ok && not_ok
}

fn match_filter(n: &models::nodes::Node, f: &Filter) -> bool {
    match f {
        Filter::And(crate::ast::AndFilter { and }) => and.iter().all(|f| match_filter(n, f)),
        Filter::Or(crate::ast::OrFilter { or }) => or.iter().any(|f| match_filter(n, f)),
        Filter::Not(crate::ast::NotFilter { not }) => !match_filter(n, not),
        Filter::Field(FieldFilter(map)) => {
            map.iter().all(|(field, ops)| match_field(n, field, ops))
        }
    }
}

fn match_field(n: &models::nodes::Node, field: &str, ops: &FieldOperators) -> bool {
    // support node top-level fields first; extend to properties later
    let val = match field {
        "id" => Some(Value::String(n.id.clone())),
        "name" => Some(Value::String(n.name.clone())),
        "path" => Some(Value::String(n.path.clone())),
        "node_type" | "nodeType" => Some(Value::String(n.node_type.clone())),
        "parent" => n.parent.as_ref().map(|s| Value::String(s.clone())),
        _ => None,
    };
    apply_ops(val, ops)
}

fn apply_ops(val: Option<Value>, ops: &FieldOperators) -> bool {
    // exists
    if let Some(exists) = ops.exists {
        if exists != val.is_some() {
            return false;
        }
    }

    if let Some(eq) = &ops.eq {
        if val.as_ref() != Some(eq) {
            return false;
        }
    }
    if let Some(ne) = &ops.ne {
        if val.as_ref() == Some(ne) {
            return false;
        }
    }
    if let Some(pattern) = &ops.like {
        let s = val.as_ref().and_then(|v| v.as_str()).unwrap_or("");
        if !s.contains(pattern) {
            return false;
        }
    }
    if let Some(contains) = &ops.contains {
        // support substring for strings
        match (val.as_ref(), contains) {
            (Some(Value::String(s)), Value::String(needle)) => {
                if !s.contains(needle) {
                    return false;
                }
            }
            _ => return false,
        }
    }
    if let Some(list) = &ops.in_ {
        if !list.iter().any(|v| Some(v) == val.as_ref()) {
            return false;
        }
    }
    // gt/lt/gte/lte could be added later for numeric/date fields when we map them properly
    true
}
