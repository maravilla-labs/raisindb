//! Global Workspace initialization
//!
//! This module provides functionality to initialize built-in workspaces
//! on repository creation. These workspaces are embedded from YAML files
//! and automatically registered in the repository.

use crate::nodetype_init::calculate_content_hash;
use include_dir::{include_dir, Dir};
use raisin_error::Result;
use raisin_models::workspace::Workspace;
use raisin_storage::{scope::RepoScope, Storage, WorkspaceRepository};
use std::sync::Arc;

/// Embedded directory containing global workspace YAML files
static GLOBAL_WORKSPACES_DIR: Dir = include_dir!("$CARGO_MANIFEST_DIR/global_workspaces");

/// Calculate version hash for global Workspaces (legacy)
///
/// This function computes an MD5 hash of all embedded global Workspace YAML files.
/// The hash is used to detect when Workspace definitions change and trigger
/// re-initialization for repositories.
///
/// # Returns
/// A hex-encoded MD5 hash string
///
/// # Deprecated
/// This function is kept for backward compatibility. New code should use
/// `load_global_workspaces_with_hashes()` for per-file hash tracking.
pub fn calculate_workspace_version() -> String {
    let mut hasher = md5::Context::new();

    // Iterate over all .yaml files in the embedded directory
    for file in GLOBAL_WORKSPACES_DIR.files() {
        if file.path().extension().and_then(|e| e.to_str()) == Some("yaml") {
            if let Some(content) = file.contents_utf8() {
                hasher.consume(content.as_bytes());
            }
        }
    }

    format!("{:x}", hasher.finalize())
}

/// Load global Workspace definitions from embedded YAML
pub fn load_global_workspaces() -> Vec<Workspace> {
    load_global_workspaces_with_hashes()
        .into_iter()
        .map(|(ws, _)| ws)
        .collect()
}

/// Load global Workspace definitions with their content hashes
///
/// This function loads all embedded Workspace YAML files and returns them
/// along with their SHA256 content hashes. The hash can be used to track
/// which version of each Workspace has been applied to a repository.
///
/// # Returns
/// A vector of tuples containing (Workspace, content_hash)
pub fn load_global_workspaces_with_hashes() -> Vec<(Workspace, String)> {
    let mut workspaces = Vec::new();

    // Iterate over all .yaml files in the embedded directory
    for file in GLOBAL_WORKSPACES_DIR.files() {
        if file.path().extension().and_then(|e| e.to_str()) == Some("yaml") {
            if let Some(content) = file.contents_utf8() {
                let content_hash = calculate_content_hash(content);
                match serde_yaml::from_str::<Workspace>(content) {
                    Ok(workspace) => {
                        tracing::debug!(
                            "Loaded workspace '{}' from {} (hash: {})",
                            workspace.name,
                            file.path().display(),
                            &content_hash[..8]
                        );
                        workspaces.push((workspace, content_hash));
                    }
                    Err(e) => {
                        tracing::error!(
                            "Failed to parse Workspace YAML from {}: {}",
                            file.path().display(),
                            e
                        );
                    }
                }
            }
        }
    }

    workspaces
}

