//! The deep readers (`deep_children_nested` / `_array` / `_flat`) answer
//! exactly what a per-node walk of `list_children` answers — same nodes, same
//! editorial order, same depth cut-off, same `has_children` — at HEAD and at a
//! past revision.
//!
//! The readers no longer scan `ORDERED_CHILDREN` per node: they take the
//! subtree's shape from PATH_INDEX, skip leaves and single children, and probe
//! only at the `max_depth` boundary. The oracle here is the slow path they
//! replaced, so any divergence in shape or order shows up as a diff.
//!
//! The tree mixes the cases that matter: a parent whose editorial order is NOT
//! creation order (a reorder), a single-child chain, leaves at several depths,
//! and a node moved AFTER the past revision (which the bulk scan used to drop
//! from every time-travel read, by honouring a tombstone newer than the bound).

use raisin_context::RepositoryConfig;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::{ChildrenField, DeepNode, Node, NodeWithChildren};
use raisin_rocksdb::RocksDBStorage;
use raisin_storage::{
    BranchRepository, CreateNodeOptions, ListOptions, NodeRepository, RegistryRepository,
    RepoScope, RepositoryManagementRepository, Storage, StorageScope, WorkspaceRepository,
};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use tempfile::TempDir;

const TENANT: &str = "deep-tenant";
const REPO: &str = "repo";
const BRANCH: &str = "main";
const WORKSPACE: &str = "default";

fn scope() -> StorageScope<'static> {
    StorageScope::new(TENANT, REPO, BRANCH, WORKSPACE)
}

async fn setup() -> Result<(RocksDBStorage, TempDir)> {
    let temp_dir = TempDir::new().unwrap();
    let storage = RocksDBStorage::new(temp_dir.path())?;
    storage
        .registry()
        .register_tenant(TENANT, HashMap::new())
        .await?;
    storage
        .repository_management()
        .create_repository(
            TENANT,
            REPO,
            RepositoryConfig {
                default_language: "en".to_string(),
                supported_languages: vec!["en".to_string()],
                locale_fallback_chains: HashMap::new(),
                default_branch: BRANCH.to_string(),
                description: None,
                tags: HashMap::new(),
            },
        )
        .await?;
    storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "system", None, None, false, false)
        .await?;
    storage
        .workspaces()
        .put(
            RepoScope::new(TENANT, REPO),
            raisin_models::workspace::Workspace::new(WORKSPACE.to_string()),
        )
        .await?;
    Ok((storage, temp_dir))
}

async fn create(storage: &RocksDBStorage, path: &str) -> Result<String> {
    let name = path.rsplit('/').next().unwrap().to_string();
    let parent = match path.rsplitn(2, '/').nth(1) {
        Some(p) if !p.is_empty() => Some(p.to_string()),
        _ => None,
    };
    let node = Node {
        id: uuid::Uuid::new_v4().to_string(),
        path: path.to_string(),
        name,
        parent,
        node_type: "raisin:Folder".to_string(),
        created_at: Some(chrono::Utc::now()),
        ..Node::default()
    };
    let id = node.id.clone();
    let options = CreateNodeOptions {
        validate_schema: false,
        validate_parent_allows_child: false,
        validate_workspace_allows_type: false,
        operation_meta: None,
    };
    storage.nodes().create(scope(), node, options).await?;
    Ok(id)
}

/// One node of a tree, comparable across the readers and the oracle.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct T {
    path: String,
    has_children: Option<bool>,
    children: Vec<T>,
}

/// The oracle: `list_children` per node, `levels` levels deep.
fn walk<'a>(
    storage: &'a RocksDBStorage,
    path: String,
    levels: u32,
    at: Option<HLC>,
) -> Pin<Box<dyn Future<Output = Result<Vec<T>>> + 'a>> {
    Box::pin(async move {
        let options = ListOptions {
            compute_has_children: true,
            max_revision: at,
            skip_properties: false,
        };
        let mut out = Vec::new();
        for child in storage
            .nodes()
            .list_children(scope(), &path, options)
            .await?
        {
            let children = if levels > 1 {
                walk(storage, child.path.clone(), levels - 1, at).await?
            } else {
                Vec::new()
            };
            out.push(T {
                path: child.path,
                has_children: child.has_children,
                children,
            });
        }
        Ok(out)
    })
}

