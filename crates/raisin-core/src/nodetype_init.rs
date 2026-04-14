//! Global NodeType initialization
//!
//! This module provides functionality to initialize built-in NodeTypes
//! on repository creation. These NodeTypes are embedded from YAML files
//! and automatically registered in the repository.

use include_dir::{include_dir, Dir};
use raisin_error::Result;
use raisin_models::nodes::types::NodeType;
use raisin_storage::{scope::BranchScope, CommitMetadata, NodeTypeRepository, Storage};
use sha2::{Digest, Sha256};
use std::sync::Arc;

/// Embedded directory containing global NodeType YAML files
static GLOBAL_NODETYPES_DIR: Dir = include_dir!("$CARGO_MANIFEST_DIR/global_nodetypes");

/// Calculate SHA256 hash of content
///
/// Used to create a content-addressable hash for tracking which version
/// of a definition has been applied to a repository.
///
/// # Arguments
/// * `content` - The content to hash
///
/// # Returns
/// A hex-encoded SHA256 hash string (64 characters)
pub fn calculate_content_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Calculate version hash for global NodeTypes (legacy)
///
/// This function computes an MD5 hash of all embedded global NodeType YAML files.
/// The hash is used to detect when NodeType definitions change and trigger
/// re-initialization for repositories.
///
/// # Returns
/// A hex-encoded MD5 hash string
///
/// # Deprecated
/// This function is kept for backward compatibility. New code should use
/// `load_global_nodetypes_with_hashes()` for per-file hash tracking.
pub fn calculate_nodetype_version() -> String {
    let mut hasher = md5::Context::new();

    // Iterate over all .yaml files in the embedded directory
    for file in GLOBAL_NODETYPES_DIR.files() {
        if file.path().extension().and_then(|e| e.to_str()) == Some("yaml") {
            if let Some(content) = file.contents_utf8() {
                hasher.consume(content.as_bytes());
            }
        }
    }

    format!("{:x}", hasher.finalize())
}

/// Load global NodeType definitions from embedded YAML
pub fn load_global_nodetypes() -> Vec<NodeType> {
    load_global_nodetypes_with_hashes()
        .into_iter()
        .map(|(nt, _)| nt)
        .collect()
}

/// Load global NodeType definitions with their content hashes
///
/// This function loads all embedded NodeType YAML files and returns them
/// along with their SHA256 content hashes. The hash can be used to track
/// which version of each NodeType has been applied to a repository.
///
/// # Returns
/// A vector of tuples containing (NodeType, content_hash)
pub fn load_global_nodetypes_with_hashes() -> Vec<(NodeType, String)> {
    let mut nodetypes = Vec::new();

    // Iterate over all .yaml files in the embedded directory
    for file in GLOBAL_NODETYPES_DIR.files() {
        if file.path().extension().and_then(|e| e.to_str()) == Some("yaml") {
            if let Some(content) = file.contents_utf8() {
                let content_hash = calculate_content_hash(content);
                match serde_yaml::from_str::<NodeType>(content) {
                    Ok(nodetype) => {
                        tracing::debug!(
                            "Loaded NodeType '{}' from {} (hash: {})",
                            nodetype.name,
                            file.path().display(),
                            &content_hash[..8]
                        );
                        nodetypes.push((nodetype, content_hash));
                    }
                    Err(e) => {
                        tracing::error!(
                            "Failed to parse NodeType YAML from {}: {}",
                            file.path().display(),
                            e
                        );
                    }
                }
            }
        }
    }

    nodetypes
}

