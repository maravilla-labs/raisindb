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

//! PropertyValue helpers and permission/condition parsing.

use std::collections::HashMap;

use raisin_models::nodes::properties::PropertyValue;
use raisin_models::permissions::{Operation, Permission, RoleCondition};

// === Helper functions for PropertyValue ===

pub(crate) fn as_string(value: &PropertyValue) -> Option<&str> {
    match value {
        PropertyValue::String(s) => Some(s.as_str()),
        _ => None,
    }
}

pub(crate) fn as_array(value: &PropertyValue) -> Option<&Vec<PropertyValue>> {
    match value {
        PropertyValue::Array(arr) => Some(arr),
        _ => None,
    }
}

pub(crate) fn as_object(value: &PropertyValue) -> Option<&HashMap<String, PropertyValue>> {
    match value {
        PropertyValue::Object(obj) => Some(obj),
        _ => None,
    }
}

pub(crate) fn extract_string_array(
    properties: &HashMap<String, PropertyValue>,
    key: &str,
) -> Vec<String> {
    properties
        .get(key)
        .and_then(as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(as_string)
                .map(|s| s.to_string())
                .collect()
        })
        .unwrap_or_default()
}

// === Permission parsing ===

pub(crate) fn parse_permission(value: &PropertyValue) -> Option<Permission> {
    let obj = as_object(value)?;

    let path = obj.get("path").and_then(as_string)?.to_string();

    let operations: Vec<Operation> = obj
        .get("operations")
        .and_then(as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(as_string)
                .filter_map(Operation::parse)
                .collect()
        })
        .unwrap_or_default();

    // Build permission using constructor and builder methods
    let mut permission = Permission::new(path, operations);

    // Parse optional workspace pattern
    if let Some(workspace) = obj.get("workspace").and_then(as_string) {
        permission = permission.with_workspace(workspace);
    }

    // Parse optional branch pattern
    if let Some(branch_pattern) = obj.get("branch_pattern").and_then(as_string) {
        permission = permission.with_branch_pattern(branch_pattern);
    }

    // Parse optional node_types
    if let Some(node_types) = obj.get("node_types").and_then(as_array).map(|arr| {
        arr.iter()
            .filter_map(as_string)
            .map(|s| s.to_string())
            .collect()
    }) {
        permission = permission.with_node_types(node_types);
    }

    // Parse optional fields whitelist
    if let Some(fields) = obj.get("fields").and_then(as_array).map(|arr| {
        arr.iter()
            .filter_map(as_string)
            .map(|s| s.to_string())
            .collect()
    }) {
        permission = permission.with_fields(fields);
    }

    // Parse optional fields blacklist
    if let Some(except_fields) = obj.get("except_fields").and_then(as_array).map(|arr| {
        arr.iter()
            .filter_map(as_string)
            .map(|s| s.to_string())
            .collect()
    }) {
        permission = permission.with_except_fields(except_fields);
    }

    // Parse optional REL condition
    if let Some(condition) = obj.get("condition").and_then(as_string) {
        permission = permission.with_condition(condition.to_string());
    }

    // `conditions` is an accepted alias for `condition`.
    //
    // The built-in `author` role shipped its ownership rule under `conditions`
    // (a map, `{owner: "$user.id"}`), which nothing ever read: the permission
    // was stored with no condition at all, so an author could update and delete
    // ANY node rather than only their own. Roles already installed in existing
    // databases still carry that spelling, so the alias is what makes them
    // start behaving correctly without a reinstall.
    //
    // Translation is fail-CLOSED: anything here that cannot be understood
    // becomes the literal `false`, never "no condition".
    if permission.condition.is_none() {
        if let Some(expr) = obj.get("conditions").map(conditions_to_rel) {
            permission = permission.with_condition(expr);
        }
    }

    Some(permission)
}

