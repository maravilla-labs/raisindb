//! Global NodeType initialization
//!
//! This module provides functionality to initialize built-in "raisin:" NodeTypes
//! on server startup or per-tenant. These NodeTypes are embedded from YAML files
//! and automatically registered in the repository.

use include_dir::{include_dir, Dir};
use raisin_error::Result;
use raisin_models::nodes::types::NodeType;
use raisin_storage::{
    init_tenant_nodetypes as storage_init_tenant, scope::BranchScope, NodeTypeRepository, Storage,
};
use std::sync::Arc;

/// Embedded directory containing global nodetype YAML files
static GLOBAL_NODETYPES_DIR: Dir = include_dir!("$CARGO_MANIFEST_DIR/global_nodetypes");

/// Calculate version hash for global NodeTypes
///
/// This function computes an MD5 hash of all embedded global NodeType YAML files.
/// The hash is used to detect when NodeType definitions change and trigger
/// re-initialization for tenants.
///
/// # Returns
/// A hex-encoded MD5 hash string
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

/// Initialize NodeTypes for a specific tenant/deployment
///
/// This function performs lazy initialization of built-in NodeTypes for a tenant.
/// It should be called on first request for each tenant/deployment combination.
///
/// # Arguments
/// * `storage` - Base storage instance (must implement ScopableStorage)
/// * `tenant_id` - Tenant identifier
/// * `deployment_key` - Deployment key (e.g., "production", "staging")
///
/// # Process
/// 1. Calculate current global NodeType version hash
/// 2. Delegates to storage-level init function with NodeType definitions
/// 3. Version checking and registration handled by storage layer
///
/// # Example
/// ```no_run
/// use raisin_core::init::init_tenant_nodetypes;
/// use raisin_storage_rocks::RocksStorage;
/// use raisin_error::Result;
///
/// #[tokio::main]
/// async fn main() -> Result<()> {
///     let storage = RocksStorage::open("./data")?;
///
///     // Initialize NodeTypes for specific tenant on first request
///     init_tenant_nodetypes(
///         storage,
///         "tenant-123",
///         "production"
///     ).await?;
///
///     Ok(())
/// }
/// ```
pub async fn init_tenant_nodetypes<S>(
    storage: S,
    tenant_id: &str,
    deployment_key: &str,
) -> Result<()>
where
    S: Storage,
{
    storage_init_tenant(storage, tenant_id, deployment_key, || {
        let version = calculate_nodetype_version();
        let nodetypes = load_global_nodetypes();
        (version, nodetypes)
    })
    .await
}

