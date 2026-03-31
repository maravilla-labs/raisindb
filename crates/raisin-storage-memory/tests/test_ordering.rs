use raisin_models::nodes::Node;
use raisin_storage::{CreateNodeOptions, ListOptions, NodeRepository, StorageScope};
use raisin_storage_memory::InMemoryNodeRepo;

fn relaxed_create_opts() -> CreateNodeOptions {
    CreateNodeOptions {
        validate_schema: false,
        validate_parent_allows_child: false,
        validate_workspace_allows_type: false,
        operation_meta: None,
    }
}

#[tokio::test]
async fn test_order_preservation() -> Result<(), Box<dyn std::error::Error>> {
    // Create an in-memory repository
    let repo = InMemoryNodeRepo::new();

    // Create workspace
    let workspace = "test-ws";

    // Create a parent node
    let mut parent = Node::default();
    parent.id = "parent-1".to_string();
    parent.name = "parent".to_string();
    parent.path = "/parent".to_string();
    parent.node_type = "folder".to_string();
    parent.workspace = Some(workspace.to_string());
    parent.parent = None;

    repo.create(
        StorageScope::new("default", "default", "main", workspace),
        parent.clone(),
        relaxed_create_opts(),
    )
    .await?;

    // Create children in specific order
    let children = vec!["zebra", "apple", "monkey", "banana"];

    for (i, name) in children.iter().enumerate() {
        let mut child = Node::default();
        child.id = format!("child-{}", i);
        child.name = name.to_string();
        child.path = format!("/parent/{}", name);
        child.node_type = "document".to_string();
        child.workspace = Some(workspace.to_string());
        child.parent = Some("/parent".to_string());

        repo.create(
            StorageScope::new("default", "default", "main", workspace),
            child,
            relaxed_create_opts(),
        )
        .await?;
    }

    // Reorder: move "apple" to position 2 (after "monkey")
    repo.reorder_child(
        StorageScope::new("default", "default", "main", workspace),
        "/parent",
        "apple",
        2,
        None,
        None,
    )
    .await?;

    let updated_parent = repo
        .get_by_path(
            StorageScope::new("default", "default", "main", workspace),
            "/parent",
            None,
        )
        .await?
        .unwrap();

    println!("Children order after reorder:");
    for (i, child_name) in updated_parent.children.iter().enumerate() {
        println!("  {}: {}", i, child_name);
    }

    // Expected order: zebra, monkey, apple, banana
    assert_eq!(
        updated_parent.children,
        vec!["zebra", "monkey", "apple", "banana"]
    );

    // Test list_by_parent respects order
    let children_list = repo
        .list_by_parent(
            StorageScope::new("default", "default", "main", workspace),
            "/parent",
            ListOptions {
                compute_has_children: false,
                max_revision: None,
            },
        )
        .await?;
    let child_names: Vec<String> = children_list.iter().map(|c| c.name.clone()).collect();

    println!("\nChildren from list_by_parent:");
    for (i, name) in child_names.iter().enumerate() {
        println!("  {}: {}", i, name);
    }

    assert_eq!(child_names, vec!["zebra", "monkey", "apple", "banana"]);

    // Test deep_children_array respects order
    let deep_children = repo
        .deep_children_array(
            StorageScope::new("default", "default", "main", workspace),
            "/parent",
            1,
            None,
        )
        .await?;
    let deep_names: Vec<String> = deep_children.iter().map(|c| c.node.name.clone()).collect();

    println!("\nChildren from deep_children_array:");
    for (i, name) in deep_names.iter().enumerate() {
        println!("  {}: {}", i, name);
    }

    assert_eq!(deep_names, vec!["zebra", "monkey", "apple", "banana"]);

    println!("\n✅ All ordering tests passed!");

    Ok(())
}

#[tokio::test]
async fn test_move_operations_preserve_order() -> Result<(), Box<dyn std::error::Error>> {
    let repo = InMemoryNodeRepo::new();
    let workspace = "test-ws";

    // Create parent
    let mut parent = Node::default();
    parent.id = "parent-1".to_string();
    parent.name = "parent".to_string();
    parent.path = "/parent".to_string();
    parent.node_type = "folder".to_string();
    parent.workspace = Some(workspace.to_string());
    parent.parent = None;
    repo.create(
        StorageScope::new("default", "default", "main", workspace),
        parent,
        relaxed_create_opts(),
    )
    .await?;

    // Create children: a, b, c, d, e
    for name in &["a", "b", "c", "d", "e"] {
        let mut child = Node::default();
        child.id = format!("child-{}", name);
        child.name = name.to_string();
        child.path = format!("/parent/{}", name);
        child.node_type = "document".to_string();
        child.workspace = Some(workspace.to_string());
        child.parent = Some("/parent".to_string());
        repo.create(
            StorageScope::new("default", "default", "main", workspace),
            child,
            relaxed_create_opts(),
        )
        .await?;
    }

    // Test move_child_before: move "d" before "b"
    repo.move_child_before(
        StorageScope::new("default", "default", "main", workspace),
        "/parent",
        "d",
        "b",
        None,
        None,
    )
    .await?;

    let parent = repo
        .get_by_path(
            StorageScope::new("default", "default", "main", workspace),
            "/parent",
            None,
        )
        .await?
        .unwrap();
    assert_eq!(parent.children, vec!["a", "d", "b", "c", "e"]);

    // Test move_child_after: move "a" after "c"
    repo.move_child_after(
        StorageScope::new("default", "default", "main", workspace),
        "/parent",
        "a",
        "c",
        None,
        None,
    )
    .await?;

    let parent = repo
        .get_by_path(
            StorageScope::new("default", "default", "main", workspace),
            "/parent",
            None,
        )
        .await?
        .unwrap();
    assert_eq!(parent.children, vec!["d", "b", "c", "a", "e"]);

    // Verify list_by_parent respects the new order
    let children_list = repo
        .list_by_parent(
            StorageScope::new("default", "default", "main", workspace),
            "/parent",
            ListOptions {
                compute_has_children: false,
                max_revision: None,
            },
        )
        .await?;
    let child_names: Vec<String> = children_list.iter().map(|c| c.name.clone()).collect();
    assert_eq!(child_names, vec!["d", "b", "c", "a", "e"]);

    Ok(())
}