/// Translate the `conditions` spelling of a permission condition into the REL
/// expression that [`Permission::condition`] holds.
///
/// Accepted shapes:
/// - a string: already a REL expression, taken verbatim
/// - a map: `{key: value, ...}`, ANDed together in sorted key order
/// - an array of maps/strings: ANDed together
///
/// Anything else — and any value this function cannot render — yields `"false"`,
/// because a condition that silently disappears is a permission that silently
/// widens.
pub(crate) fn conditions_to_rel(value: &PropertyValue) -> String {
    const DENY: &str = "false";

    match value {
        PropertyValue::String(s) => s.clone(),
        PropertyValue::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort(); // deterministic: the expression is compared and logged
            let mut parts = Vec::with_capacity(keys.len());
            for key in keys {
                match condition_pair_to_rel(key, &map[key]) {
                    Some(part) => parts.push(part),
                    None => return DENY.to_string(),
                }
            }
            if parts.is_empty() {
                DENY.to_string()
            } else {
                parts.join(" && ")
            }
        }
        PropertyValue::Array(items) => {
            if items.is_empty() {
                return DENY.to_string();
            }
            let parts: Vec<String> = items
                .iter()
                .map(|item| format!("({})", conditions_to_rel(item)))
                .collect();
            parts.join(" && ")
        }
        _ => DENY.to_string(),
    }
}

/// Render one `key: value` entry of a `conditions` map as a REL predicate.
///
/// Returns `None` when the value cannot be rendered, which the caller turns
/// into a deny.
fn condition_pair_to_rel(key: &str, value: &PropertyValue) -> Option<String> {
    let rendered = render_condition_operand(value)?;

    // `owner` is not a node property: it is the ownership test. A node's owner
    // is `owner_id` when set and its author otherwise. The leading null guard
    // is load-bearing — REL's `==` says `null == null`, so without it an
    // unauthenticated caller (whose `auth.user_id` is null) would compare equal
    // to every node with an unstamped `created_by` and own the lot.
    if key == "owner" {
        return Some(format!(
            "{rendered} != null && (node.owner_id == {rendered} || (node.owner_id == null && node.created_by == {rendered}))"
        ));
    }

    if !is_plain_identifier(key) {
        return None;
    }

    Some(format!("node.{key} == {rendered}"))
}

/// Render the right-hand side of a condition: an auth variable reference or a
/// literal.
fn render_condition_operand(value: &PropertyValue) -> Option<String> {
    match value {
        PropertyValue::String(s) if s.starts_with('$') => match s.as_str() {
            "$user.id" | "$user.user_id" | "$auth.user_id" => Some("auth.user_id".to_string()),
            "$user.local_id" | "$user.local_user_id" | "$auth.local_user_id" => {
                Some("auth.local_user_id".to_string())
            }
            "$user.email" | "$auth.email" => Some("auth.email".to_string()),
            "$user.home" | "$auth.home" => Some("auth.home".to_string()),
            // An unrecognised variable must not degrade into a literal string:
            // that would compare a property against "$user.whatever" and either
            // never match or, worse, match a value an attacker can write.
            _ => None,
        },
        PropertyValue::String(s) => Some(quote_rel_string(s)),
        PropertyValue::Boolean(b) => Some(b.to_string()),
        PropertyValue::Integer(i) => Some(i.to_string()),
        PropertyValue::Float(f) => Some(f.to_string()),
        PropertyValue::Null => Some("null".to_string()),
        _ => None,
    }
}

fn is_plain_identifier(key: &str) -> bool {
    !key.is_empty()
        && key.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
}

fn quote_rel_string(s: &str) -> String {
    let escaped = s.replace('\\', "\\\\").replace('\'', "\\'");
    format!("'{escaped}'")
}

pub(crate) fn parse_conditions(value: &PropertyValue) -> Option<Vec<RoleCondition>> {
    if let Some(arr) = as_array(value) {
        let conditions: Vec<RoleCondition> =
            arr.iter().filter_map(parse_single_condition).collect();
        if conditions.is_empty() {
            None
        } else {
            Some(conditions)
        }
    } else {
        parse_single_condition(value).map(|c| vec![c])
    }
}

