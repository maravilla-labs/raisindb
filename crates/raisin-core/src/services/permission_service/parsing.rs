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

    Some(permission)
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
