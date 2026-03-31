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

//! Condition types for permission evaluation.

use serde::{Deserialize, Serialize};

use crate::nodes::properties::PropertyValue;

/// A value used in permission conditions.
///
/// Can be either a literal value or a reference to an auth context variable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ConditionValue {
    /// A literal property value (string, number, etc.)
    Literal(Box<PropertyValue>),
    /// A reference to an auth context variable (e.g., "$auth.user_id", "$auth.email")
    AuthVariable(String),
}

impl ConditionValue {
    /// Check if this is an auth variable reference
    pub fn is_auth_variable(&self) -> bool {
        matches!(self, ConditionValue::AuthVariable(s) if s.starts_with("$auth."))
    }

    /// Create a literal string value
    pub fn string(s: impl Into<String>) -> Self {
        ConditionValue::Literal(Box::new(PropertyValue::String(s.into())))
    }

    /// Create an auth variable reference
    pub fn auth_var(var: impl Into<String>) -> Self {
        ConditionValue::AuthVariable(var.into())
    }
}

/// Conditions that must be met for a permission to apply.
///
/// These conditions are evaluated at runtime against the node being accessed
/// and the current auth context.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoleCondition {
    /// Property must equal a specific value
    /// Example: `author = $auth.user_id` (ownership check)
    PropertyEquals(PropertyCondition),

    /// Property must be in a list of values
    /// Example: `status IN ['draft', 'review']`
    PropertyIn(PropertyInCondition),

    /// Property must be greater than a value
    /// Example: `priority > 5`
    PropertyGreaterThan(PropertyCondition),

    /// Property must be less than a value
    /// Example: `priority < 10`
    PropertyLessThan(PropertyCondition),

    /// User must have a specific role
    /// Example: `user_has_role: 'editor'`
    UserHasRole(String),

    /// User must be in a specific group
    /// Example: `user_in_group: 'engineering'`
    UserInGroup(String),

    /// All sub-conditions must be true (AND)
    All(Vec<RoleCondition>),

    /// Any sub-condition must be true (OR)
    Any(Vec<RoleCondition>),
}

/// A property-based condition with key and value
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PropertyCondition {
    /// The property key to check
    pub key: String,
    /// The value to compare against
    pub value: ConditionValue,
}

/// A property-in-list condition
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PropertyInCondition {
    /// The property key to check
    pub key: String,
    /// The list of values to check against
    pub values: Vec<ConditionValue>,
}

impl RoleCondition {
    /// Create an ownership condition (author = current user)
    pub fn ownership(property: impl Into<String>) -> Self {
        RoleCondition::PropertyEquals(PropertyCondition {
            key: property.into(),
            value: ConditionValue::auth_var("$auth.user_id"),
        })
    }

    /// Create a property equals condition
    pub fn property_equals(key: impl Into<String>, value: ConditionValue) -> Self {
        RoleCondition::PropertyEquals(PropertyCondition {
            key: key.into(),
            value,
        })
    }

    /// Create a property in list condition
    pub fn property_in(key: impl Into<String>, values: Vec<ConditionValue>) -> Self {
        RoleCondition::PropertyIn(PropertyInCondition {
            key: key.into(),
            values,
        })
    }

    /// Create an AND condition
    pub fn all(conditions: Vec<RoleCondition>) -> Self {
        RoleCondition::All(conditions)
    }

    /// Create an OR condition
    pub fn any(conditions: Vec<RoleCondition>) -> Self {
        RoleCondition::Any(conditions)
    }

    /// Create a user has role condition
    pub fn user_has_role(role: impl Into<String>) -> Self {
        RoleCondition::UserHasRole(role.into())
    }

    /// Create a user in group condition
    pub fn user_in_group(group: impl Into<String>) -> Self {
        RoleCondition::UserInGroup(group.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ownership_condition() {
        let condition = RoleCondition::ownership("author");
        match condition {
            RoleCondition::PropertyEquals(pc) => {
                assert_eq!(pc.key, "author");
                assert!(pc.value.is_auth_variable());
            }
            _ => panic!("Expected PropertyEquals"),
        }
    }

    #[test]
    fn test_composite_conditions() {
        let condition = RoleCondition::all(vec![
            RoleCondition::user_has_role("editor"),
            RoleCondition::property_equals("status", ConditionValue::string("draft")),
        ]);

        match condition {
            RoleCondition::All(conditions) => {
                assert_eq!(conditions.len(), 2);
            }
            _ => panic!("Expected All"),
        }
    }
}