fn parse_single_condition(value: &PropertyValue) -> Option<RoleCondition> {
    use raisin_models::permissions::{ConditionValue, PropertyCondition, PropertyInCondition};

    let obj = as_object(value)?;

    if let Some(pe) = obj.get("property_equals").and_then(as_object) {
        let key = pe.get("key").and_then(as_string)?.to_string();
        let value = parse_condition_value(pe.get("value")?)?;
        return Some(RoleCondition::PropertyEquals(PropertyCondition {
            key,
            value,
        }));
    }

    if let Some(pi) = obj.get("property_in").and_then(as_object) {
        let key = pi.get("key").and_then(as_string)?.to_string();
        let values: Vec<ConditionValue> = pi
            .get("values")
            .and_then(as_array)
            .map(|arr| arr.iter().filter_map(parse_condition_value).collect())
            .unwrap_or_default();
        return Some(RoleCondition::PropertyIn(PropertyInCondition {
            key,
            values,
        }));
    }

    if let Some(pg) = obj.get("property_greater_than").and_then(as_object) {
        let key = pg.get("key").and_then(as_string)?.to_string();
        let value = parse_condition_value(pg.get("value")?)?;
        return Some(RoleCondition::PropertyGreaterThan(PropertyCondition {
            key,
            value,
        }));
    }

    if let Some(pl) = obj.get("property_less_than").and_then(as_object) {
        let key = pl.get("key").and_then(as_string)?.to_string();
        let value = parse_condition_value(pl.get("value")?)?;
        return Some(RoleCondition::PropertyLessThan(PropertyCondition {
            key,
            value,
        }));
    }

    if let Some(role) = obj.get("user_has_role").and_then(as_string) {
        return Some(RoleCondition::UserHasRole(role.to_string()));
    }

    if let Some(group) = obj.get("user_in_group").and_then(as_string) {
        return Some(RoleCondition::UserInGroup(group.to_string()));
    }

    if let Some(all) = obj.get("all").and_then(as_array) {
        let conditions: Vec<RoleCondition> =
            all.iter().filter_map(parse_single_condition).collect();
        return Some(RoleCondition::All(conditions));
    }

    if let Some(any) = obj.get("any").and_then(as_array) {
        let conditions: Vec<RoleCondition> =
            any.iter().filter_map(parse_single_condition).collect();
        return Some(RoleCondition::Any(conditions));
    }

    None
}

fn parse_condition_value(
    value: &PropertyValue,
) -> Option<raisin_models::permissions::ConditionValue> {
    use raisin_models::permissions::ConditionValue;

    if let Some(s) = as_string(value) {
        if s.starts_with("$auth.") {
            return Some(ConditionValue::AuthVariable(s.to_string()));
        }
    }

    Some(ConditionValue::Literal(Box::new(value.clone())))
}

#[cfg(test)]
mod conditions_alias_tests {
    use super::*;
    use raisin_models::auth::AuthContext;
    use raisin_models::nodes::Node;

    /// The exact shape the built-in `author` role shipped with, and the shape
    /// still sitting in every database installed before the fix.
    fn author_update_permission() -> PropertyValue {
        let mut conditions = HashMap::new();
        conditions.insert(
            "owner".to_string(),
            PropertyValue::String("$user.id".to_string()),
        );

        let mut perm = HashMap::new();
        perm.insert("path".to_string(), PropertyValue::String("**".to_string()));
        perm.insert(
            "operations".to_string(),
            PropertyValue::Array(vec![
                PropertyValue::String("update".to_string()),
                PropertyValue::String("delete".to_string()),
            ]),
        );
        perm.insert("conditions".to_string(), PropertyValue::Object(conditions));
        PropertyValue::Object(perm)
    }

    fn node_created_by(author: Option<&str>) -> Node {
        Node {
            id: "node-1".to_string(),
            name: "one".to_string(),
            path: "/posts/one".to_string(),
            node_type: "blog:Post".to_string(),
            properties: HashMap::new(),
            created_by: author.map(|a| a.to_string()),
            ..Default::default()
        }
    }

    fn evaluate(expr: &str, node: &Node, user: Option<&str>) -> bool {
        let auth = match user {
            Some(u) => AuthContext::for_user(u),
            None => AuthContext::anonymous(),
        };
        crate::services::rls_filter::context::evaluate_rel_condition(expr, node, &auth)
    }

    /// The bug: `conditions` was never read, so the permission carried no
    /// condition and an author could update and delete anyone's node.
    #[test]
    fn the_conditions_spelling_is_no_longer_dropped() {
        let permission = parse_permission(&author_update_permission()).expect("permission parses");
        assert!(
            permission.condition.is_some(),
            "the `conditions` key must not be silently dropped - that is the vulnerability"
        );
    }

