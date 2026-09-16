//! Node property query and update operations
//!
//! This module provides functions for querying and updating node properties.

use super::super::helpers::{hash_property_value, is_tombstone};
use super::super::NodeRepositoryImpl;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;

impl NodeRepositoryImpl {
    /// Find nodes by property name and value
    pub(in crate::repositories::nodes) async fn find_by_property_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        property_name: &str,
        property_value: &PropertyValue,
    ) -> Result<Vec<Node>> {
        // Use PROPERTY_INDEX for efficient lookup
        let value_hash = hash_property_value(property_value);

        // Build prefix for this property+value across all revisions
        let prefix = keys::KeyBuilder::new()
            .push(tenant_id)
            .push(repo_id)
            .push(branch)
            .push(workspace)
            .push("prop") // Non-published properties
            .push(property_name)
            .push(&value_hash)
            .build_prefix();

        let cf_property = cf_handle(&self.db, cf::PROPERTY_INDEX)?;
        let prefix_clone = prefix.clone();
        let iter = crate::prefix_scan(&self.db, cf_property, prefix);

        let mut node_ids = std::collections::HashSet::new();

        // Collect unique node IDs (deduplicate across revisions)
        for item in iter {
            let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            // Verify key actually starts with our prefix
            if !key.starts_with(&prefix_clone) {
                break;
            }

            // Skip tombstones
            if is_tombstone(&value) {
                continue;
            }

            // Extract node_id from key (last component)
            let parts: Vec<&[u8]> = key.split(|&b| b == 0).collect();
            if let Some(node_id_bytes) = parts.last() {
                let node_id = String::from_utf8_lossy(node_id_bytes).to_string();
                node_ids.insert(node_id);
            }
        }

        // Fetch actual nodes
        let mut nodes = Vec::new();
        for node_id in node_ids {
            // Public API - populate has_children for frontend display
            if let Some(node) = self
                .get_impl(tenant_id, repo_id, branch, workspace, &node_id, true)
                .await?
            {
                // Double-check property value matches (hash collisions are possible).
                //
                // Compared by the SAME canonical rendering the index key is built
                // from, not by `PropertyValue` equality. The index already treats
                // `String("76133")` and `Decimal(76133)` as one key — the scan
                // above finds the node either way — so a variant comparison here
                // threw away rows the index had correctly returned.
                //
                // That is not theoretical. A decimal and a string are the same
                // bytes on the wire, so an all-digit string property can come
                // back from storage as a `Decimal`; looking it up with the
                // `String` the caller holds then matched nothing. Two lookups
                // that matter run exactly this way: `find_user_by_identity_id`
                // on `raisin:User.user_id` (declared String — and several OIDC
                // providers issue all-digit `sub` values), and the one-time-token
                // prefix lookup, which carries a sentinel character purely to
                // dodge this. The failure is silent and total: no user matches,
                // permissions resolve to nothing, and a read 404s as "not found"
                // rather than "identity lookup broke".
                let matches = node.properties.get(property_name).is_some_and(|stored| {
                    crate::repositories::nodes::helpers::hash_property_value(stored) == value_hash
                });
                if matches {
                    nodes.push(node);
                }
            }
        }

        Ok(nodes)
    }

    /// Find nodes with a specific property (any value)
    pub(in crate::repositories::nodes) async fn find_nodes_with_property_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        property_name: &str,
    ) -> Result<Vec<Node>> {
        // Use PROPERTY_INDEX for efficient lookup (prefix scan without value_hash)
        let prefix = keys::KeyBuilder::new()
            .push(tenant_id)
            .push(repo_id)
            .push(branch)
            .push(workspace)
            .push("prop") // Non-published properties
            .push(property_name)
            .build_prefix();

        let cf_property = cf_handle(&self.db, cf::PROPERTY_INDEX)?;
        let prefix_clone = prefix.clone();
        let iter = crate::prefix_scan(&self.db, cf_property, prefix);

        let mut node_ids = std::collections::HashSet::new();

        // Collect unique node IDs (deduplicate across revisions and values)
        for item in iter {
            let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            // Verify key actually starts with our prefix
            if !key.starts_with(&prefix_clone) {
                break;
            }

            // Skip tombstones
            if is_tombstone(&value) {
                continue;
            }

            // Extract node_id from key (last component)
            let parts: Vec<&[u8]> = key.split(|&b| b == 0).collect();
            if let Some(node_id_bytes) = parts.last() {
                let node_id = String::from_utf8_lossy(node_id_bytes).to_string();
                node_ids.insert(node_id);
            }
        }

        // Fetch actual nodes
        let mut nodes = Vec::new();
        for node_id in node_ids {
            // Public API - populate has_children for frontend display
            if let Some(node) = self
                .get_impl(tenant_id, repo_id, branch, workspace, &node_id, true)
                .await?
            {
                nodes.push(node);
            }
        }

        Ok(nodes)
    }

    /// Get property value by path
    pub(in crate::repositories::nodes) async fn get_property_by_path_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_path: &str,
        property_path: &str,
        max_revision: Option<&HLC>,
    ) -> Result<Option<PropertyValue>> {
        let node = self
            .get_by_path_impl(
                tenant_id,
                repo_id,
                branch,
                workspace,
                node_path,
                max_revision,
            )
            .await?
            .ok_or_else(|| raisin_error::Error::NotFound("Node not found".to_string()))?;

        Ok(node.properties.get(property_path).cloned())
    }

    /// Update property by path
    pub(in crate::repositories::nodes) async fn update_property_by_path_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_path: &str,
        property_path: &str,
        value: PropertyValue,
    ) -> Result<()> {
        // Always use HEAD for write operations (no max_revision)
        let mut node = self
            .get_by_path_impl(tenant_id, repo_id, branch, workspace, node_path, None)
            .await?
            .ok_or_else(|| raisin_error::Error::NotFound("Node not found".to_string()))?;

        node.properties.insert(property_path.to_string(), value);

        self.update_impl(
            tenant_id,
            repo_id,
            branch,
            workspace,
            node,
            crate::repositories::nodes::WriteAttribution::default(),
        )
        .await
    }
}

