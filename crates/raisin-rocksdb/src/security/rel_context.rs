//! REL context builder for permission evaluation.
//!
//! Builds an `EvalContext` from `AuthContext` and `Node` for evaluating
//! REL (Raisin Expression Language) conditions in permissions.
//!
//! # Variables
//!
//! ## `auth.*` variables
//! - `auth.user_id` - Global identity (JWT sub claim)
//! - `auth.local_user_id` - Workspace-specific raisin:User node ID
//! - `auth.email` - User's email
//! - `auth.home` - User's home path (raisin:User node path)
//! - `auth.is_anonymous` - Whether user is unauthenticated
//! - `auth.is_system` - Whether this is a system operation
//! - `auth.roles` - Array of role IDs
//! - `auth.groups` - Array of group IDs
//!
//! ## `node.*` variables
//! - `node.id` - Node ID
//! - `node.name` - Node name
//! - `node.path` - Node path
//! - `node.node_type` - Node type
//! - `node.created_by` - User who created the node
//! - `node.updated_by` - User who last updated the node
//! - `node.owner_id` - Owner user ID
//! - `node.workspace` - Workspace name
//! - `node.<property>` - Any property from node.properties

use raisin_models::auth::AuthContext;
use raisin_models::nodes::Node;
use raisin_rel::{EvalContext, Value};
use std::collections::HashMap;

/// Build a REL EvalContext from AuthContext and Node.
///
/// This creates a context with `auth.*` and `node.*` variables
/// that can be used to evaluate permission conditions.
pub fn build_rel_context(auth: &AuthContext, node: &Node) -> EvalContext {
    let mut ctx = EvalContext::new();

    // Build auth object
    let mut auth_obj = HashMap::new();

    // auth.user_id (global identity)
    if let Some(user_id) = &auth.user_id {
        auth_obj.insert("user_id".to_string(), Value::String(user_id.clone()));
    } else {
        auth_obj.insert("user_id".to_string(), Value::Null);
    }

    // auth.local_user_id (workspace-specific)
    if let Some(local_user_id) = &auth.local_user_id {
        auth_obj.insert(
            "local_user_id".to_string(),
            Value::String(local_user_id.clone()),
        );
    } else {
        auth_obj.insert("local_user_id".to_string(), Value::Null);
    }

    // auth.email
    if let Some(email) = &auth.email {
        auth_obj.insert("email".to_string(), Value::String(email.clone()));
    } else {
        auth_obj.insert("email".to_string(), Value::Null);
    }

    // auth.is_anonymous
    auth_obj.insert(
        "is_anonymous".to_string(),
        Value::Boolean(auth.is_anonymous),
    );

    // auth.is_system
    auth_obj.insert("is_system".to_string(), Value::Boolean(auth.is_system));

    // auth.roles (array)
    auth_obj.insert(
        "roles".to_string(),
        Value::Array(
            auth.roles
                .iter()
                .map(|r| Value::String(r.clone()))
                .collect(),
        ),
    );

    // auth.groups (array)
    auth_obj.insert(
        "groups".to_string(),
        Value::Array(
            auth.groups
                .iter()
                .map(|g| Value::String(g.clone()))
                .collect(),
        ),
    );

    // auth.home (user's home path)
    if let Some(home) = &auth.home {
        auth_obj.insert("home".to_string(), Value::String(home.clone()));
    } else {
        auth_obj.insert("home".to_string(), Value::Null);
    }

    ctx.set("auth", Value::Object(auth_obj));

    // Build node object
    let mut node_obj = HashMap::new();

    // node.id
    node_obj.insert("id".to_string(), Value::String(node.id.clone()));

    // node.name
    node_obj.insert("name".to_string(), Value::String(node.name.clone()));

    // node.path
    node_obj.insert("path".to_string(), Value::String(node.path.clone()));

    // node.node_type
    node_obj.insert(
        "node_type".to_string(),
        Value::String(node.node_type.clone()),
    );

    // node.created_by
    if let Some(created_by) = &node.created_by {
        node_obj.insert("created_by".to_string(), Value::String(created_by.clone()));
    } else {
        node_obj.insert("created_by".to_string(), Value::Null);
    }

    // node.updated_by
    if let Some(updated_by) = &node.updated_by {
        node_obj.insert("updated_by".to_string(), Value::String(updated_by.clone()));
    } else {
        node_obj.insert("updated_by".to_string(), Value::Null);
    }

    // node.owner_id
    if let Some(owner_id) = &node.owner_id {
        node_obj.insert("owner_id".to_string(), Value::String(owner_id.clone()));
    } else {
        node_obj.insert("owner_id".to_string(), Value::Null);
    }

    // node.workspace
    if let Some(workspace) = &node.workspace {
        node_obj.insert("workspace".to_string(), Value::String(workspace.clone()));
    } else {
        node_obj.insert("workspace".to_string(), Value::Null);
    }

    // Add all node properties as node.<property>
    for (key, value) in &node.properties {
        node_obj.insert(key.clone(), property_value_to_rel_value(value));
    }

    ctx.set("node", Value::Object(node_obj));

    ctx
}

