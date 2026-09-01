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
    // Memoized: the input is compiled INTO the binary, so the answer cannot
    // change while the process lives. It was being recomputed — an MD5 over
    // every embedded NodeType YAML — on every HTTP request, because
    // `ensure_tenant_middleware` calls this to decide whether re-initialization
    // is needed. Constant work on the hot path of every request.
    static VERSION: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    VERSION
        .get_or_init(|| {
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
        })
        .clone()
}

/// Load the effective global NodeType definitions.
///
/// Resolves through the process-wide definition stack, so an overlay definition
/// is what every caller sees. Reading the embedded YAML directly here would let
/// the legacy init paths overwrite an overlay definition with the built-in one.
pub fn load_global_nodetypes() -> Vec<NodeType> {
    load_global_nodetypes_with_hashes()
        .into_iter()
        .map(|(nt, _)| nt)
        .collect()
}

/// Load the effective global NodeType definitions with their content hashes.
///
/// Resolved through the process-wide definition stack (see
/// [`crate::definitions`]). Use [`load_embedded_nodetypes_with_hashes`] when you
/// specifically need the binary's own copies.
pub fn load_global_nodetypes_with_hashes() -> Vec<(NodeType, String)> {
    crate::definitions::global_resolver().nodetypes()
}

/// Load NodeType definitions straight from the embedded YAML.
///
/// This is the raw baseline layer. It must not consult the resolver — the
/// resolver's own embedded layer is built from it.
///
/// # Returns
/// A vector of tuples containing (NodeType, content_hash)
pub fn load_embedded_nodetypes_with_hashes() -> Vec<(NodeType, String)> {
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

    /// The loader logs-and-skips a YAML file it cannot deserialize, so a typo in
    /// one of these would silently drop the type rather than fail the build.
    /// Assert the connector-config types load and that `raisin:Integration`
    /// carries the schema-driven-config revision.
    #[test]
    fn connector_config_nodetypes_are_loadable() {
        let nodetypes = load_global_nodetypes();
        let by_name = |n: &str| nodetypes.iter().find(|nt| nt.name == n).cloned();

        for name in ["raisin:ConnectorConfig", "raisin:ConnectionConfig"] {
            let nt = by_name(name).unwrap_or_else(|| panic!("{name} failed to load"));
            // Never instantiated as nodes; strictness would only trap subtypes
            // that legitimately carry provider-specific keys.
            assert_eq!(nt.strict, Some(false), "{name} must not be strict");
        }

        // The base connection type carries `label`, and its UI hints must survive
        // deserialization — PropertyValueSchema drops top-level title/description,
        // which is exactly why they live under `meta`.
        let conn = by_name("raisin:ConnectionConfig").expect("loaded above");
        let label = conn
            .properties
            .as_ref()
            .and_then(|ps| ps.iter().find(|p| p.name.as_deref() == Some("label")))
            .expect("raisin:ConnectionConfig should declare `label`");
        assert!(
            label.meta.as_ref().is_some_and(|m| m.contains_key("label")),
            "UI hints must be carried in `meta` to survive the round trip"
        );

        let integration = by_name("raisin:Integration").expect("raisin:Integration must load");
        // raisin:Integration is strict, so each property below must be declared or a
        // connector using it is rejected on write — and adding one is a schema change
        // that must bump the version, or existing repos never resync.
        assert_eq!(integration.version, Some(5));
        for prop in [
            "config_type",
            "connection_config_type",
            "oauth_config_type",
            "config",
            "config_secrets_encrypted",
            "config_secret_fields",
            // v4: connector-authored mount presets, read by the admin console only.
            // v5 added `prompts` and per-entry `target_workspace`/`root_override`
            // INSIDE that array, which needs no new property here — the version bump
            // is what makes existing repos resync the description operators read.
            "mount_bundles",
        ] {
            assert!(
                integration
                    .properties
                    .as_ref()
                    .is_some_and(|ps| ps.iter().any(|p| p.name.as_deref() == Some(prop))),
                "raisin:Integration is strict, so `{prop}` must be declared or writes are rejected"
            );
        }
    }

    /// Assert the MCP *client* types load: the connection itself, and the
    /// `mcp_proxy` marker on `raisin:Function`.
    #[test]
    fn mcp_client_nodetypes_are_loadable() {
        let nodetypes = load_global_nodetypes();
        let by_name = |n: &str| nodetypes.iter().find(|nt| nt.name == n).cloned();

        let conn = by_name("raisin:McpConnection").expect("raisin:McpConnection must load");
        let prop = |name: &str| {
            conn.properties
                .as_ref()
                .and_then(|ps| ps.iter().find(|p| p.name.as_deref() == Some(name)))
                .unwrap_or_else(|| panic!("raisin:McpConnection must declare `{name}`"))
        };
        for name in [
            "slug",
            "url",
            "auth_kind",
            "credential_encrypted",
            "tokens_encrypted",
            "discovered_tools",
            "health",
        ] {
            let _ = prop(name);
        }
        // The slug namespaces every generated proxy name; two connections
        // claiming the same one would collide on raisin:Function.name, which
        // IS enforced, and the second connection's discovery would hard-fail.
        assert_eq!(
            prop("slug").unique,
            Some(true),
            "`slug` must be unique or two connections can generate the same proxy names"
        );
        // Discovery rewrites health/last_synced_at on every refresh; keeping
        // this versionable mints a revision per refresh, forever.
        assert_eq!(conn.versionable, Some(false));
        // UI hints must live under `meta` — PropertyValueSchema drops
        // top-level title/description/enum/placeholder.
        assert!(
            prop("auth_kind")
                .meta
                .as_ref()
                .is_some_and(|m| m.contains_key("enum")),
            "UI hints must be carried in `meta` to survive the round trip"
        );

        // raisin:Function is strict, so an undeclared `mcp_proxy` makes every
        // generated proxy write fail with a strict_mode_violation pointing at
        // the node rather than at the missing definition.
        let function = by_name("raisin:Function").expect("raisin:Function must load");
        assert!(
            function.version.unwrap_or(0) >= 2,
            "adding `mcp_proxy` is a schema change and must bump the version to resync \
             (a floor, not an equality: a later additive bump is fine)"
        );
        assert!(
            function
                .properties
                .as_ref()
                .is_some_and(|ps| ps.iter().any(|p| p.name.as_deref() == Some("mcp_proxy"))),
            "raisin:Function is strict, so `mcp_proxy` must be declared or proxy writes are rejected"
        );
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

    /// Guards the raisin:InboxTask schema so a STANDALONE task
    /// (`raisin.tasks.create`, which has no owning flow) validates. Previously
    /// flow_instance_ref + step_id were `required: true`, so every standalone
    /// create failed schema validation with "Missing required property
    /// 'flow_instance_ref'". Because the type is `strict: true`, it must also
    /// declare every property the writers actually set.
    #[test]
    fn test_inbox_task_schema_allows_standalone_tasks() {
        let nodetypes = load_global_nodetypes();
        let inbox = nodetypes
            .iter()
            .find(|nt| nt.name == "raisin:InboxTask")
            .expect("raisin_inbox_task.yaml should load");

        let props = inbox
            .properties
            .as_ref()
            .expect("InboxTask should declare properties");
        let by_name = |n: &str| props.iter().find(|p| p.name.as_deref() == Some(n));
        let is_required = |n: &str| by_name(n).and_then(|p| p.required).unwrap_or(false);

        // Core identity fields stay required.
        assert!(is_required("task_type"), "task_type must stay required");
        assert!(is_required("title"), "title must stay required");
        assert!(is_required("status"), "status must stay required");

        // Flow linkage must be OPTIONAL — a standalone task has no flow.
        assert!(
            !is_required("flow_instance_ref"),
            "flow_instance_ref must be optional so standalone tasks validate"
        );
        assert!(
            !is_required("step_id"),
            "step_id must be optional so standalone tasks validate"
        );

        // strict:true → every property any writer sets must be declared, else
        // the write is rejected as an undefined property.
        for name in [
            "assignee",
            "flow_instance_id",
            "due_at",
            "due_in_seconds",
            "created_at",
            "completed_by",
        ] {
            assert!(
                by_name(name).is_some(),
                "InboxTask must declare '{}' (a property its writers set under strict mode)",
                name
            );
        }
    }

    /// The raisin:Event v2 reshape, plus the mixin that carries the engine-owned
    /// reserved properties.
    ///
    /// Two things this guards that nothing else would notice:
    /// - the VERSION BUMP. Both legacy init paths update an existing NodeType
    ///   only when `new_version > existing_version`, so shipping the reshape at
    ///   version 1 would leave every existing repository on the old schema with
    ///   no error anywhere.
    /// - the RENAME of `start`/`end` to `start_utc`/`end_utc`. Reusing the old
    ///   names would leave stale nodes holding a naive-local or offset-bearing
    ///   value in a column whose new contract is a fixed-width UTC instant —
    ///   silently wrong ordering rather than a visible NULL.
    #[test]
    fn event_v2_is_the_provider_neutral_calendar_model() {
        let nodetypes = load_global_nodetypes();
        let by_name = |n: &str| nodetypes.iter().find(|nt| nt.name == n).cloned();

        let mixin = by_name("raisin:VirtualNode").expect("raisin_virtual_node.yaml must load");
        assert_eq!(mixin.is_mixin, Some(true), "must be usable as a mixin");
        let mixin_props = mixin
            .properties
            .as_ref()
            .expect("mixin declares properties");
        for name in [
            "__virtual",
            "__mount_id",
            "__external_id",
            "__etag",
            "__synced_at",
            // The one raisin:Event omitted while the materializer stamped it.
            "__pushed_state",
        ] {
            assert!(
                mixin_props.iter().any(|p| p.name.as_deref() == Some(name)),
                "raisin:VirtualNode must declare '{name}'"
            );
        }

        let event = by_name("raisin:Event").expect("raisin_event.yaml must load");
        assert!(
            event.version.unwrap_or(0) >= 2,
            "the reshape must bump the version or the legacy init paths skip it \
             (a floor, not an equality: a later additive bump is fine)"
        );
        assert_eq!(event.strict, Some(true));
        assert_eq!(event.mixins, vec!["raisin:VirtualNode".to_string()]);

        let props = event
            .properties
            .as_ref()
            .expect("Event declares properties");
        let has = |n: &str| props.iter().any(|p| p.name.as_deref() == Some(n));

        for name in [
            "start_utc",
            "end_utc",
            "start_local",
            "end_local",
            "timezone",
            "recurrence_type",
            "series_master_external_id",
            "original_start_utc",
            "show_as",
            "my_response",
            "organizer_email",
            "organizer_name",
            "ical_uid",
            "location_geo",
        ] {
            assert!(has(name), "raisin:Event v2 must declare '{name}'");
        }

        // Renamed / split, so the old names must be GONE — a strict type that
        // still declared them would let a stale-shaped write through.
        for name in ["start", "end", "organizer"] {
            assert!(!has(name), "'{name}' was renamed in v2 and must not remain");
        }

        // The reserved properties come from the mixin. Re-declaring them here
        // would fork the definition and let the two drift.
        for name in ["__virtual", "__pushed_state"] {
            assert!(
                !has(name),
                "'{name}' belongs to raisin:VirtualNode, not to raisin:Event"
            );
        }

        // recurrence is RFC 5545 content LINES now, not a provider blob.
        let recurrence = props
            .iter()
            .find(|p| p.name.as_deref() == Some("recurrence"))
            .expect("recurrence must exist");
        assert_eq!(
            recurrence.property_type,
            raisin_models::nodes::properties::schema::PropertyType::Array
        );

        // UI hints must live under `meta` — PropertyValueSchema silently drops
        // top-level title/description/enum.
        let status = props
            .iter()
            .find(|p| p.name.as_deref() == Some("status"))
            .expect("status must exist");
        assert!(
            status.meta.as_ref().is_some_and(|m| m.contains_key("enum")),
            "status must carry its enum under `meta` to survive deserialization"
        );
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

    /// The raisin:Mail v2 extension, and the two changes that would otherwise be
    /// silent.
    ///
    /// - the VERSION BUMP. Both legacy init paths update an existing NodeType
    ///   only when `new_version > existing_version`, so shipping this at
    ///   version 1 would leave every existing repository on the old schema with
    ///   no error anywhere.
    /// - `allowed_children`. It was `[]`, which makes an attachment child a
    ///   VALIDATION failure at write time — so the whole attachment feature is
    ///   inert until raisin:Asset is listed here.
    #[test]
    fn mail_v2_carries_the_write_path_fields_and_allows_asset_children() {
        let nodetypes = load_global_nodetypes();
        let by_name = |n: &str| nodetypes.iter().find(|nt| nt.name == n).cloned();

        let mail = by_name("raisin:Mail").expect("raisin_mail.yaml must load");
        assert!(
            mail.version.unwrap_or(0) >= 2,
            "the reshape must bump the version (a floor, not an equality)"
        );

        // The Vector leg. Mail carried only the lexical leg, which biases
        // reciprocal-rank fusion toward whichever corpus is wider -- and left
        // cross-lingual retrieval, the reason a multilingual embedder is
        // configured at all, unreachable for a mailbox.
        assert!(
            mail.index_types
                .as_ref()
                .is_some_and(|t| t.iter().any(|i| format!("{i:?}") == "Vector")),
            "raisin:Mail must declare the Vector index type"
        );
        let props = mail.properties.as_ref().expect("Mail declares properties");
        let has = |n: &str| props.iter().any(|p| p.name.as_deref() == Some(n));

        // `body` + `body_type` REPLACED: one column whose meaning depended on a
        // sibling column could not be searched or replied to without branching.
        assert!(has("body_html") && has("body_text"), "the body split");
        assert!(
            !has("body") && !has("body_type"),
            "the ambiguous pair must be gone, not left alongside its replacement"
        );

        for name in [
            "bcc",
            "reply_to",
            "in_reply_to",
            "references",
            "thread_id",
            "flags",
            "labels",
            "size",
            "sent_at",
            "received_at",
            "is_draft",
        ] {
            assert!(has(name), "raisin:Mail v2 must declare '{name}'");
        }

        // Attachments are raisin:Asset CHILDREN. With `allowed_children: []`
        // every attachment write fails validation and the message still lands,
        // so the failure is one WARN per attachment and an empty subtree.
        assert_eq!(
            mail.allowed_children,
            vec!["raisin:Asset".to_string()],
            "raisin:Mail must allow raisin:Asset children"
        );

        // Non-strict, deliberately: a mapper carries provider-specific keys and
        // the engine stamps reserved `__` properties. The mixin documents and
        // indexes those without closing the type.
        assert_ne!(mail.strict, Some(true), "raisin:Mail must stay non-strict");
        assert!(
            mail.mixins.iter().any(|m| m == "raisin:VirtualNode"),
            "mount-written types declare the reserved properties through the mixin"
        );
    }

    /// `raisin:Asset.file` must be OPTIONAL, or a metadata-only attachment
    /// cannot exist.
    ///
    /// Sync writes attachment metadata and no bytes; `get_content` fills `file`
    /// in later, on demand. With `required: true` the only alternatives are
    /// downloading every attachment of every message during the sync hot loop,
    /// or writing a placeholder Resource whose `storage_key` resolves to
    /// nothing — which is worse, because every consumer reads presence as "the
    /// bytes are there".
    #[test]
    fn asset_file_is_optional_so_an_unfetched_attachment_can_exist() {
        let nodetypes = load_global_nodetypes();
        let asset = nodetypes
            .iter()
            .find(|nt| nt.name == "raisin:Asset")
            .expect("raisin_asset.yaml must load");
        // `>=`, not `==`. This assertion is about the widening that made `file`
        // optional (version 3); pinning it exactly turns every LATER, unrelated
        // bump into a failure of a test that has nothing to say about the
        // change — which is what happened between 3 and 5, leaving a red test
        // that named the wrong cause.
        assert!(
            asset.version.unwrap_or(0) >= 3,
            "the widening must bump the version (got {:?})",
            asset.version
        );
        let props = asset
            .properties
            .as_ref()
            .expect("Asset declares properties");
        let by_name = |n: &str| props.iter().find(|p| p.name.as_deref() == Some(n));

        assert_eq!(
            by_name("file").and_then(|p| p.required),
            Some(false),
            "`file == null` must be representable: it means 'not fetched yet'"
        );
        assert!(
            by_name("inline").is_some() && by_name("content_id").is_some(),
            "inline images need `inline` + `content_id` to resolve `cid:` refs in body_html"
        );
    }

    /// A caption must be SEMANTICALLY searchable, not merely lexically.
    ///
    /// `description` / `alt_text` / `keywords` are the only text an IMAGE ever
    /// has: the binary contributes nothing to `__extracted_text`, so the `doc`
    /// spec is empty for a photo and the node's own vector is the only one it
    /// gets. Without `Vector` on these three, everything a captioning trigger
    /// writes is reachable only by the literal words someone typed — the
    /// feature would produce captions nobody can find by meaning.
    ///
    /// `alt_text` is called out separately because it had no `index:` key at
    /// all, which is a quieter bug than a wrong one: absent reads as "not
    /// indexed for anything", so it was invisible to BOTH engines.
    ///
    /// This is the whole core-side contract behind the captioning boundary
    /// decision (Studio picks the model and the prompt; core turns the text it
    /// writes into a vector), so it is asserted rather than assumed.
    #[test]
    fn asset_caption_fields_are_vector_indexed() {
        use raisin_models::nodes::properties::schema::IndexType;

        let nodetypes = load_global_nodetypes();
        let asset = nodetypes
            .iter()
            .find(|nt| nt.name == "raisin:Asset")
            .expect("raisin_asset.yaml must load");
        let props = asset
            .properties
            .as_ref()
            .expect("Asset declares properties");

        for field in ["description", "alt_text", "keywords"] {
            let prop = props
                .iter()
                .find(|p| p.name.as_deref() == Some(field))
                .unwrap_or_else(|| panic!("raisin:Asset must declare `{field}`"));
            let index = prop
                .index
                .as_ref()
                .unwrap_or_else(|| panic!("`{field}` must declare an `index:` list"));
            assert!(
                index.contains(&IndexType::Vector),
                "`{field}` must be Vector-indexed or a caption written into it is \
                 semantically invisible; got {index:?}"
            );
            assert!(
                index.contains(&IndexType::Fulltext),
                "`{field}` must stay Fulltext-indexed: an exact-word search for a \
                 caption must keep working; got {index:?}"
            );
        }

        // The type-level gate. `index_settings.vector` is read from THIS list,
        // and it is what decides whether an `EmbeddingGenerate` job is enqueued
        // at all — per-property `Vector` flags are inert without it.
        assert!(
            asset
                .index_types
                .as_ref()
                .is_some_and(|t| t.contains(&IndexType::Vector)),
            "raisin:Asset must be Vector-indexable at the type level, or no \
             embedding job is ever enqueued for an asset"
        );
    }

    /// The two `submit` command types, and the three declarations that make
    /// them safe.
    ///
    /// Each one is load-bearing rather than tidy:
    ///
    /// 1. **`status` defaults to `draft`.** The drain picks up `queued` and
    ///    nothing else, so a command created without a status must land
    ///    somewhere inert. Default it to `queued` — or leave it absent on a
    ///    type whose engine treated absence as ready — and an agent that
    ///    creates one of these to fill in later has sent an email.
    /// 2. **The `raisin:VirtualNode` mixin, carrying `__write_seq`.** That
    ///    counter is the durable half of the at-most-once claim. Without the
    ///    declaration a strict consumer of these types rejects the very write
    ///    that records a send as having happened.
    /// 3. **`auditable: true`**, unlike raisin:Mail. Inbound mail's audit log
    ///    would mirror the sync job; an outbound command is an irreversible act
    ///    taken on a person's behalf, and who queued it is the question.
    #[test]
    fn submit_command_types_default_to_draft_and_declare_the_write_seq_guard() {
        let nodetypes = load_global_nodetypes();
        let by_name = |n: &str| nodetypes.iter().find(|nt| nt.name == n).cloned();

        // The reserved counter is declared ONCE, on the mixin — not on whichever
        // concrete type happened to need it first.
        let mixin = by_name("raisin:VirtualNode").expect("raisin_virtual_node.yaml must load");
        assert!(
            mixin
                .properties
                .as_ref()
                .expect("the mixin declares properties")
                .iter()
                .any(|p| p.name.as_deref() == Some("__write_seq")),
            "the at-most-once guard must be declared on the mixin"
        );

        for name in ["raisin:OutboundMail", "raisin:CalendarAction"] {
            let nt = by_name(name).unwrap_or_else(|| panic!("{name} must load"));
            let props = nt
                .properties
                .as_ref()
                .unwrap_or_else(|| panic!("{name} declares properties"));
            let by_prop = |p: &str| props.iter().find(|x| x.name.as_deref() == Some(p)).cloned();

            let status = by_prop("status").unwrap_or_else(|| panic!("{name}.status"));
            assert_eq!(
                status.default,
                Some(raisin_models::nodes::properties::PropertyValue::String(
                    "draft".to_string()
                )),
                "{name}.status must default to 'draft' — creating a command must not send it"
            );
            assert_eq!(
                by_prop("action").and_then(|p| p.required),
                Some(true),
                "{name} must name what it does"
            );
            assert!(
                nt.mixins.iter().any(|m| m == "raisin:VirtualNode"),
                "{name} must inherit the engine-owned reserved properties"
            );
            assert_eq!(
                nt.auditable,
                Some(true),
                "{name} records an irreversible act on someone's behalf"
            );
        }

        // Attachments on an outbound mail are raisin:Asset CHILDREN, exactly as
        // they are inbound: ONE convention for "a blob hanging off a message",
        // in both directions.
        assert_eq!(
            by_name("raisin:OutboundMail").unwrap().allowed_children,
            vec!["raisin:Asset".to_string()],
        );
    }
}