#[cfg(test)]
mod all_digit_lookup_tests {
    use raisin_models::nodes::properties::value::PropertyValue;
    use raisin_models::nodes::Node;
    use raisin_storage::{CreateNodeOptions, NodeRepository, Storage, StorageScope};
    use std::collections::HashMap;

    /// A storage with the branch the node repository writes into.
    ///
    /// Creating the branch is not optional: without it every `create` fails with
    /// `Branch 'main' not found`, which is also why the sibling fixture in
    /// `crud/read/list_operations.rs` has a failing test.
    async fn storage_with_branch(tmp: &tempfile::TempDir) -> crate::RocksDBStorage {
        use raisin_storage::BranchRepository;
        let storage = crate::RocksDBStorage::new(tmp.path()).unwrap();
        storage
            .branches()
            .create_branch("t", "r", "main", "test", None, None, false, false)
            .await
            .expect("create branch");
        storage
    }

    /// Store a node carrying `user_id`, exactly as `raisin:User` does.
    async fn create_user(storage: &crate::RocksDBStorage, id: &str, user_id: &str) {
        let mut properties = HashMap::new();
        properties.insert(
            "user_id".to_string(),
            PropertyValue::String(user_id.to_string()),
        );
        storage
            .nodes()
            .create(
                StorageScope::new("t", "r", "main", "users"),
                Node {
                    id: id.to_string(),
                    path: format!("/{id}"),
                    name: id.to_string(),
                    parent: Some("/".to_string()),
                    node_type: "raisin:User".to_string(),
                    properties,
                    ..Default::default()
                },
                CreateNodeOptions {
                    validate_schema: false,
                    validate_parent_allows_child: false,
                    validate_workspace_allows_type: false,
                    ..Default::default()
                },
            )
            .await
            .expect("create");
    }

