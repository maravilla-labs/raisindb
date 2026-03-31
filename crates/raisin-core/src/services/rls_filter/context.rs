//! REL context building for RLS condition evaluation.
//!
//! Converts AuthContext and Node data into a REL EvalContext
//! with `auth.*` and `node.*` variables.

use raisin_models::auth::AuthContext;
use raisin_models::nodes::Node;
use raisin_rel::{EvalContext, Value};
use std::collections::HashMap;

/// Evaluate a REL (Raisin Expression Language) condition against a node.
///
/// Returns true if the expression evaluates to a truthy value.
/// Returns false on parse/eval errors (fail-closed security).
pub(super) fn evaluate_rel_condition(expr: &str, node: &Node, auth: &AuthContext) -> bool {
    let ctx = build_rel_context(auth, node);
    match raisin_rel::eval(expr, &ctx) {
        Ok(value) => value.is_truthy(),
        Err(e) => {
            tracing::warn!(
                expr = %expr,
                error = %e,
                "REL condition evaluation failed in RLS filter"
            );
            false // Fail-closed: deny on error
        }
    }
}

/// Build a REL EvalContext from AuthContext and Node.
fn build_rel_context(auth: &AuthContext, node: &Node) -> EvalContext {
    let mut ctx = EvalContext::new();

    // Build auth object
    let mut auth_obj = HashMap::new();

    if let Some(user_id) = &auth.user_id {
        auth_obj.insert("user_id".to_string(), Value::String(user_id.clone()));
    } else {
        auth_obj.insert("user_id".to_string(), Value::Null);
    }

    if let Some(local_user_id) = &auth.local_user_id {
        auth_obj.insert(
            "local_user_id".to_string(),
            Value::String(local_user_id.clone()),
        );
    } else {
        auth_obj.insert("local_user_id".to_string(), Value::Null);
    }

    if let Some(email) = &auth.email {
        auth_obj.insert("email".to_string(), Value::String(email.clone()));
    } else {
        auth_obj.insert("email".to_string(), Value::Null);
    }

    auth_obj.insert(
        "is_anonymous".to_string(),
        Value::Boolean(auth.is_anonymous),
    );
    auth_obj.insert("is_system".to_string(), Value::Boolean(auth.is_system));

    auth_obj.insert(
        "roles".to_string(),
        Value::Array(
            auth.roles
                .iter()
                .map(|r| Value::String(r.clone()))
                .collect(),
        ),
    );

    auth_obj.insert(
        "groups".to_string(),
        Value::Array(
            auth.groups
                .iter()
                .map(|g| Value::String(g.clone()))
                .collect(),
        ),
    );

    if let Some(ward_id) = &auth.acting_as_ward {
        auth_obj.insert("acting_as_ward".to_string(), Value::String(ward_id.clone()));
    } else {
        auth_obj.insert("acting_as_ward".to_string(), Value::Null);
    }

    if let Some(source) = &auth.active_stewardship_source {
        auth_obj.insert(
            "active_stewardship_source".to_string(),
            Value::String(source.clone()),
        );
    } else {
        auth_obj.insert("active_stewardship_source".to_string(), Value::Null);
    }

    if let Some(home) = &auth.home {
        auth_obj.insert("home".to_string(), Value::String(home.clone()));
    } else {
        auth_obj.insert("home".to_string(), Value::Null);
    }

    ctx.set("auth", Value::Object(auth_obj));

    // Build node object
    let mut node_obj = HashMap::new();
    node_obj.insert("id".to_string(), Value::String(node.id.clone()));
    node_obj.insert("name".to_string(), Value::String(node.name.clone()));
    node_obj.insert("path".to_string(), Value::String(node.path.clone()));
    node_obj.insert(
        "node_type".to_string(),
        Value::String(node.node_type.clone()),
    );

    if let Some(created_by) = &node.created_by {
        node_obj.insert("created_by".to_string(), Value::String(created_by.clone()));
    } else {
        node_obj.insert("created_by".to_string(), Value::Null);
    }

    if let Some(updated_by) = &node.updated_by {
        node_obj.insert("updated_by".to_string(), Value::String(updated_by.clone()));
    } else {
        node_obj.insert("updated_by".to_string(), Value::Null);
    }

    if let Some(owner_id) = &node.owner_id {
        node_obj.insert("owner_id".to_string(), Value::String(owner_id.clone()));
    } else {
        node_obj.insert("owner_id".to_string(), Value::Null);
    }

    if let Some(workspace) = &node.workspace {
        node_obj.insert("workspace".to_string(), Value::String(workspace.clone()));
    } else {
        node_obj.insert("workspace".to_string(), Value::Null);
    }

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
        PropertyValue::Date(dt) => Value::String(dt.to_string()),
        PropertyValue::Decimal(d) => Value::String(d.to_string()),
        PropertyValue::Reference(r) => Value::String(r.id.clone()),
        PropertyValue::Url(u) => Value::String(u.url.clone()),
        PropertyValue::Resource(r) => Value::String(r.uuid.clone()),
        PropertyValue::Composite(_) => Value::Null,
        PropertyValue::Element(_) => Value::Null,
        PropertyValue::Vector(v) => {
            Value::Array(v.iter().map(|f| Value::Float(*f as f64)).collect())
        }
        PropertyValue::Geometry(_) => Value::Null,
    }
}
