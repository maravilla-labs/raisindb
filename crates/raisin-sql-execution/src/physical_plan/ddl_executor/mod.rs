//! DDL Executor for Schema Management
//!
//! Executes DDL statements (CREATE/ALTER/DROP) for NodeTypes, Archetypes, and ElementTypes.
//! Converts DDL AST nodes to model types and calls storage repository methods.

mod archetype;
mod conversions;
mod element_type;
mod mixin;
mod nested_properties;
mod node_type;

use crate::physical_plan::executor::{Row, RowStream};
use indexmap::IndexMap;
use raisin_error::Error;
use raisin_models::nodes::properties::PropertyValue;
use raisin_sql::ast::ddl::DdlStatement;
use raisin_storage::Storage;
use std::sync::Arc;

/// Execute a DDL statement and return a result stream
pub async fn execute_ddl<S: Storage + 'static>(
    ddl: &DdlStatement,
    storage: Arc<S>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
) -> Result<RowStream, Error> {
    match ddl {
        // NodeType operations
        DdlStatement::CreateNodeType(create) => {
            node_type::execute_create_nodetype(create, storage, tenant_id, repo_id, branch).await
        }
        DdlStatement::AlterNodeType(alter) => {
            node_type::execute_alter_nodetype(alter, storage, tenant_id, repo_id, branch).await
        }
        DdlStatement::DropNodeType(drop) => {
            node_type::execute_drop_nodetype(drop, storage, tenant_id, repo_id, branch).await
        }

        // Mixin operations
        DdlStatement::CreateMixin(create) => {
            mixin::execute_create_mixin(create, storage, tenant_id, repo_id, branch).await
        }
        DdlStatement::AlterMixin(alter) => {
            mixin::execute_alter_mixin(alter, storage, tenant_id, repo_id, branch).await
        }
        DdlStatement::DropMixin(drop) => {
            mixin::execute_drop_mixin(drop, storage, tenant_id, repo_id, branch).await
        }

        // Archetype operations
        DdlStatement::CreateArchetype(create) => {
            archetype::execute_create_archetype(create, storage, tenant_id, repo_id, branch).await
        }
        DdlStatement::AlterArchetype(alter) => {
            archetype::execute_alter_archetype(alter, storage, tenant_id, repo_id, branch).await
        }
        DdlStatement::DropArchetype(drop) => {
            archetype::execute_drop_archetype(drop, storage, tenant_id, repo_id, branch).await
        }

        // ElementType operations
        DdlStatement::CreateElementType(create) => {
            element_type::execute_create_elementtype(create, storage, tenant_id, repo_id, branch)
                .await
        }
        DdlStatement::AlterElementType(alter) => {
            element_type::execute_alter_elementtype(alter, storage, tenant_id, repo_id, branch)
                .await
        }
        DdlStatement::DropElementType(drop) => {
            element_type::execute_drop_elementtype(drop, storage, tenant_id, repo_id, branch).await
        }
    }
}

/// Create a success result stream with a message
fn ddl_success_stream(message: &str) -> Result<RowStream, Error> {
    let mut row = IndexMap::new();
    row.insert(
        "result".to_string(),
        PropertyValue::String(message.to_string()),
    );
    row.insert("success".to_string(), PropertyValue::Boolean(true));

    let rows = vec![Ok(Row::from_map(row))];
    Ok(Box::pin(futures::stream::iter(rows)))
}