/// Initialize global workspaces for a specific tenant and repository
///
/// This function initializes built-in workspaces from embedded YAML files
/// when a new repository is created. It ensures idempotent behavior by
/// checking if workspaces already exist before creating them.
///
/// # Arguments
/// * `storage` - Storage instance
/// * `tenant_id` - Tenant identifier
/// * `repo_id` - Repository identifier
///
/// # Process
/// 1. Loads workspace definitions from embedded YAML files
/// 2. Checks if each workspace already exists in the repository
/// 3. If doesn't exist, creates it
/// 4. If exists, skips creation (idempotent)
///
/// # Example
/// ```no_run
/// use raisin_core::workspace_init::init_repository_workspaces;
/// use raisin_storage_rocks::RocksStorage;
/// use raisin_error::Result;
/// use std::sync::Arc;
///
/// #[tokio::main]
/// async fn main() -> Result<()> {
///     let storage = Arc::new(RocksStorage::open("./data")?);
///
///     // Initialize workspaces for a new repository
///     init_repository_workspaces(
///         storage.clone(),
///         "tenant-123",
///         "repo-456"
///     ).await?;
///
///     Ok(())
/// }
/// ```
pub async fn init_repository_workspaces<S: Storage>(
    storage: Arc<S>,
    tenant_id: &str,
    repo_id: &str,
) -> Result<()> {
    let workspaces = load_global_workspaces();

    tracing::info!(
        "Initializing {} global workspace(s) for repository: {}/{}",
        workspaces.len(),
        tenant_id,
        repo_id
    );

    let workspace_repo = storage.workspaces();

    for mut workspace in workspaces {
        let workspace_name = workspace.name.clone();

        // Check if workspace already exists
        match workspace_repo
            .get(RepoScope::new(tenant_id, repo_id), &workspace_name)
            .await?
        {
            Some(_existing) => {
                tracing::debug!(
                    "Workspace '{}' already exists in {}/{}, skipping",
                    workspace_name,
                    tenant_id,
                    repo_id
                );
            }
            None => {
                // Set timestamps
                workspace.created_at = raisin_models::StorageTimestamp::now();
                workspace.updated_at = None;

                // Create new workspace
                workspace_repo
                    .put(RepoScope::new(tenant_id, repo_id), workspace.clone())
                    .await?;

                tracing::info!(
                    "Initialized global workspace '{}' in {}/{}",
                    workspace_name,
                    tenant_id,
                    repo_id
                );
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_storage_memory::InMemoryStorage;

    #[tokio::test]
    async fn test_init_repository_workspaces() {
        let tenant_id = "test-tenant";
        let repo_id = "test-repo";

        let storage = Arc::new(InMemoryStorage::default());

        // Initialize global workspaces
        init_repository_workspaces(storage.clone(), tenant_id, repo_id)
            .await
            .unwrap();

        // Verify default workspace was created
        let workspace_repo = storage.workspaces();
        let default_workspace = workspace_repo
            .get(RepoScope::new(tenant_id, repo_id), "default")
            .await
            .unwrap();

        assert!(default_workspace.is_some());
        let ws = default_workspace.unwrap();
        assert_eq!(ws.name, "default");
        assert!(ws.allowed_node_types.contains(&"raisin:Folder".to_string()));
        assert!(ws.allowed_node_types.contains(&"raisin:Page".to_string()));
        assert!(ws.allowed_node_types.contains(&"raisin:Asset".to_string()));
    }

    #[tokio::test]
    async fn test_idempotent_initialization() {
        let storage = Arc::new(InMemoryStorage::default());
        let tenant_id = "test-tenant";
        let repo_id = "test-repo";

        // Initialize once
        init_repository_workspaces(storage.clone(), tenant_id, repo_id)
            .await
            .unwrap();

        // Initialize again (should be idempotent)
        init_repository_workspaces(storage.clone(), tenant_id, repo_id)
            .await
            .unwrap();

        // Should still only have one workspace
        let workspace_repo = storage.workspaces();
        let default_workspace = workspace_repo
            .get(RepoScope::new(tenant_id, repo_id), "default")
            .await
            .unwrap();

        assert!(default_workspace.is_some());
    }

    #[test]
    fn test_calculate_workspace_version() {
        let version = calculate_workspace_version();
        assert!(!version.is_empty());
        assert_eq!(version.len(), 32); // MD5 hash is 32 hex characters
    }

    #[test]
    fn test_load_global_workspaces() {
        let workspaces = load_global_workspaces();
        assert!(!workspaces.is_empty());

        // Verify all expected workspaces are loaded
        assert!(
            workspaces.iter().any(|ws| ws.name == "default"),
            "Should load default.yaml"
        );
        assert!(
            workspaces
                .iter()
                .any(|ws| ws.name == "raisin:access_control"),
            "Should load access_control.yaml"
        );

        println!("Loaded {} workspaces:", workspaces.len());
        for ws in &workspaces {
            println!("  - {}", ws.name);
        }
    }

    #[test]
    fn test_load_global_workspaces_with_hashes() {
        let workspaces_with_hashes = load_global_workspaces_with_hashes();
        assert!(!workspaces_with_hashes.is_empty());

        // Each workspace should have a valid hash
        for (workspace, hash) in &workspaces_with_hashes {
            assert_eq!(
                hash.len(),
                64,
                "Hash for {} should be 64 chars",
                workspace.name
            );
        }

        // Verify we have at least the core workspaces
        assert!(
            workspaces_with_hashes
                .iter()
                .any(|(ws, _)| ws.name == "default"),
            "Should have default workspace"
        );

        println!(
            "Loaded {} workspaces with hashes:",
            workspaces_with_hashes.len()
        );
        for (ws, hash) in &workspaces_with_hashes {
            println!("  - {} (hash: {}...)", ws.name, &hash[..8]);
        }
    }
}