    #[test]
    fn an_author_may_modify_only_their_own_node() {
        let permission = parse_permission(&author_update_permission()).unwrap();
        let expr = permission.condition.as_deref().unwrap();

        let mine = node_created_by(Some("user-a"));
        let theirs = node_created_by(Some("user-b"));

        assert!(evaluate(expr, &mine, Some("user-a")), "own node: allowed");
        assert!(
            !evaluate(expr, &theirs, Some("user-a")),
            "another user's node must be refused"
        );
    }

    /// `null == null` is true in REL, so an unstamped `created_by` plus an
    /// unauthenticated caller must not read as ownership.
    #[test]
    fn nobody_owns_an_unattributed_node() {
        let permission = parse_permission(&author_update_permission()).unwrap();
        let expr = permission.condition.as_deref().unwrap();

        let orphan = node_created_by(None);
        assert!(!evaluate(expr, &orphan, None));
        assert!(!evaluate(expr, &orphan, Some("user-a")));
    }

    /// `owner_id`, when set, is the owner - the author of an owned node is not.
    #[test]
    fn an_explicit_owner_id_wins_over_the_author() {
        let permission = parse_permission(&author_update_permission()).unwrap();
        let expr = permission.condition.as_deref().unwrap();

        let mut node = node_created_by(Some("user-a"));
        node.owner_id = Some("user-b".to_string());

        assert!(!evaluate(expr, &node, Some("user-a")));
        assert!(evaluate(expr, &node, Some("user-b")));
    }

    /// `condition` (singular) still wins; the alias only fills a gap.
    #[test]
    fn the_singular_spelling_is_not_overridden() {
        let mut perm = HashMap::new();
        perm.insert("path".to_string(), PropertyValue::String("**".to_string()));
        perm.insert(
            "operations".to_string(),
            PropertyValue::Array(vec![PropertyValue::String("read".to_string())]),
        );
        perm.insert(
            "condition".to_string(),
            PropertyValue::String("node.status == 'published'".to_string()),
        );
        perm.insert(
            "conditions".to_string(),
            PropertyValue::String("node.status == 'draft'".to_string()),
        );

        let permission = parse_permission(&PropertyValue::Object(perm)).unwrap();
        assert_eq!(
            permission.condition.as_deref(),
            Some("node.status == 'published'")
        );
    }

    #[test]
    fn a_plain_property_condition_becomes_a_node_comparison() {
        let mut map = HashMap::new();
        map.insert(
            "status".to_string(),
            PropertyValue::String("published".to_string()),
        );
        assert_eq!(
            conditions_to_rel(&PropertyValue::Object(map)),
            "node.status == 'published'"
        );
    }

    /// Fail CLOSED. An unrecognised value must never degrade into "no
    /// condition", which is what would widen the permission.
    #[test]
    fn an_untranslatable_condition_denies() {
        let mut map = HashMap::new();
        map.insert(
            "owner".to_string(),
            PropertyValue::String("$user.nonsense".to_string()),
        );
        assert_eq!(conditions_to_rel(&PropertyValue::Object(map)), "false");

        assert_eq!(
            conditions_to_rel(&PropertyValue::Object(HashMap::new())),
            "false"
        );
        assert_eq!(conditions_to_rel(&PropertyValue::Integer(3)), "false");

        let node = node_created_by(Some("user-a"));
        assert!(!evaluate("false", &node, Some("user-a")));
    }

    /// The shipped YAML and the alias must agree, or an upgraded database and a
    /// fresh one would enforce different rules under the same role name.
    #[test]
    fn the_shipped_role_matches_what_the_alias_produces() {
        let expected = "auth.user_id != null && (node.owner_id == auth.user_id || (node.owner_id == null && node.created_by == auth.user_id))";
        let permission = parse_permission(&author_update_permission()).unwrap();
        assert_eq!(permission.condition.as_deref(), Some(expected));

        let yaml = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../builtin-packages/raisin-auth/content/_raisin__access_control/roles/author/.node.yaml"
        ));
        assert!(
            yaml.contains(expected),
            "builtin author role must ship exactly the alias expression"
        );
    }
}
