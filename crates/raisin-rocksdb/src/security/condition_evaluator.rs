//! Condition evaluation for permission rules.
//!
//! Evaluates REL (Raisin Expression Language) conditions like:
//! - `node.created_by == auth.user_id` - User can only see their own posts
//! - `auth.roles.contains('editor')` - User must have the editor role
//! - `node.status == 'published' || node.created_by == auth.user_id` - OR conditions
//! - `node.status == 'draft' && node.created_by == auth.user_id` - AND conditions
//! - `RELATES(node.id, target.id, ['owns'], 1, 3, 'outgoing')` - Graph relationship checks

use super::rel_context::build_rel_context;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_models::permissions::{ConditionValue, PropertyCondition, RoleCondition};
use raisin_rel::eval::RelationResolver;

/// Condition evaluator for permission checks.
pub struct ConditionEvaluator<'a> {
    auth: &'a AuthContext,
}

impl<'a> ConditionEvaluator<'a> {
    /// Create a new condition evaluator.
    pub fn new(auth: &'a AuthContext) -> Self {
        Self { auth }
    }

    /// Evaluate a single condition against a node.
    ///
    /// Returns true if the condition is satisfied.
    pub fn evaluate(&self, condition: &RoleCondition, node: &Node) -> bool {
        match condition {
            RoleCondition::PropertyEquals(prop_cond) => {
                self.evaluate_property_equals(prop_cond, node)
            }
            RoleCondition::PropertyIn(prop_in) => self.evaluate_property_in(prop_in, node),
            RoleCondition::PropertyGreaterThan(prop_cond) => {
                self.evaluate_property_greater_than(prop_cond, node)
            }
            RoleCondition::PropertyLessThan(prop_cond) => {
                self.evaluate_property_less_than(prop_cond, node)
            }
            RoleCondition::UserHasRole(role) => self.auth.has_role(role),
            RoleCondition::UserInGroup(group) => self.auth.in_group(group),
            RoleCondition::All(conditions) => conditions.iter().all(|c| self.evaluate(c, node)),
            RoleCondition::Any(conditions) => conditions.iter().any(|c| self.evaluate(c, node)),
        }
    }

    /// Evaluate all conditions for a permission (AND logic).
    ///
    /// Returns true if all conditions are satisfied.
    pub fn evaluate_all(&self, conditions: &[RoleCondition], node: &Node) -> bool {
        conditions.iter().all(|c| self.evaluate(c, node))
    }

    /// Evaluate a REL (Raisin Expression Language) expression against a node.
    ///
    /// This evaluates conditions like:
    /// - `node.created_by == auth.user_id`
    /// - `node.status == 'published' || auth.roles.contains('editor')`
    ///
    /// Returns true if the expression evaluates to a truthy value.
    /// Returns false on parse/eval errors (fail-closed security).
    pub fn evaluate_rel_expression(&self, expr: &str, node: &Node) -> bool {
        let ctx = build_rel_context(self.auth, node);
        match raisin_rel::eval(expr, &ctx) {
            Ok(value) => value.is_truthy(),
            Err(e) => {
                tracing::warn!(
                    expr = %expr,
                    error = %e,
                    "REL condition evaluation failed"
                );
                false // Fail-closed: deny on error
            }
        }
    }

    /// Evaluate a REL expression that may require async operations (e.g., RELATES).
    ///
    /// This is the async version that supports graph relationship checks.
    /// Use this when the expression contains RELATES or other async operations.
    ///
    /// # Arguments
    /// * `expr` - REL expression string
    /// * `node` - Node being evaluated
    /// * `graph_resolver` - Optional graph resolver for RELATES expressions
    ///
    /// Returns true if the expression evaluates to a truthy value.
    /// Returns false on parse/eval errors (fail-closed security).
    pub async fn evaluate_rel_expression_async(
        &self,
        expr: &str,
        node: &Node,
        _graph_resolver: Option<&dyn RelationResolver>,
    ) -> bool {
        // For now, fall back to sync evaluation
        // TODO: Implement async evaluation with RELATES support when REL parser
        // supports RELATES expressions
        self.evaluate_rel_expression(expr, node)
    }

    /// Check if an expression requires async evaluation.
    ///
    /// Returns true if the expression contains RELATES or other async operations.
    pub fn requires_async(expr_str: &str) -> bool {
        // Parse the expression first, then check if it requires async
        match raisin_rel::parse(expr_str) {
            Ok(expr) => raisin_rel::eval::requires_async(&expr),
            Err(_) => false, // If parsing fails, doesn't require async (will fail in eval)
        }
    }

    /// Resolve a condition value, substituting auth variables.
    fn resolve_value(&self, value: &ConditionValue) -> Option<PropertyValue> {
        match value {
            ConditionValue::Literal(pv) => Some(pv.as_ref().clone()),
            ConditionValue::AuthVariable(var) => self.resolve_auth_variable(var),
        }
    }