/// Initialize global NodeTypes for a specific tenant, repository, and branch
///
/// This function initializes built-in NodeTypes from embedded YAML files
/// when a new repository is created. It ensures idempotent behavior by
/// checking if NodeTypes already exist before creating them, and updates
/// them if a newer version is found.
///
/// # Arguments
/// * `storage` - Storage instance
/// * `tenant_id` - Tenant identifier
/// * `repo_id` - Repository identifier
/// * `branch` - Branch name
///
/// # Process
/// 1. Loads NodeType definitions from embedded YAML files
/// 2. Checks if each NodeType already exists in the repository
/// 3. If doesn't exist, creates it
/// 4. If exists and version is newer, updates it
/// 5. If exists and version is same/older, skips (idempotent)
///
/// # Example
/// ```no_run
/// use raisin_core::nodetype_init::init_repository_nodetypes;
/// use raisin_storage_rocks::RocksStorage;
/// use raisin_error::Result;
/// use std::sync::Arc;
///
/// #[tokio::main]
/// async fn main() -> Result<()> {
///     let storage = Arc::new(RocksStorage::open("./data")?);
///
///     // Initialize NodeTypes for a new repository
///     init_repository_nodetypes(
///         storage.clone(),
///         "tenant-123",
///         "repo-456",
///         "main"
///     ).await?;
///
///     Ok(())
/// }
/// ```
pub async fn init_repository_nodetypes<S: Storage>(
    storage: Arc<S>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
) -> Result<()> {
    let nodetypes = load_global_nodetypes();

    tracing::info!(
        "Initializing {} global NodeType(s) for repository: {}/{}/{}",
        nodetypes.len(),
        tenant_id,
        repo_id,
        branch
    );

    let repo = storage.node_types();

    for mut nodetype in nodetypes {
        let nodetype_name = nodetype.name.clone();

        // Generate ID if not present
        if nodetype.id.is_none() {
            nodetype.id = Some(nanoid::nanoid!());
        }

        // Set timestamps
        if nodetype.created_at.is_none() {
            nodetype.created_at = Some(chrono::Utc::now());
        }

        // Check if NodeType already exists
        match repo
            .get(
                BranchScope::new(tenant_id, repo_id, branch),
                &nodetype_name,
                None,
            )
            .await?
        {
            Some(existing) => {
                tracing::debug!(
                    "NodeType '{}' already exists in {}/{}/{}, checking version",
                    nodetype_name,
                    tenant_id,
                    repo_id,
                    branch
                );

                // Compare versions and update if newer
                let existing_version = existing.version.unwrap_or(0);
                let new_version = nodetype.version.unwrap_or(0);

                if new_version > existing_version {
                    nodetype.updated_at = Some(chrono::Utc::now());
                    nodetype.created_at = existing.created_at;
                    nodetype.id = existing.id; // Preserve existing ID

                    repo.put(
                        BranchScope::new(tenant_id, repo_id, branch),
                        nodetype.clone(),
                        CommitMetadata::system(format!("Update NodeType {}", nodetype_name)),
                    )
                    .await?;

                    tracing::info!(
                        "Updated NodeType '{}' in {}/{}/{} from version {} to {}",
                        nodetype_name,
                        tenant_id,
                        repo_id,
                        branch,
                        existing_version,
                        new_version
                    );
                } else {
                    tracing::debug!(
                        "NodeType '{}' version {} is up to date (existing: {})",
                        nodetype_name,
                        new_version,
                        existing_version
                    );
                }
            }
            None => {
                // Create new NodeType
                repo.put(
                    BranchScope::new(tenant_id, repo_id, branch),
                    nodetype.clone(),
                    CommitMetadata::system(format!("Create NodeType {}", nodetype_name)),
                )
                .await?;

                tracing::info!(
                    "Initialized global NodeType '{}' in {}/{}/{}",
                    nodetype_name,
                    tenant_id,
                    repo_id,
                    branch
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
    async fn test_init_repository_nodetypes() {
        let tenant_id = "test-tenant";
        let repo_id = "test-repo";
        let branch = "main";

        let storage = Arc::new(InMemoryStorage::default());

        // Initialize global NodeTypes
        init_repository_nodetypes(storage.clone(), tenant_id, repo_id, branch)
            .await
            .unwrap();

        // Verify NodeTypes were created
        let repo = storage.node_types();

        // Check for built-in types
        let folder = repo
            .get(
                BranchScope::new(tenant_id, repo_id, branch),
                "raisin:Folder",
                None,
            )
            .await
            .unwrap();
        assert!(folder.is_some());

        let page = repo
            .get(
                BranchScope::new(tenant_id, repo_id, branch),
                "raisin:Page",
                None,
            )
            .await
            .unwrap();
        assert!(page.is_some());

        let asset = repo
            .get(
                BranchScope::new(tenant_id, repo_id, branch),
                "raisin:Asset",
                None,
            )
            .await
            .unwrap();
        assert!(asset.is_some());

        // Check for access control types
        let user = repo
            .get(
                BranchScope::new(tenant_id, repo_id, branch),
                "raisin:User",
                None,
            )
            .await
            .unwrap();
        assert!(user.is_some());

        let role = repo
            .get(
                BranchScope::new(tenant_id, repo_id, branch),
                "raisin:Role",
                None,
            )
            .await
            .unwrap();
        assert!(role.is_some());

        let group = repo
            .get(
                BranchScope::new(tenant_id, repo_id, branch),
                "raisin:Group",
                None,
            )
            .await
            .unwrap();
        assert!(group.is_some());
    }

    #[tokio::test]
    async fn test_idempotent_initialization() {
        let storage = Arc::new(InMemoryStorage::default());
        let tenant_id = "test-tenant";
        let repo_id = "test-repo";
        let branch = "main";

        // Initialize once
        init_repository_nodetypes(storage.clone(), tenant_id, repo_id, branch)
            .await
            .unwrap();

        // Initialize again (should be idempotent)
        init_repository_nodetypes(storage.clone(), tenant_id, repo_id, branch)
            .await
            .unwrap();

        // Should still only have NodeTypes with no duplicates
        let repo = storage.node_types();
        let folder = repo
            .get(
                BranchScope::new(tenant_id, repo_id, branch),
                "raisin:Folder",
                None,
            )
            .await
            .unwrap();
        assert!(folder.is_some());
    }

    #[test]
    fn test_calculate_nodetype_version() {
        let version = calculate_nodetype_version();
        assert!(!version.is_empty());
        assert_eq!(version.len(), 32); // MD5 hash is 32 hex characters
    }

    #[test]
    fn test_load_global_nodetypes() {
        let nodetypes = load_global_nodetypes();
        assert!(!nodetypes.is_empty());

        // Verify all expected NodeTypes are loaded
        assert!(
            nodetypes.iter().any(|nt| nt.name == "raisin:Folder"),
            "Should load raisin_folder.yaml"
        );
        assert!(
            nodetypes.iter().any(|nt| nt.name == "raisin:Page"),
            "Should load raisin_page.yaml"
        );
        assert!(
            nodetypes.iter().any(|nt| nt.name == "raisin:Asset"),
            "Should load raisin_asset.yaml"
        );
        assert!(
            nodetypes.iter().any(|nt| nt.name == "raisin:User"),
            "Should load raisin_user.yaml"
        );
        assert!(
            nodetypes.iter().any(|nt| nt.name == "raisin:Role"),
            "Should load raisin_role.yaml"
        );
        assert!(
            nodetypes.iter().any(|nt| nt.name == "raisin:Group"),
            "Should load raisin_group.yaml"
        );

        println!("Loaded {} NodeTypes:", nodetypes.len());
        for nt in &nodetypes {
            println!("  - {}", nt.name);
        }
    }

    #[test]
    fn test_calculate_content_hash() {
        let hash = calculate_content_hash("test content");
        assert!(!hash.is_empty());
        assert_eq!(hash.len(), 64); // SHA256 hash is 64 hex characters

        // Same content should produce same hash
        let hash2 = calculate_content_hash("test content");
        assert_eq!(hash, hash2);

        // Different content should produce different hash
        let hash3 = calculate_content_hash("different content");
        assert_ne!(hash, hash3);
    }

    #[test]
    fn test_load_global_nodetypes_with_hashes() {
        let nodetypes_with_hashes = load_global_nodetypes_with_hashes();
        assert!(!nodetypes_with_hashes.is_empty());

        // Each nodetype should have a valid hash
        for (nodetype, hash) in &nodetypes_with_hashes {
            assert_eq!(
                hash.len(),
                64,
                "Hash for {} should be 64 chars",
                nodetype.name
            );
        }

        // Verify we have at least the core types
        assert!(
            nodetypes_with_hashes
                .iter()
                .any(|(nt, _)| nt.name == "raisin:Folder"),
            "Should have raisin:Folder"
        );

        println!(
            "Loaded {} NodeTypes with hashes:",
            nodetypes_with_hashes.len()
        );
        for (nt, hash) in &nodetypes_with_hashes {
            println!("  - {} (hash: {}...)", nt.name, &hash[..8]);
        }
    }
}
