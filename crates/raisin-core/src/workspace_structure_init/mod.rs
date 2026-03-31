//! Workspace initial_structure initialization
//!
//! This module handles the creation of initial root-level nodes
//! defined in a workspace's initial_structure field.

mod children;

use raisin_error::Result;
use raisin_models as models;
use raisin_models::auth::AuthContext;
use raisin_storage::{scope::RepoScope, transactional::TransactionalStorage, WorkspaceRepository};
use std::collections::HashMap;
use std::sync::Arc;

fn convert_properties(
    properties: Option<&HashMap<String, serde_json::Value>>,
) -> HashMap<String, models::nodes::properties::PropertyValue> {
    properties
        .map(|props| {
            props
                .iter()
                .filter_map(|(key, value)| {
                    serde_json::from_value::<models::nodes::properties::PropertyValue>(
                        value.clone(),
                    )
                    .ok()
                    .map(|pv| (key.clone(), pv))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn convert_translations(
    translations: Option<&HashMap<String, serde_json::Value>>,
) -> Option<HashMap<String, models::nodes::properties::PropertyValue>> {
    translations.map(|trans| {
        trans
            .iter()
            .filter_map(|(key, value)| {
                serde_json::from_value::<models::nodes::properties::PropertyValue>(value.clone())
                    .ok()
                    .map(|pv| (key.clone(), pv))
            })
            .collect()
    })
}

/// Creates initial root-level nodes for a workspace based on its initial_structure definition
///
/// This function uses a transaction to atomically create all nodes and update the branch HEAD.
pub async fn create_workspace_initial_structure<S: TransactionalStorage>(
    storage: Arc<S>,
    tenant_id: &str,
    repository_id: &str,
    workspace_name: &str,
) -> Result<()> {
    tracing::info!(
        "Creating initial_structure for workspace {}/{}/{}",
        tenant_id,
        repository_id,
        workspace_name
    );

    // Get the workspace definition
    let workspace = storage
        .workspaces()
        .get(RepoScope::new(tenant_id, repository_id), workspace_name)
        .await?
        .ok_or_else(|| {
            raisin_error::Error::NotFound(format!("Workspace '{}' not found", workspace_name))
        })?;

    // Check if workspace has initial_structure defined
    let initial_children = match workspace
        .initial_structure
        .as_ref()
        .and_then(|structure| structure.children.as_ref())
    {
        Some(children) if !children.is_empty() => children,
        _ => {
            tracing::debug!(
                "Workspace {}/{}/{} has no initial_structure defined, skipping",
                tenant_id,
                repository_id,
                workspace_name
            );
            return Ok(());
        }
    };

    // Get the default branch from workspace config
    let branch = workspace.config.default_branch.clone();

    tracing::info!(
        "Creating {} root nodes in workspace {}/{}/{} on branch '{}'",
        initial_children.len(),
        tenant_id,
        repository_id,
        workspace_name,
        branch
    );

    // Begin transaction to atomically create all nodes and update HEAD
    let ctx = storage.begin_context().await?;

    // Set transaction metadata (required for HEAD update and RevisionMeta)
    ctx.set_tenant_repo(tenant_id, repository_id)?;
    ctx.set_branch(&branch)?;
    ctx.set_actor("system")?;
    ctx.set_auth_context(AuthContext::system())?;
    ctx.set_message(&format!(
        "Initialize workspace structure for {}",
        workspace_name
    ))?;

    // Create all nodes within the transaction
    children::create_children_iterative(
        ctx.as_ref(),
        tenant_id,
        repository_id,
        &branch,
        workspace_name,
        initial_children,
    )
    .await?;

    // Commit the transaction (atomically updates all nodes + HEAD)
    ctx.commit().await?;

    tracing::info!(
        "Successfully created initial_structure for workspace {}/{}/{}",
        tenant_id,
        repository_id,
        workspace_name
    );

    Ok(())
}

#[cfg(test)]
mod tests;