fn from_array(nodes: Vec<NodeWithChildren>) -> Vec<T> {
    nodes
        .into_iter()
        .map(|n| T {
            path: n.node.path,
            has_children: n.node.has_children,
            children: match n.children {
                ChildrenField::Nodes(kids) => from_array(kids.into_iter().map(|k| *k).collect()),
                ChildrenField::Names(_) => panic!("no node is cut off by name within max_depth"),
            },
        })
        .collect()
}

/// Nested output is a map, so both sides are compared in path order.
fn from_nested(nodes: HashMap<String, DeepNode>) -> Vec<T> {
    let mut out: Vec<T> = nodes
        .into_values()
        .map(|n| T {
            path: n.node.path,
            has_children: n.node.has_children,
            children: from_nested(n.children),
        })
        .collect();
    out.sort();
    out
}

fn sorted(mut nodes: Vec<T>) -> Vec<T> {
    for node in &mut nodes {
        node.children = sorted(std::mem::take(&mut node.children));
    }
    nodes.sort();
    nodes
}

/// Pre-order paths, the flat reader's output.
fn preorder(nodes: &[T], out: &mut Vec<String>) {
    for node in nodes {
        out.push(node.path.clone());
        preorder(&node.children, out);
    }
}

async fn assert_readers_match_oracle(
    storage: &RocksDBStorage,
    parent: &str,
    at: Option<HLC>,
) -> Result<()> {
    let nodes = storage.nodes();
    for depth in [1u32, 2, 3, 10] {
        let expected = walk(storage, parent.to_string(), depth, at).await?;

        let array = nodes
            .deep_children_array(scope(), parent, depth, at.as_ref())
            .await?;
        assert_eq!(
            from_array(array),
            expected,
            "array, parent={parent} depth={depth} at={at:?}"
        );

        let nested = nodes
            .deep_children_nested(scope(), parent, depth, at.as_ref())
            .await?;
        assert_eq!(
            from_nested(nested),
            sorted(expected.clone()),
            "nested, parent={parent} depth={depth} at={at:?}"
        );

        // Flat reaches one level further than the tree readers.
        let flat_expected = walk(storage, parent.to_string(), depth + 1, at).await?;
        let mut expected_paths = Vec::new();
        preorder(&flat_expected, &mut expected_paths);
        let flat: Vec<String> = nodes
            .deep_children_flat(scope(), parent, depth, at.as_ref())
            .await?
            .into_iter()
            .map(|n| n.path)
            .collect();
        assert_eq!(
            flat, expected_paths,
            "flat, parent={parent} depth={depth} at={at:?}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn deep_readers_match_a_per_node_walk_at_head_and_in_the_past() -> Result<()> {
    let (storage, _tmp) = setup().await?;
    create(&storage, "/root-a").await?;
    create(&storage, "/root-a/c1").await?;
    create(&storage, "/root-a/c2").await?;
    create(&storage, "/root-a/c3").await?;
    create(&storage, "/root-a/c1/only").await?;
    create(&storage, "/root-a/c1/only/leaf-deep").await?;
    create(&storage, "/root-a/c1/only/leaf-deep/deeper").await?;
    let moved = create(&storage, "/root-a/c2/moved").await?;
    create(&storage, "/root-b").await?;
    let past = storage.branches().get_head(TENANT, REPO, BRANCH).await?;

    // After `past`: editorial order stops being creation order, and a node
    // changes parent.
    storage
        .nodes()
        .move_child_before(scope(), "/root-a", "c3", "c1", None, None)
        .await?;
    storage
        .nodes()
        .move_node(scope(), &moved, "/root-b/moved", None)
        .await?;

    for parent in ["/", "/root-a", "/root-a/c1"] {
        assert_readers_match_oracle(&storage, parent, None).await?;
        assert_readers_match_oracle(&storage, parent, Some(past)).await?;
    }

    // And the reorder really is visible, so the oracle compared something.
    let head = walk(&storage, "/root-a".to_string(), 1, None).await?;
    let order: Vec<&str> = head.iter().map(|t| t.path.as_str()).collect();
    assert_eq!(order, ["/root-a/c3", "/root-a/c1", "/root-a/c2"]);
    Ok(())
}

fn flatten(nodes: Vec<NodeWithChildren>, out: &mut Vec<Node>) {
    for n in nodes {
        if let ChildrenField::Nodes(kids) = n.children {
            flatten(kids.into_iter().map(|k| *k).collect(), out);
        }
        out.push(n.node);
    }
}

/// Every node a deep read returns is that node's NEWEST version at or below
/// the read revision — what a point read returns — not the version current
/// when its PATH_INDEX entry was last written. A property edit and a reorder
/// (which restamps `order_key` in a new node revision) both move a node's
/// version without moving its path.
#[tokio::test]
async fn deep_readers_return_each_nodes_newest_version() -> Result<()> {
    use raisin_models::nodes::properties::PropertyValue;

    let (storage, _tmp) = setup().await?;
    create(&storage, "/doc-root").await?;
    let a = create(&storage, "/doc-root/a").await?;
    create(&storage, "/doc-root/b").await?;
    storage
        .nodes()
        .update_property_by_path(
            scope(),
            "/doc-root/a",
            "title",
            PropertyValue::String("v1".into()),
        )
        .await?;
    let past = storage.branches().get_head(TENANT, REPO, BRANCH).await?;

    storage
        .nodes()
        .update_property_by_path(
            scope(),
            "/doc-root/a",
            "title",
            PropertyValue::String("v2".into()),
        )
        .await?;
    storage
        .nodes()
        .move_child_before(scope(), "/doc-root", "b", "a", None, None)
        .await?;

    for at in [None, Some(past)] {
        let mut returned = Vec::new();
        flatten(
            storage
                .nodes()
                .deep_children_array(scope(), "/", 3, at.as_ref())
                .await?,
            &mut returned,
        );
        assert_eq!(returned.len(), 3, "at={at:?}");
        for node in returned {
            let point = storage
                .nodes()
                .get(scope(), &node.id, at.as_ref())
                .await?
                .expect("every returned node is readable at the same revision");
            assert_eq!(node.properties, point.properties, "{} at={at:?}", node.path);
            assert_eq!(node.order_key, point.order_key, "{} at={at:?}", node.path);
            if node.id == a {
                let expected = if at.is_some() { "v1" } else { "v2" };
                assert_eq!(
                    node.properties.get("title"),
                    Some(&PropertyValue::String(expected.into())),
                    "at={at:?}"
                );
            }
        }
    }
    Ok(())
}

/// `scan_by_path_prefix` returns each node at its newest version at or below
/// the read revision — matching a point read on properties, `order_key` and
/// `has_children` — not the version (and child set) current when the node's
/// PATH_INDEX entry was written.
#[tokio::test]
async fn path_prefix_scan_returns_each_nodes_newest_version() -> Result<()> {
    use raisin_models::nodes::properties::PropertyValue;

    let (storage, _tmp) = setup().await?;
    create(&storage, "/pfx").await?;
    let a = create(&storage, "/pfx/a").await?;
    create(&storage, "/pfx/b").await?;
    storage
        .nodes()
        .update_property_by_path(
            scope(),
            "/pfx/a",
            "title",
            PropertyValue::String("v1".into()),
        )
        .await?;
    let past = storage.branches().get_head(TENANT, REPO, BRANCH).await?;

    storage
        .nodes()
        .update_property_by_path(
            scope(),
            "/pfx/a",
            "title",
            PropertyValue::String("v2".into()),
        )
        .await?;
    storage
        .nodes()
        .move_child_before(scope(), "/pfx", "b", "a", None, None)
        .await?;
    // A child added after every write that touched a's path entry.
    create(&storage, "/pfx/a/kid").await?;

    for at in [None, Some(past)] {
        let options = ListOptions {
            compute_has_children: true,
            max_revision: at,
            skip_properties: false,
        };
        let scanned = storage
            .nodes()
            .scan_by_path_prefix(scope(), "/pfx/", options)
            .await?;
        let expected_count = if at.is_some() { 2 } else { 3 };
        assert_eq!(scanned.len(), expected_count, "at={at:?}");
        for node in scanned {
            let point = storage
                .nodes()
                .get(scope(), &node.id, at.as_ref())
                .await?
                .expect("every scanned node is readable at the same revision");
            assert_eq!(node.properties, point.properties, "{} at={at:?}", node.path);
            assert_eq!(node.order_key, point.order_key, "{} at={at:?}", node.path);
            assert_eq!(
                node.has_children, point.has_children,
                "{} at={at:?}",
                node.path
            );
            if node.id == a {
                let (title, has_kid) = if at.is_some() {
                    ("v1", false)
                } else {
                    ("v2", true)
                };
                assert_eq!(
                    node.properties.get("title"),
                    Some(&PropertyValue::String(title.into())),
                    "at={at:?}"
                );
                assert_eq!(node.has_children, Some(has_kid), "at={at:?}");
            }
        }
    }
    Ok(())
}