/// Load global NodeType definitions from embedded YAML
fn load_global_nodetypes() -> Vec<NodeType> {
    let mut nodetypes = Vec::new();

    // Iterate over all .yaml files in the embedded directory
    for file in GLOBAL_NODETYPES_DIR.files() {
        if file.path().extension().and_then(|e| e.to_str()) == Some("yaml") {
            if let Some(content) = file.contents_utf8() {
                match serde_yaml::from_str::<NodeType>(content) {
                    Ok(nodetype) => {
                        tracing::debug!(
                            "Loaded nodetype '{}' from {}",
                            nodetype.name,
                            file.path().display()
                        );
                        nodetypes.push(nodetype);
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

/// Initialize global "raisin:" NodeTypes from embedded YAML files
///
/// This function:
/// 1. Loads built-in NodeType definitions from embedded YAML
/// 2. Checks if each NodeType already exists in the default repository
/// 3. If exists, compares versions and updates if newer version available
/// 4. If doesn't exist, creates it
///
/// Built-in NodeTypes are initialized in the default tenant for multiple repositories:
/// - tenant_id: "default"
/// - repo_id: "main" (legacy default) and "default" (new default)
/// - branch: "main"
///
/// Built-in NodeTypes:
/// - `raisin:Folder` - Container for organizing nodes
/// - `raisin:Page` - Basic content page with title and content
/// - `raisin:Asset` - Media asset (images, videos, documents)
///
/// # Example
/// ```no_run
/// use raisin_core::init::init_global_nodetypes;
/// use raisin_storage_rocks::RocksStorage;
/// use raisin_error::Result;
/// use std::sync::Arc;
///
/// #[tokio::main]
/// async fn main() -> Result<()> {
///     let storage = Arc::new(RocksStorage::open("./data")?);
///
///     // Initialize global NodeTypes on startup
///     init_global_nodetypes(storage.clone()).await?;
///
///     // Now raisin:Folder, raisin:Page, and raisin:Asset are available
///     Ok(())
/// }
/// ```
pub async fn init_global_nodetypes<S: Storage>(storage: Arc<S>) -> Result<()> {
    // Global NodeTypes are stored in the default tenant
    let tenant_id = "default";

    // Initialize for both "main" and "default" repositories for backwards compatibility
    // TODO: Replace with event-driven initialization when repositories are created
    let repos = vec![
        ("main", "main"),    // Legacy default repository
        ("default", "main"), // New default repository (used by integration tests)
    ];

    // Load global NodeTypes from embedded directory
    let global_nodetypes = load_global_nodetypes();

    for (repo_id, branch) in repos {
        tracing::info!(
            "Initializing NodeTypes for repository: {}/{}",
            repo_id,
            branch
        );

        for nodetype in &global_nodetypes {
            // Clone the nodetype to allow modifications
            let mut node_type = nodetype.clone();

            // Generate ID if not present
            if node_type.id.is_none() {
                node_type.id = Some(nanoid::nanoid!());
            }

            // Set timestamps
            if node_type.created_at.is_none() {
                node_type.created_at = Some(chrono::Utc::now());
            }

            // Check if NodeType already exists
            let repo = storage.node_types();
            match repo
                .get(
                    BranchScope::new(tenant_id, repo_id, branch),
                    &node_type.name,
                    None,
                )
                .await?
            {
                Some(existing) => {
                    // Compare versions
                    let existing_version = existing.version.unwrap_or(0);
                    let new_version = node_type.version.unwrap_or(0);

                    if new_version > existing_version {
                        // Update to newer version
                        node_type.updated_at = Some(chrono::Utc::now());
                        node_type.created_at = existing.created_at; // Preserve original creation time
                        repo.put(
                            BranchScope::new(tenant_id, repo_id, branch),
                            node_type.clone(),
                            raisin_storage::CommitMetadata::system(format!(
                                "Initialize global node type {}",
                                node_type.name.clone()
                            )),
                        )
                        .await?;
                        tracing::info!(
                            "Updated global NodeType '{}' in {}/{} from version {} to {}",
                            node_type.name,
                            repo_id,
                            branch,
                            existing_version,
                            new_version
                        );
                    } else {
                        tracing::debug!(
                            "Global NodeType '{}' already exists in {}/{} with version {} (current: {})",
                            node_type.name,
                            repo_id,
                            branch,
                            existing_version,
                            new_version
                        );
                    }
                }
                None => {
                    // Create new NodeType
                    repo.put(
                        BranchScope::new(tenant_id, repo_id, branch),
                        node_type.clone(),
                        raisin_storage::CommitMetadata::system(format!(
                            "Initialize global node type {}",
                            node_type.name.clone()
                        )),
                    )
                    .await?;
                    tracing::info!(
                        "Initialized global NodeType '{}' in {}/{}",
                        node_type.name,
                        repo_id,
                        branch
                    );
                }
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
    async fn test_init_global_nodetypes() {
        let tenant_id = "default";
        let repo_id = "main";
        let branch = "main";

        let storage = Arc::new(InMemoryStorage::default());

        // Initialize global NodeTypes
        init_global_nodetypes(storage.clone()).await.unwrap();

        // Verify all three NodeTypes were created
        let repo = storage.node_types();
        let tenant_id = "default";
        let repo_id = "main";
        let branch = "main";

        let folder = repo
            .get(
                BranchScope::new(tenant_id, repo_id, branch),
                "raisin:Folder",
                None,
            )
            .await
            .unwrap();
        assert!(folder.is_some());
        assert_eq!(folder.unwrap().name, "raisin:Folder");

        let page = repo
            .get(
                BranchScope::new(tenant_id, repo_id, branch),
                "raisin:Page",
                None,
            )
            .await
            .unwrap();
        assert!(page.is_some());
        assert_eq!(page.unwrap().name, "raisin:Page");

        let asset = repo
            .get(
                BranchScope::new(tenant_id, repo_id, branch),
                "raisin:Asset",
                None,
            )
            .await
            .unwrap();
        assert!(asset.is_some());
        assert_eq!(asset.unwrap().name, "raisin:Asset");
    }

    #[tokio::test]
    async fn test_version_update() {
        let storage = Arc::new(InMemoryStorage::default());
        let branch = "main";

        // Initialize multiple times to reach steady state.
        // The InMemoryNodeTypeRepo auto-increments versions on each upsert,
        // so the YAML version (e.g. 3) must be reached via repeated inits
        // before idempotency kicks in (YAML version <= stored version).
        for _ in 0..5 {
            init_global_nodetypes(storage.clone()).await.unwrap();
        }

        // Get stable version
        let stable = storage
            .node_types()
            .get(
                BranchScope::new("default", "main", branch),
                "raisin:Folder",
                None,
            )
            .await
            .unwrap()
            .unwrap();
        let stable_version = stable.version;

        // One more init should NOT update since stored version >= YAML version
        init_global_nodetypes(storage.clone()).await.unwrap();

        let after_extra_init = storage
            .node_types()
            .get(
                BranchScope::new("default", "main", branch),
                "raisin:Folder",
                None,
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(after_extra_init.version, stable_version);
    }
}