    /// Resolve an auth variable like "$auth.user_id", "$auth.local_user_id", or "$auth.email".
    fn resolve_auth_variable(&self, var: &str) -> Option<PropertyValue> {
        match var {
            "$auth.user_id" => self
                .auth
                .user_id
                .as_ref()
                .map(|s| PropertyValue::String(s.clone())),
            "$auth.local_user_id" => self
                .auth
                .local_user_id
                .as_ref()
                .map(|s| PropertyValue::String(s.clone())),
            "$auth.email" => self
                .auth
                .email
                .as_ref()
                .map(|s| PropertyValue::String(s.clone())),
            _ => {
                // Unknown variable - treat as literal string for now
                tracing::warn!("Unknown auth variable: {}", var);
                Some(PropertyValue::String(var.to_string()))
            }
        }
    }

    fn evaluate_property_equals(&self, cond: &PropertyCondition, node: &Node) -> bool {
        let actual = node.properties.get(&cond.key);
        let expected = self.resolve_value(&cond.value);

        match (actual, expected) {
            (Some(a), Some(e)) => property_value_equals(a, &e),
            (None, None) => true, // Both missing = equal
            _ => false,
        }
    }

    fn evaluate_property_in(
        &self,
        cond: &raisin_models::permissions::PropertyInCondition,
        node: &Node,
    ) -> bool {
        let actual = match node.properties.get(&cond.key) {
            Some(v) => v,
            None => return false,
        };

        cond.values
            .iter()
            .filter_map(|v| self.resolve_value(v))
            .any(|expected| property_value_equals(actual, &expected))
    }

    fn evaluate_property_greater_than(&self, cond: &PropertyCondition, node: &Node) -> bool {
        let actual = node.properties.get(&cond.key);
        let expected = self.resolve_value(&cond.value);

        match (actual, expected) {
            (Some(a), Some(e)) => {
                property_value_compare(a, &e) == Some(std::cmp::Ordering::Greater)
            }
            _ => false,
        }
    }

    fn evaluate_property_less_than(&self, cond: &PropertyCondition, node: &Node) -> bool {
        let actual = node.properties.get(&cond.key);
        let expected = self.resolve_value(&cond.value);

        match (actual, expected) {
            (Some(a), Some(e)) => property_value_compare(a, &e) == Some(std::cmp::Ordering::Less),
            _ => false,
        }
    }
}

/// Compare two PropertyValues for equality.
fn property_value_equals(a: &PropertyValue, b: &PropertyValue) -> bool {
    match (a, b) {
        (PropertyValue::String(s1), PropertyValue::String(s2)) => s1 == s2,
        (PropertyValue::Float(f1), PropertyValue::Float(f2)) => {
            // Compare as f64 with epsilon for floating point
            (*f1 - *f2).abs() < f64::EPSILON
        }
        (PropertyValue::Integer(i1), PropertyValue::Integer(i2)) => i1 == i2,
        (PropertyValue::Integer(i), PropertyValue::Float(f))
        | (PropertyValue::Float(f), PropertyValue::Integer(i)) => {
            (*i as f64 - *f).abs() < f64::EPSILON
        }
        (PropertyValue::Boolean(b1), PropertyValue::Boolean(b2)) => b1 == b2,
        (PropertyValue::Null, PropertyValue::Null) => true,
        (PropertyValue::Array(a1), PropertyValue::Array(a2)) => {
            a1.len() == a2.len()
                && a1
                    .iter()
                    .zip(a2.iter())
                    .all(|(x, y)| property_value_equals(x, y))
        }
        _ => false,
    }
}

