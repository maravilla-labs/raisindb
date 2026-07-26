// SPDX-License-Identifier: BSL-1.1

//! Shared helpers for node operation handlers.
//!
//! Contains type conversion utilities and common context extraction
//! logic used by all node handler submodules.

use parking_lot::RwLock;
use raisin_core::NodeService;
use raisin_models::nodes::properties::value::Element;
use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::transactional::TransactionalStorage;
use std::sync::Arc;

use crate::{
    connection::ConnectionState, error::WsError, handler::WsState, protocol::RequestEnvelope,
};

/// Convert serde_json::Value to PropertyValue (from request payload)
pub(crate) fn json_to_property_value(value: &serde_json::Value) -> PropertyValue {
    match value {
        serde_json::Value::String(s) => PropertyValue::String(s.clone()),
        serde_json::Value::Number(n) => {
            // Check if the number is an integer or float
            if n.is_i64() || n.is_u64() {
                PropertyValue::Integer(n.as_i64().unwrap_or(0))
            } else {
                PropertyValue::Float(n.as_f64().unwrap_or(0.0))
            }
        }
        serde_json::Value::Bool(b) => PropertyValue::Boolean(*b),
        serde_json::Value::Array(arr) => {
            PropertyValue::Array(arr.iter().map(json_to_property_value).collect())
        }
        serde_json::Value::Object(obj) => {
            // Flat maps carrying element_type are Elements — mirrors the
            // canonical serde deserializer so SectionField content written
            // over WS survives archetype validation.
            if let Some(serde_json::Value::String(element_type)) = obj.get("element_type") {
                let uuid = obj
                    .get("uuid")
                    .and_then(|u| u.as_str())
                    .unwrap_or("")
                    .to_string();
                let content = obj
                    .iter()
                    .filter(|(k, _)| k.as_str() != "element_type" && k.as_str() != "uuid")
                    .map(|(k, v)| (k.clone(), json_to_property_value(v)))
                    .collect();
                return PropertyValue::Element(Element {
                    uuid,
                    element_type: element_type.clone(),
                    content,
                });
            }
            PropertyValue::Object(
                obj.iter()
                    .map(|(k, v)| (k.clone(), json_to_property_value(v)))
                    .collect(),
            )
        }
        // Preserve explicit nulls. The HTTP path deserializes JSON null to
        // PropertyValue::Null via serde; coercing to "" here silently
        // corrupted null-valued properties written over WS.
        serde_json::Value::Null => PropertyValue::Null,
    }
}

/// Validated context extracted from a request envelope.
pub(crate) struct RequestContext<'a> {
    pub tenant_id: &'a str,
    pub repo: &'a str,
    pub branch: &'a str,
    pub workspace: &'a str,
}

/// Extract and validate the common context fields from a request envelope.
///
/// Returns references into the envelope so no allocations are needed for
/// the happy path.
pub(crate) fn extract_context<'a>(
    request: &'a RequestEnvelope,
) -> Result<RequestContext<'a>, WsError> {
    let tenant_id = request.context.tenant_id.as_str();
    let repo = request
        .context
        .repository
        .as_deref()
        .ok_or_else(|| WsError::InvalidRequest("Repository required".to_string()))?;
    let branch = request.context.branch.as_deref().unwrap_or("main");
    let workspace = request
        .context
        .workspace
        .as_deref()
        .ok_or_else(|| WsError::InvalidRequest("Workspace required".to_string()))?;

    Ok(RequestContext {
        tenant_id,
        repo,
        branch,
        workspace,
    })
}

/// The acting user for this connection, for commit attribution.
///
/// Ordering operations record a revision, so they should be attributed like any
/// other write. Returns `None` for unauthenticated connections, which the
/// storage layer renders as `"system"`.
pub(crate) fn connection_actor(connection_state: &Arc<RwLock<ConnectionState>>) -> Option<String> {
    let conn = connection_state.read();
    conn.auth_context().and_then(|auth| auth.user_id.clone())
}

/// Build a `NodeService` from the shared WsState, connection state, and
/// validated request context. Applies auth context for RLS when present.
pub(crate) fn build_node_service<S, B>(
    state: &Arc<WsState<S, B>>,
    connection_state: &Arc<RwLock<ConnectionState>>,
    ctx: &RequestContext<'_>,
) -> NodeService<S>
where
    S: raisin_storage::Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let auth_context = {
        let conn = connection_state.read();
        conn.auth_context().cloned()
    };

    // Audit logging is handled globally by the event-bus AuditEventHandler
    // (covers node API, SQL DML, and functions uniformly), so the per-service
    // audit sink is not wired here.
    let mut node_service = NodeService::new_with_context(
        state.storage.clone(),
        ctx.tenant_id.to_string(),
        ctx.repo.to_string(),
        ctx.branch.to_string(),
        ctx.workspace.to_string(),
    );

    if let Some(auth) = auth_context {
        node_service = node_service.with_auth(auth);
    }

    node_service
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_maps_to_property_value_null_not_empty_string() {
        let value = serde_json::Value::Null;
        let converted = json_to_property_value(&value);
        assert!(
            matches!(converted, PropertyValue::Null),
            "JSON null must map to PropertyValue::Null, got {:?}",
            converted
        );
    }

    #[test]
    fn null_round_trips_through_json() {
        // Null must survive WS write -> PropertyValue -> JSON response.
        let converted = json_to_property_value(&serde_json::Value::Null);
        let back = serde_json::to_value(&converted).expect("serialize");
        assert_eq!(back, serde_json::Value::Null);
    }

    #[test]
    fn nested_nulls_are_preserved() {
        let value = serde_json::json!({
            "assignee": null,
            "tags": [null, "a"],
        });
        let converted = json_to_property_value(&value);
        match converted {
            PropertyValue::Object(obj) => {
                assert!(matches!(obj.get("assignee"), Some(PropertyValue::Null)));
                match obj.get("tags") {
                    Some(PropertyValue::Array(items)) => {
                        assert!(matches!(items[0], PropertyValue::Null));
                        assert!(matches!(items[1], PropertyValue::String(ref s) if s == "a"));
                    }
                    other => panic!("expected array, got {:?}", other),
                }
            }
            other => panic!("expected object, got {:?}", other),
        }
    }
}
