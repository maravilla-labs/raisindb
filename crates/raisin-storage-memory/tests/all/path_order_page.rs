//! The default `scan_by_path_order_page`, which a backend without an ordered
//! path index (this one) inherits: byte-wise path order, an exclusive cursor in
//! both directions, and `next_after` as the only end-of-scan signal.

use raisin_models::nodes::Node;
use raisin_storage::{CreateNodeOptions, ListOptions, NodeRepository, StorageScope};
use raisin_storage_memory::InMemoryNodeRepo;

const WS: &str = "test-ws";

fn scope() -> StorageScope<'static> {
    StorageScope::new("default", "default", "main", WS)
}

async fn repo_with(paths: &[&str]) -> InMemoryNodeRepo {
    let repo = InMemoryNodeRepo::new();
    for (i, path) in paths.iter().enumerate() {
        let (parent, name) = path.rsplit_once('/').unwrap();
        let node = Node {
            id: format!("n{i}"),
            name: name.to_string(),
            path: path.to_string(),
            node_type: "document".to_string(),
            workspace: Some(WS.to_string()),
            parent: Some(if parent.is_empty() { "/" } else { parent }.to_string()),
            ..Default::default()
        };
        let opts = CreateNodeOptions {
            validate_schema: false,
            validate_parent_allows_child: false,
            validate_workspace_allows_type: false,
            operation_meta: None,
        };
        repo.create(scope(), node, opts).await.unwrap();
    }
    repo
}

async fn walk(repo: &InMemoryNodeRepo, descending: bool, page: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let p = repo
            .scan_by_path_order_page(
                scope(),
                cursor.as_deref(),
                page,
                descending,
                ListOptions::for_sql(),
            )
            .await
            .unwrap();
        assert!(p.nodes.len() <= page);
        out.extend(p.nodes.into_iter().map(|n| n.path));
        match p.next_after {
            Some(next) => cursor = Some(next),
            None => return out,
        }
    }
}

#[tokio::test]
async fn default_pages_in_byte_wise_path_order() {
    // Inserted in tree order; `/a-b` sorts before `/a/x` in path order.
    let repo = repo_with(&["/a", "/a/x", "/a-b", "/z"]).await;
    let asc = vec!["/a", "/a-b", "/a/x", "/z"];
    for page in [1, 2, 3, 10] {
        assert_eq!(walk(&repo, false, page).await, asc, "page size {page}");
        let desc: Vec<_> = asc.iter().rev().copied().collect();
        assert_eq!(walk(&repo, true, page).await, desc, "page size {page}");
    }
}
