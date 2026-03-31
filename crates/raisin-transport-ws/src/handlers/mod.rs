// SPDX-License-Identifier: BSL-1.1

//! Request handlers for WebSocket operations
//!
//! This module routes incoming requests to appropriate handlers.

use parking_lot::RwLock;
use std::sync::Arc;

use crate::{
    connection::ConnectionState,
    error::WsError,
    handler::WsState,
    protocol::{RequestEnvelope, RequestType, ResponseEnvelope},
};

mod archetypes;
mod auth;
mod branches;
mod element_types;
mod node_types;
mod nodes;
mod repositories;
mod subscriptions;
mod tags;
mod transactions;
mod flow_events;
mod flows;
mod functions;
mod translations;
mod workspaces;

pub use archetypes::*;
pub use auth::*;
pub use branches::*;
pub use element_types::*;
pub use node_types::*;
pub use nodes::*;
pub use repositories::*;
pub use subscriptions::*;
pub use tags::*;
pub use transactions::*;
pub use flow_events::*;
pub use flows::*;
pub use functions::*;
pub use translations::*;
pub use workspaces::*;

/// Route a request to the appropriate handler
///
/// Returns:
/// - Ok(Some(response)) - Response should be sent back to client
/// - Ok(None) - Response already sent (e.g., streaming or async)
/// - Err(error) - Error occurred, send error response
pub async fn route_request<S, B>(
    state: &Arc<WsState<S, B>>,
    connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage + raisin_storage::transactional::TransactionalStorage + 'static,
    B: raisin_binary::BinaryStorage + 'static,
{
    let request_id = request.request_id.clone();

    match request.request_type {
        // Authentication
        RequestType::Authenticate => handle_authenticate(state, connection_state, request).await,
        RequestType::AuthenticateJwt => {
            handle_authenticate_jwt(state, connection_state, request).await
        }
        RequestType::RefreshToken => handle_refresh_token(state, connection_state, request).await,

        // Node operations
        RequestType::NodeCreate => handle_node_create(state, connection_state, request).await,
        RequestType::NodeUpdate => handle_node_update(state, connection_state, request).await,
        RequestType::NodeDelete => handle_node_delete(state, connection_state, request).await,
        RequestType::NodeGet => handle_node_get(state, connection_state, request).await,
        RequestType::NodeQuery => handle_node_query(state, connection_state, request).await,
        RequestType::NodeQueryByPath => {
            handle_node_query_by_path(state, connection_state, request).await
        }
        RequestType::NodeQueryByProperty => {
            handle_node_query_by_property(state, connection_state, request).await
        }

        // SQL queries
        RequestType::SqlQuery => handle_sql_query(state, connection_state, request).await,

        // Workspace operations
        RequestType::WorkspaceCreate => {
            handle_workspace_create(state, connection_state, request).await
        }
        RequestType::WorkspaceGet => handle_workspace_get(state, connection_state, request).await,
        RequestType::WorkspaceList => handle_workspace_list(state, connection_state, request).await,
        RequestType::WorkspaceDelete => {
            handle_workspace_delete(state, connection_state, request).await
        }

        // Event subscriptions
        RequestType::Subscribe => handle_subscribe(state, connection_state, request).await,
        RequestType::Unsubscribe => handle_unsubscribe(state, connection_state, request).await,

        // Node manipulation operations
        RequestType::NodeMove => handle_node_move(state, connection_state, request).await,
        RequestType::NodeRename => handle_node_rename(state, connection_state, request).await,
        RequestType::NodeCopy => handle_node_copy(state, connection_state, request).await,
        RequestType::NodeCopyTree => handle_node_copy_tree(state, connection_state, request).await,
        RequestType::NodeReorder => handle_node_reorder(state, connection_state, request).await,
        RequestType::NodeMoveChildBefore => {
            handle_node_move_child_before(state, connection_state, request).await
        }
        RequestType::NodeMoveChildAfter => {
            handle_node_move_child_after(state, connection_state, request).await
        }

        // Tree operations
        RequestType::NodeListChildren => {
            handle_node_list_children(state, connection_state, request).await
        }
        RequestType::NodeGetTree => handle_node_get_tree(state, connection_state, request).await,
        RequestType::NodeGetTreeFlat => {
            handle_node_get_tree_flat(state, connection_state, request).await
        }

        // Property path operations
        RequestType::PropertyGet => handle_property_get(state, connection_state, request).await,
        RequestType::PropertyUpdate => {
            handle_property_update(state, connection_state, request).await
        }

        // Relationship operations
        RequestType::RelationAdd => handle_relation_add(state, connection_state, request).await,
        RequestType::RelationRemove => {
            handle_relation_remove(state, connection_state, request).await
        }
        RequestType::RelationsGet => handle_relations_get(state, connection_state, request).await,

        // Translation operations
        RequestType::TranslationUpdate => {
            handle_translation_update(state, connection_state, request).await
        }
        RequestType::TranslationList => {
            handle_translation_list(state, connection_state, request).await
        }
        RequestType::TranslationDelete => {
            handle_translation_delete(state, connection_state, request).await
        }
        RequestType::TranslationHide => {
            handle_translation_hide(state, connection_state, request).await
        }
        RequestType::TranslationUnhide => {
            handle_translation_unhide(state, connection_state, request).await
        }

        // Transaction operations
        RequestType::TransactionBegin => {
            handle_transaction_begin(state, connection_state, request).await
        }
        RequestType::TransactionCommit => {
            handle_transaction_commit(state, connection_state, request).await
        }
        RequestType::TransactionRollback => {
            handle_transaction_rollback(state, connection_state, request).await
        }

        // Repository operations
        RequestType::RepositoryCreate => {
            handle_repository_create(state, connection_state, request).await
        }
        RequestType::RepositoryGet => handle_repository_get(state, connection_state, request).await,
        RequestType::RepositoryList => {
            handle_repository_list(state, connection_state, request).await
        }
        RequestType::RepositoryUpdate => {
            handle_repository_update(state, connection_state, request).await
        }
        RequestType::RepositoryDelete => {
            handle_repository_delete(state, connection_state, request).await
        }

        // NodeType operations
        RequestType::NodeTypeCreate => {
            handle_node_type_create(state, connection_state, request).await
        }
        RequestType::NodeTypeGet => handle_node_type_get(state, connection_state, request).await,
        RequestType::NodeTypeList => handle_node_type_list(state, connection_state, request).await,
        RequestType::NodeTypeUpdate => {
            handle_node_type_update(state, connection_state, request).await
        }
        RequestType::NodeTypeDelete => {
            handle_node_type_delete(state, connection_state, request).await
        }
        RequestType::NodeTypePublish => {
            handle_node_type_publish(state, connection_state, request).await
        }
        RequestType::NodeTypeUnpublish => {
            handle_node_type_unpublish(state, connection_state, request).await
        }
        RequestType::NodeTypeValidate => {
            handle_node_type_validate(state, connection_state, request).await
        }
        RequestType::NodeTypeGetResolved => {
            handle_node_type_get_resolved(state, connection_state, request).await
        }

        // Archetype operations
        RequestType::ArchetypeCreate => {
            handle_archetype_create(state, connection_state, request).await
        }
        RequestType::ArchetypeGet => handle_archetype_get(state, connection_state, request).await,
        RequestType::ArchetypeList => handle_archetype_list(state, connection_state, request).await,
        RequestType::ArchetypeUpdate => {
            handle_archetype_update(state, connection_state, request).await
        }
        RequestType::ArchetypeDelete => {
            handle_archetype_delete(state, connection_state, request).await
        }
        RequestType::ArchetypePublish => {
            handle_archetype_publish(state, connection_state, request).await
        }
        RequestType::ArchetypeUnpublish => {
            handle_archetype_unpublish(state, connection_state, request).await
        }

        // ElementType operations
        RequestType::ElementTypeCreate => {
            handle_element_type_create(state, connection_state, request).await
        }
        RequestType::ElementTypeGet => {
            handle_element_type_get(state, connection_state, request).await
        }
        RequestType::ElementTypeList => {
            handle_element_type_list(state, connection_state, request).await
        }
        RequestType::ElementTypeUpdate => {
            handle_element_type_update(state, connection_state, request).await
        }
        RequestType::ElementTypeDelete => {
            handle_element_type_delete(state, connection_state, request).await
        }
        RequestType::ElementTypePublish => {
            handle_element_type_publish(state, connection_state, request).await
        }
        RequestType::ElementTypeUnpublish => {
            handle_element_type_unpublish(state, connection_state, request).await
        }

        // Branch operations
        RequestType::BranchCreate => handle_branch_create(state, connection_state, request).await,
        RequestType::BranchGet => handle_branch_get(state, connection_state, request).await,
        RequestType::BranchList => handle_branch_list(state, connection_state, request).await,
        RequestType::BranchDelete => handle_branch_delete(state, connection_state, request).await,
        RequestType::BranchGetHead => {
            handle_branch_get_head(state, connection_state, request).await
        }
        RequestType::BranchUpdateHead => {
            handle_branch_update_head(state, connection_state, request).await
        }
        RequestType::BranchMerge => handle_branch_merge(state, connection_state, request).await,
        RequestType::BranchCompare => handle_branch_compare(state, connection_state, request).await,

        // Tag operations
        RequestType::TagCreate => handle_tag_create(state, connection_state, request).await,
        RequestType::TagGet => handle_tag_get(state, connection_state, request).await,
        RequestType::TagList => handle_tag_list(state, connection_state, request).await,
        RequestType::TagDelete => handle_tag_delete(state, connection_state, request).await,

        // Workspace update (missing from earlier)
        RequestType::WorkspaceUpdate => {
            handle_workspace_update(state, connection_state, request).await
        }

        // Flow operations
        RequestType::FlowRun => handle_flow_run(state, connection_state, request).await,
        RequestType::FlowResume => handle_flow_resume(state, connection_state, request).await,
        RequestType::FlowGetInstanceStatus => {
            handle_flow_get_instance_status(state, connection_state, request).await
        }
        RequestType::FlowCancel => handle_flow_cancel(state, connection_state, request).await,
        RequestType::FlowSubscribeEvents => {
            handle_flow_subscribe_events(state, connection_state, request).await
        }
        RequestType::FlowUnsubscribeEvents => {
            handle_flow_unsubscribe_events(state, connection_state, request).await
        }

        // Function operations
        RequestType::FunctionInvoke => {
            handle_function_invoke(state, connection_state, request).await
        }
        RequestType::FunctionInvokeSync => {
            handle_function_invoke_sync(state, connection_state, request).await
        }

        // Not yet implemented
        _ => Ok(Some(ResponseEnvelope::error(
            request_id,
            "NOT_IMPLEMENTED".to_string(),
            format!(
                "Request type {:?} not yet implemented",
                request.request_type
            ),
        ))),
    }
}