/// Compare two PropertyValues for ordering.
fn property_value_compare(a: &PropertyValue, b: &PropertyValue) -> Option<std::cmp::Ordering> {
    match (a, b) {
        (PropertyValue::String(s1), PropertyValue::String(s2)) => Some(s1.cmp(s2)),
        (PropertyValue::Float(f1), PropertyValue::Float(f2)) => f1.partial_cmp(f2),
        (PropertyValue::Integer(i1), PropertyValue::Integer(i2)) => Some(i1.cmp(i2)),
        (PropertyValue::Integer(i), PropertyValue::Float(f)) => (*i as f64).partial_cmp(f),
        (PropertyValue::Float(f), PropertyValue::Integer(i)) => f.partial_cmp(&(*i as f64)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn make_node(props: Vec<(&str, PropertyValue)>) -> Node {
        let mut properties = HashMap::new();
        for (k, v) in props {
            properties.insert(k.to_string(), v);
        }
        Node {
            id: "test".to_string(),
            name: "test".to_string(),
            path: "/test".to_string(),
            node_type: "test:Type".to_string(),
            properties,
            ..Default::default()
        }
    }

    #[test]
    fn test_property_equals_literal() {
        let auth = AuthContext::for_user("user1");
        let evaluator = ConditionEvaluator::new(&auth);

        let node = make_node(vec![(
            "status",
            PropertyValue::String("published".to_string()),
        )]);

        let condition = RoleCondition::PropertyEquals(PropertyCondition {
            key: "status".to_string(),
            value: ConditionValue::Literal(Box::new(PropertyValue::String(
                "published".to_string(),
            ))),
        });

        assert!(evaluator.evaluate(&condition, &node));
    }

    #[test]
    fn test_property_equals_auth_variable() {
        let auth = AuthContext::for_user("user123");
        let evaluator = ConditionEvaluator::new(&auth);

        let node = make_node(vec![(
            "author_id",
            PropertyValue::String("user123".to_string()),
        )]);

        let condition = RoleCondition::PropertyEquals(PropertyCondition {
            key: "author_id".to_string(),
            value: ConditionValue::AuthVariable("$auth.user_id".to_string()),
        });

        assert!(evaluator.evaluate(&condition, &node));
    }

    #[test]
    fn test_user_has_role() {
        let auth = AuthContext::for_user("user1").with_roles(vec!["editor".to_string()]);
        let evaluator = ConditionEvaluator::new(&auth);

        let node = make_node(vec![]);

        assert!(evaluator.evaluate(&RoleCondition::UserHasRole("editor".to_string()), &node));
        assert!(!evaluator.evaluate(&RoleCondition::UserHasRole("admin".to_string()), &node));
    }

    #[test]
    fn test_all_conditions() {
        let auth = AuthContext::for_user("user1")
            .with_roles(vec!["editor".to_string()])
            .with_groups(vec!["team-a".to_string()]);
        let evaluator = ConditionEvaluator::new(&auth);

        let node = make_node(vec![]);

        let all_condition = RoleCondition::All(vec![
            RoleCondition::UserHasRole("editor".to_string()),
            RoleCondition::UserInGroup("team-a".to_string()),
        ]);

        assert!(evaluator.evaluate(&all_condition, &node));

        let failing_all = RoleCondition::All(vec![
            RoleCondition::UserHasRole("editor".to_string()),
            RoleCondition::UserHasRole("admin".to_string()), // User doesn't have this
        ]);

        assert!(!evaluator.evaluate(&failing_all, &node));
    }

    #[test]
    fn test_any_conditions() {
        let auth = AuthContext::for_user("user1").with_roles(vec!["editor".to_string()]);
        let evaluator = ConditionEvaluator::new(&auth);

        let node = make_node(vec![]);

        let any_condition = RoleCondition::Any(vec![
            RoleCondition::UserHasRole("admin".to_string()), // User doesn't have this
            RoleCondition::UserHasRole("editor".to_string()), // User has this
        ]);

        assert!(evaluator.evaluate(&any_condition, &node));
    }

    // REL expression tests
    #[test]
    fn test_rel_ownership_check() {
        let auth = AuthContext::for_user("user123");
        let evaluator = ConditionEvaluator::new(&auth);

        let mut node = make_node(vec![]);
        node.created_by = Some("user123".to_string());

        assert!(evaluator.evaluate_rel_expression("node.created_by == auth.user_id", &node));
    }

    #[test]
    fn test_rel_ownership_check_mismatch() {
        let auth = AuthContext::for_user("different-user");
        let evaluator = ConditionEvaluator::new(&auth);

        let mut node = make_node(vec![]);
        node.created_by = Some("user123".to_string());

        assert!(!evaluator.evaluate_rel_expression("node.created_by == auth.user_id", &node));
    }

    #[test]
    fn test_rel_property_condition() {
        let auth = AuthContext::for_user("user1");
        let evaluator = ConditionEvaluator::new(&auth);

        let node = make_node(vec![(
            "status",
            PropertyValue::String("published".to_string()),
        )]);

        assert!(evaluator.evaluate_rel_expression("node.status == 'published'", &node));
        assert!(!evaluator.evaluate_rel_expression("node.status == 'draft'", &node));
    }

    #[test]
    fn test_rel_combined_conditions() {
        let auth = AuthContext::for_user("user123");
        let evaluator = ConditionEvaluator::new(&auth);

        let mut node = make_node(vec![("status", PropertyValue::String("draft".to_string()))]);
        node.created_by = Some("user123".to_string());

        // Owner can see their draft
        assert!(evaluator.evaluate_rel_expression(
            "node.status == 'draft' && node.created_by == auth.user_id",
            &node
        ));

        // Non-owner cannot see draft
        let other_auth = AuthContext::for_user("other-user");
        let other_evaluator = ConditionEvaluator::new(&other_auth);
        assert!(!other_evaluator.evaluate_rel_expression(
            "node.status == 'draft' && node.created_by == auth.user_id",
            &node
        ));
    }

    #[test]
    fn test_rel_or_conditions() {
        let auth = AuthContext::for_user("editor-user").with_roles(vec!["editor".to_string()]);
        let evaluator = ConditionEvaluator::new(&auth);

        let mut node = make_node(vec![]);
        node.created_by = Some("different-user".to_string());

        // Not owner but has editor role
        assert!(evaluator.evaluate_rel_expression(
            "node.created_by == auth.user_id || auth.roles.contains('editor')",
            &node
        ));
    }

    #[test]
    fn test_rel_invalid_expression() {
        let auth = AuthContext::for_user("user1");
        let evaluator = ConditionEvaluator::new(&auth);

        let node = make_node(vec![]);

        // Invalid expression should return false (fail-closed)
        assert!(!evaluator.evaluate_rel_expression("invalid syntax !!!", &node));
    }
}