    /// An identity whose subject is all digits must still be findable.
    ///
    /// This is the lookup behind `find_user_by_identity_id`, which the auth
    /// middleware runs for every session and the asset-grant reader runs for
    /// every signed URL. An all-digit string comes back from storage as a
    /// `Decimal` (a decimal and a string are the same bytes on the wire), and
    /// the post-scan double-check compared VARIANTS — so the row the index had
    /// correctly returned was thrown away, no user matched, permissions resolved
    /// to nothing, and the read 404'd as "not found" rather than "identity
    /// lookup broke". Several OIDC providers issue numeric `sub` values, so this
    /// is the ordinary case, not a corner.
    #[tokio::test]
    async fn an_all_digit_identity_is_found_by_its_string_value() {
        let tmp = tempfile::TempDir::new().unwrap();
        let storage = storage_with_branch(&tmp).await;

        // Canonical (indistinguishable from a decimal), a leading zero, and a
        // plainly non-numeric one for contrast.
        create_user(&storage, "u1", "123456789").await;
        create_user(&storage, "u2", "007").await;
        create_user(&storage, "u3", "auth0|abc").await;

        for (id, user_id) in [("u1", "123456789"), ("u2", "007"), ("u3", "auth0|abc")] {
            let found = storage
                .nodes()
                .find_by_property(
                    StorageScope::new("t", "r", "main", "users"),
                    "user_id",
                    &PropertyValue::String(user_id.to_string()),
                )
                .await
                .expect("lookup");
            assert_eq!(
                found.len(),
                1,
                "user_id {user_id:?} resolved to {} nodes — a session with this \
                 subject cannot sign in",
                found.len()
            );
            assert_eq!(found[0].id, id);
        }
    }

    /// The double-check still does its job: a different value must not match.
    #[tokio::test]
    async fn a_different_value_is_still_rejected() {
        let tmp = tempfile::TempDir::new().unwrap();
        let storage = storage_with_branch(&tmp).await;
        create_user(&storage, "u1", "123456789").await;

        let found = storage
            .nodes()
            .find_by_property(
                StorageScope::new("t", "r", "main", "users"),
                "user_id",
                &PropertyValue::String("987654321".to_string()),
            )
            .await
            .expect("lookup");
        assert!(found.is_empty(), "a different subject must not match");
    }
}

#[cfg(test)]
mod storage_round_trip_tests {
    use raisin_models::nodes::properties::value::PropertyValue;
    use raisin_models::nodes::Node;
    use raisin_storage::{
        BranchRepository, CreateNodeOptions, NodeRepository, Storage, StorageScope,
    };
    use std::collections::HashMap;

    /// A `String` written through the REAL storage encoder comes back a `String`.
    ///
    /// This is the whole fix, proved where it matters rather than at the type:
    /// every write door (node REST API, SQL, package install) funnels into this
    /// same `create` → MessagePack → read path, so if the variant survives here
    /// it survives for all of them. Before the self-describing form, a canonical
    /// all-digit string came back a `Decimal` and every
    /// `match ... Some(PropertyValue::String(s))` arm fell through.
    #[tokio::test]
    async fn a_numeric_string_survives_real_storage_as_a_string() {
        let tmp = tempfile::TempDir::new().unwrap();
        let storage = crate::RocksDBStorage::new(tmp.path()).unwrap();
        storage
            .branches()
            .create_branch("t", "r", "main", "test", None, None, false, false)
            .await
            .expect("create branch");

        let cases = [
            ("postal", "76133"),   // canonical: the case that used to flip
            ("jersey", "05"),      // leading zero
            ("duns", "123456789"), // always digits
            ("phone", "+41442345678"),
            ("plain", "hello"),
        ];
        let mut properties = HashMap::new();
        for (name, value) in cases {
            properties.insert(name.to_string(), PropertyValue::String(value.to_string()));
        }
        // A genuine decimal alongside them, to prove the other half still works.
        properties.insert(
            "rate".to_string(),
            PropertyValue::Decimal("19.90".parse().unwrap()),
        );

        storage
            .nodes()
            .create(
                StorageScope::new("t", "r", "main", "ws"),
                Node {
                    id: "n1".to_string(),
                    path: "/n1".to_string(),
                    name: "n1".to_string(),
                    parent: Some("/".to_string()),
                    node_type: "test:Thing".to_string(),
                    properties,
                    ..Default::default()
                },
                CreateNodeOptions {
                    validate_schema: false,
                    validate_parent_allows_child: false,
                    validate_workspace_allows_type: false,
                    ..Default::default()
                },
            )
            .await
            .expect("create");

        let back = storage
            .nodes()
            .get(StorageScope::new("t", "r", "main", "ws"), "n1", None)
            .await
            .expect("read")
            .expect("node exists");

        for (name, value) in cases {
            assert_eq!(
                back.properties.get(name),
                Some(&PropertyValue::String(value.to_string())),
                "{name} came back as {:?}, not the String it was written as",
                back.properties.get(name)
            );
        }
        assert_eq!(
            back.properties.get("rate"),
            Some(&PropertyValue::Decimal("19.90".parse().unwrap())),
            "a real decimal must still come back a Decimal"
        );
    }
}