/// Convert a PropertyValue to a REL Value.
fn property_value_to_rel_value(pv: &raisin_models::nodes::properties::PropertyValue) -> Value {
    use raisin_models::nodes::properties::PropertyValue;

    match pv {
        PropertyValue::Null => Value::Null,
        PropertyValue::Boolean(b) => Value::Boolean(*b),
        PropertyValue::Integer(i) => Value::Integer(*i),
        PropertyValue::Float(f) => Value::Float(*f),
        PropertyValue::String(s) => Value::String(s.clone()),
        PropertyValue::Array(arr) => {
            Value::Array(arr.iter().map(property_value_to_rel_value).collect())
        }
        PropertyValue::Object(obj) => {
            let map: HashMap<String, Value> = obj
                .iter()
                .map(|(k, v)| (k.clone(), property_value_to_rel_value(v)))
                .collect();
            Value::Object(map)
        }
        // Convert complex types to their string representation for REL comparison
        PropertyValue::Date(dt) => Value::String(dt.to_string()),
        PropertyValue::Decimal(d) => Value::String(d.to_string()),
        PropertyValue::Reference(r) => Value::String(r.id.clone()),
        PropertyValue::Url(u) => Value::String(u.url.clone()),
        PropertyValue::Resource(r) => Value::String(r.uuid.clone()),
        PropertyValue::Composite(_) => Value::Null, // Complex type - not comparable in REL
        PropertyValue::Element(_) => Value::Null,   // Complex type - not comparable in REL
        PropertyValue::Vector(v) => {
            Value::Array(v.iter().map(|f| Value::Float(*f as f64)).collect())
        }
        PropertyValue::Geometry(_) => Value::Null, // Complex type - not comparable in REL
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn make_test_node() -> Node {
        Node {
            id: "node-123".to_string(),
            name: "test-node".to_string(),
            path: "/content/test-node".to_string(),
            node_type: "blog:Article".to_string(),
            created_by: Some("user-456".to_string()),
            updated_by: Some("user-789".to_string()),
            owner_id: Some("user-456".to_string()),
            workspace: Some("main".to_string()),
            properties: {
                let mut props = HashMap::new();
                props.insert(
                    "status".to_string(),
                    raisin_models::nodes::properties::PropertyValue::String(
                        "published".to_string(),
                    ),
                );
                props.insert(
                    "priority".to_string(),
                    raisin_models::nodes::properties::PropertyValue::Integer(5),
                );
                props
            },
            ..Default::default()
        }
    }

    #[test]
    fn test_build_rel_context_auth_variables() {
        let auth = AuthContext::for_user("user-123")
            .with_email("test@example.com")
            .with_roles(vec!["editor".to_string(), "reviewer".to_string()])
            .with_groups(vec!["team-a".to_string()]);
        let node = make_test_node();

        let ctx = build_rel_context(&auth, &node);

        // Check auth variables
        let auth_val = ctx.get("auth").unwrap();
        assert_eq!(
            auth_val.get("user_id"),
            Some(&Value::String("user-123".to_string()))
        );
        assert_eq!(
            auth_val.get("email"),
            Some(&Value::String("test@example.com".to_string()))
        );
        assert_eq!(auth_val.get("is_anonymous"), Some(&Value::Boolean(false)));
        assert_eq!(auth_val.get("is_system"), Some(&Value::Boolean(false)));

        // Check roles array
        if let Some(Value::Array(roles)) = auth_val.get("roles") {
            assert_eq!(roles.len(), 2);
            assert!(roles.contains(&Value::String("editor".to_string())));
            assert!(roles.contains(&Value::String("reviewer".to_string())));
        } else {
            panic!("Expected roles to be an array");
        }
    }

    #[test]
    fn test_build_rel_context_node_variables() {
        let auth = AuthContext::for_user("user-123");
        let node = make_test_node();

        let ctx = build_rel_context(&auth, &node);

        // Check node variables
        let node_val = ctx.get("node").unwrap();
        assert_eq!(
            node_val.get("id"),
            Some(&Value::String("node-123".to_string()))
        );
        assert_eq!(
            node_val.get("created_by"),
            Some(&Value::String("user-456".to_string()))
        );
        assert_eq!(
            node_val.get("node_type"),
            Some(&Value::String("blog:Article".to_string()))
        );

        // Check node properties
        assert_eq!(
            node_val.get("status"),
            Some(&Value::String("published".to_string()))
        );
        assert_eq!(node_val.get("priority"), Some(&Value::Integer(5)));
    }

    #[test]
    fn test_build_rel_context_anonymous_user() {
        let auth = AuthContext::anonymous();
        let node = make_test_node();

        let ctx = build_rel_context(&auth, &node);

        let auth_val = ctx.get("auth").unwrap();
        assert_eq!(auth_val.get("user_id"), Some(&Value::Null));
        assert_eq!(auth_val.get("is_anonymous"), Some(&Value::Boolean(true)));
    }

    #[test]
    fn test_build_rel_context_system_user() {
        let auth = AuthContext::system();
        let node = make_test_node();

        let ctx = build_rel_context(&auth, &node);

        let auth_val = ctx.get("auth").unwrap();
        assert_eq!(auth_val.get("is_system"), Some(&Value::Boolean(true)));
    }

    #[test]
    fn test_ownership_expression() {
        let auth = AuthContext::for_user("user-456");
        let node = make_test_node(); // created_by = "user-456"

        let ctx = build_rel_context(&auth, &node);

        // Evaluate ownership condition
        let result = raisin_rel::eval("node.created_by == auth.user_id", &ctx).unwrap();
        assert_eq!(result, Value::Boolean(true));
    }

    #[test]
    fn test_ownership_expression_mismatch() {
        let auth = AuthContext::for_user("different-user");
        let node = make_test_node(); // created_by = "user-456"

        let ctx = build_rel_context(&auth, &node);

        // Evaluate ownership condition - should fail
        let result = raisin_rel::eval("node.created_by == auth.user_id", &ctx).unwrap();
        assert_eq!(result, Value::Boolean(false));
    }

    #[test]
    fn test_property_condition() {
        let auth = AuthContext::for_user("user-123");
        let node = make_test_node(); // status = "published", priority = 5

        let ctx = build_rel_context(&auth, &node);

        // Test property conditions
        let result = raisin_rel::eval("node.status == 'published'", &ctx).unwrap();
        assert_eq!(result, Value::Boolean(true));

        let result = raisin_rel::eval("node.priority >= 5", &ctx).unwrap();
        assert_eq!(result, Value::Boolean(true));

        let result = raisin_rel::eval("node.priority < 3", &ctx).unwrap();
        assert_eq!(result, Value::Boolean(false));
    }

    #[test]
    fn test_complex_condition() {
        let auth = AuthContext::for_user("user-456").with_roles(vec!["editor".to_string()]);
        let node = make_test_node();

        let ctx = build_rel_context(&auth, &node);

        // Owner OR has editor role
        let result = raisin_rel::eval(
            "node.created_by == auth.user_id || auth.roles.contains('editor')",
            &ctx,
        )
        .unwrap();
        assert_eq!(result, Value::Boolean(true));

        // Both conditions: owner AND published
        let result = raisin_rel::eval(
            "node.created_by == auth.user_id && node.status == 'published'",
            &ctx,
        )
        .unwrap();
        assert_eq!(result, Value::Boolean(true));
    }

    #[test]
    fn test_auth_home_variable() {
        let auth = AuthContext::for_user("user-123").with_home("/users/user-123");
        let node = make_test_node();

        let ctx = build_rel_context(&auth, &node);

        // Check auth.home is set
        let auth_val = ctx.get("auth").unwrap();
        assert_eq!(
            auth_val.get("home"),
            Some(&Value::String("/users/user-123".to_string()))
        );
    }

    #[test]
    fn test_auth_home_null_when_not_set() {
        let auth = AuthContext::for_user("user-123");
        let node = make_test_node();

        let ctx = build_rel_context(&auth, &node);

        // Check auth.home is null when not set
        let auth_val = ctx.get("auth").unwrap();
        assert_eq!(auth_val.get("home"), Some(&Value::Null));
    }
}
