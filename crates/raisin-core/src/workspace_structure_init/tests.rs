use super::*;
use raisin_models::nodes::types::initial_structure::{InitialChild, InitialNodeStructure};
use raisin_storage::{
    NodeRepository, NodeTypeRepository, Storage, StorageScope, WorkspaceRepository,
};
use raisin_storage_memory::InMemoryStorage;

#[tokio::test]
async fn creates_root_nodes_from_initial_structure_children() {
    let storage = Arc::new(InMemoryStorage::default());

    let mut workspace = raisin_models::workspace::Workspace::new("access".to_string());
    workspace.allowed_node_types = vec![
        "raisin:User".to_string(),
        "raisin:Role".to_string(),
        "raisin:Group".to_string(),
    ];
    workspace.allowed_root_node_types = vec![
        "raisin:User".to_string(),
        "raisin:Role".to_string(),
        "raisin:Group".to_string(),
    ];
    workspace.initial_structure = Some(InitialNodeStructure {
        properties: None,
        children: Some(vec![
            InitialChild {
                name: "Users".to_string(),
                node_type: "raisin:User".to_string(),
                archetype: None,
                properties: None,
                translations: None,
                children: None,
            },
            InitialChild {
                name: "Roles".to_string(),
                node_type: "raisin:Role".to_string(),
                archetype: None,
                properties: None,
                translations: None,
                children: None,
            },
            InitialChild {
                name: "Groups".to_string(),
                node_type: "raisin:Group".to_string(),
                archetype: None,
                properties: None,
                translations: None,
                children: None,
            },
        ]),
    });

    storage
        .workspaces()
        .put("tenant", "repo", workspace.clone())
        .await
        .unwrap();

    create_workspace_initial_structure(storage.clone(), "tenant", "repo", "access")
        .await
        .unwrap();

    for (original_name, sanitized_name) in
        [("Users", "users"), ("Roles", "roles"), ("Groups", "groups")]
    {
        let node = storage
            .nodes()
            .get_by_path(
                StorageScope::new("tenant", "repo", "main", "access"),
                &format!("/{}", sanitized_name),
                None,
            )
            .await
            .unwrap();
        assert!(
            node.is_some(),
            "Expected node '{}' (sanitized to '{}') to be created in workspace initial structure",
            original_name,
            sanitized_name
        );
    }
}

#[tokio::test]
async fn creates_nested_children_from_initial_structure() {
    let storage = Arc::new(InMemoryStorage::default());

    let mut workspace = raisin_models::workspace::Workspace::new("access".to_string());
    workspace.allowed_node_types = vec!["raisin:Folder".to_string()];
    workspace.allowed_root_node_types = vec!["raisin:Folder".to_string()];
    workspace.initial_structure = Some(InitialNodeStructure {
        properties: None,
        children: Some(vec![InitialChild {
            name: "Users".to_string(),
            node_type: "raisin:Folder".to_string(),
            archetype: None,
            properties: None,
            translations: None,
            children: Some(vec![InitialChild {
                name: "Admins".to_string(),
                node_type: "raisin:Folder".to_string(),
                archetype: None,
                properties: None,
                translations: None,
                children: None,
            }]),
        }]),
    });

    storage
        .workspaces()
        .put("tenant", "repo", workspace.clone())
        .await
        .unwrap();

    create_workspace_initial_structure(storage.clone(), "tenant", "repo", "access")
        .await
        .unwrap();

    let _users = storage
        .nodes()
        .get_by_path(
            StorageScope::new("tenant", "repo", "main", "access"),
            "/users",
            None,
        )
        .await
        .unwrap()
        .expect("users node must exist");

    let admins = storage
        .nodes()
        .get_by_path(
            StorageScope::new("tenant", "repo", "main", "access"),
            "/users/admins",
            None,
        )
        .await
        .unwrap()
        .expect("admins nested node must exist");

    assert_eq!(admins.parent.as_deref(), Some("users"));
    assert_eq!(admins.node_type, "raisin:Folder");
}

#[tokio::test]
async fn test_end_to_end_repository_initialization() {
    let storage = Arc::new(InMemoryStorage::default());
    let tenant_id = "test-tenant";
    let repo_id = "test-repo";

    crate::nodetype_init::init_repository_nodetypes(storage.clone(), tenant_id, repo_id, "main")
        .await
        .unwrap();

    let nodetype_repo = storage.node_types();
    let folder_type = nodetype_repo
        .get(tenant_id, repo_id, "main", "raisin:Folder", None)
        .await
        .unwrap();
    assert!(folder_type.is_some(), "raisin:Folder NodeType should exist");

    let acl_folder_type = nodetype_repo
        .get(tenant_id, repo_id, "main", "raisin:AclFolder", None)
        .await
        .unwrap();
    assert!(
        acl_folder_type.is_some(),
        "raisin:AclFolder NodeType should exist"
    );

    crate::workspace_init::init_repository_workspaces(storage.clone(), tenant_id, repo_id)
        .await
        .unwrap();

    let workspace_repo = storage.workspaces();
    let default_workspace = workspace_repo
        .get(tenant_id, repo_id, "default")
        .await
        .unwrap();
    assert!(
        default_workspace.is_some(),
        "default workspace should exist"
    );

    let access_workspace = workspace_repo
        .get(tenant_id, repo_id, "raisin:access_control")
        .await
        .unwrap();
    assert!(
        access_workspace.is_some(),
        "raisin:access_control workspace should exist"
    );

    create_workspace_initial_structure(
        storage.clone(),
        tenant_id,
        repo_id,
        "raisin:access_control",
    )
    .await
    .unwrap();

    let node_repo = storage.nodes();

    for name in ["users", "roles", "groups"] {
        let node = node_repo
            .get_by_path(
                StorageScope::new(tenant_id, repo_id, "main", "raisin:access_control"),
                &format!("/{}", name),
                None,
            )
            .await
            .unwrap();
        assert!(
            node.is_some(),
            "Expected '{}' node to be created in raisin:access_control workspace",
            name
        );

        let node = node.unwrap();
        assert_eq!(node.node_type, "raisin:AclFolder");
        assert_eq!(node.workspace.as_deref(), Some("raisin:access_control"));
    }
}

#[tokio::test]
async fn test_deeply_nested_structure() {
    let storage = Arc::new(InMemoryStorage::default());

    let mut workspace = raisin_models::workspace::Workspace::new("content".to_string());
    workspace.allowed_node_types = vec!["raisin:Folder".to_string()];
    workspace.allowed_root_node_types = vec!["raisin:Folder".to_string()];
    workspace.initial_structure = Some(InitialNodeStructure {
        properties: None,
        children: Some(vec![InitialChild {
            name: "docs".to_string(),
            node_type: "raisin:Folder".to_string(),
            archetype: None,
            properties: None,
            translations: None,
            children: Some(vec![InitialChild {
                name: "guides".to_string(),
                node_type: "raisin:Folder".to_string(),
                archetype: None,
                properties: None,
                translations: None,
                children: Some(vec![InitialChild {
                    name: "api".to_string(),
                    node_type: "raisin:Folder".to_string(),
                    archetype: None,
                    properties: None,
                    translations: None,
                    children: Some(vec![InitialChild {
                        name: "authentication".to_string(),
                        node_type: "raisin:Folder".to_string(),
                        archetype: None,
                        properties: None,
                        translations: None,
                        children: None,
                    }]),
                }]),
            }]),
        }]),
    });

    storage
        .workspaces()
        .put("tenant", "repo", workspace.clone())
        .await
        .unwrap();

    create_workspace_initial_structure(storage.clone(), "tenant", "repo", "content")
        .await
        .unwrap();

    let paths = [
        "/docs",
        "/docs/guides",
        "/docs/guides/api",
        "/docs/guides/api/authentication",
    ];
    for path in paths {
        let node = storage
            .nodes()
            .get_by_path(
                StorageScope::new("tenant", "repo", "main", "content"),
                path,
                None,
            )
            .await
            .unwrap();
        assert!(node.is_some(), "Expected node at path '{}' to exist", path);
    }

    let authentication = storage
        .nodes()
        .get_by_path(
            StorageScope::new("tenant", "repo", "main", "content"),
            "/docs/guides/api/authentication",
            None,
        )
        .await
        .unwrap()
        .expect("authentication node must exist");
    assert_eq!(authentication.parent.as_deref(), Some("api"));
}

#[tokio::test]
async fn test_mixed_siblings_and_nested_children() {
    let storage = Arc::new(InMemoryStorage::default());

    let mut workspace = raisin_models::workspace::Workspace::new("content".to_string());
    workspace.allowed_node_types = vec!["raisin:Folder".to_string()];
    workspace.allowed_root_node_types = vec!["raisin:Folder".to_string()];
    workspace.initial_structure = Some(InitialNodeStructure {
        properties: None,
        children: Some(vec![
            InitialChild {
                name: "docs".to_string(),
                node_type: "raisin:Folder".to_string(),
                archetype: None,
                properties: None,
                translations: None,
                children: Some(vec![
                    InitialChild {
                        name: "guides".to_string(),
                        node_type: "raisin:Folder".to_string(),
                        archetype: None,
                        properties: None,
                        translations: None,
                        children: None,
                    },
                    InitialChild {
                        name: "api".to_string(),
                        node_type: "raisin:Folder".to_string(),
                        archetype: None,
                        properties: None,
                        translations: None,
                        children: None,
                    },
                ]),
            },
            InitialChild {
                name: "assets".to_string(),
                node_type: "raisin:Folder".to_string(),
                archetype: None,
                properties: None,
                translations: None,
                children: Some(vec![
                    InitialChild {
                        name: "images".to_string(),
                        node_type: "raisin:Folder".to_string(),
                        archetype: None,
                        properties: None,
                        translations: None,
                        children: None,
                    },
                    InitialChild {
                        name: "videos".to_string(),
                        node_type: "raisin:Folder".to_string(),
                        archetype: None,
                        properties: None,
                        translations: None,
                        children: None,
                    },
                ]),
            },
            InitialChild {
                name: "pages".to_string(),
                node_type: "raisin:Folder".to_string(),
                archetype: None,
                properties: None,
                translations: None,
                children: None,
            },
        ]),
    });

    storage
        .workspaces()
        .put("tenant", "repo", workspace.clone())
        .await
        .unwrap();

    create_workspace_initial_structure(storage.clone(), "tenant", "repo", "content")
        .await
        .unwrap();

    for name in ["docs", "assets", "pages"] {
        let node = storage
            .nodes()
            .get_by_path(
                StorageScope::new("tenant", "repo", "main", "content"),
                &format!("/{}", name),
                None,
            )
            .await
            .unwrap();
        assert!(node.is_some(), "Root node '{}' should exist", name);
    }

    for name in ["guides", "api"] {
        let node = storage
            .nodes()
            .get_by_path(
                StorageScope::new("tenant", "repo", "main", "content"),
                &format!("/docs/{}", name),
                None,
            )
            .await
            .unwrap();
        assert!(node.is_some(), "Node '/docs/{}' should exist", name);
    }

    for name in ["images", "videos"] {
        let node = storage
            .nodes()
            .get_by_path(
                StorageScope::new("tenant", "repo", "main", "content"),
                &format!("/assets/{}", name),
                None,
            )
            .await
            .unwrap();
        assert!(node.is_some(), "Node '/assets/{}' should exist", name);
    }

    let docs = storage
        .nodes()
        .get_by_path(
            StorageScope::new("tenant", "repo", "main", "content"),
            "/docs",
            None,
        )
        .await
        .unwrap()
        .unwrap();
    let assets = storage
        .nodes()
        .get_by_path(
            StorageScope::new("tenant", "repo", "main", "content"),
            "/assets",
            None,
        )
        .await
        .unwrap()
        .unwrap();
    let pages = storage
        .nodes()
        .get_by_path(
            StorageScope::new("tenant", "repo", "main", "content"),
            "/pages",
            None,
        )
        .await
        .unwrap()
        .unwrap();

    assert_ne!(docs.order_key, assets.order_key);
    assert_ne!(assets.order_key, pages.order_key);
    assert_ne!(docs.order_key, pages.order_key);
}

#[tokio::test]
async fn test_initial_structure_with_properties_and_translations() {
    let storage = Arc::new(InMemoryStorage::default());

    let mut workspace = raisin_models::workspace::Workspace::new("content".to_string());
    workspace.allowed_node_types = vec!["raisin:Folder".to_string()];
    workspace.allowed_root_node_types = vec!["raisin:Folder".to_string()];

    let mut properties = HashMap::new();
    properties.insert("title".to_string(), serde_json::json!("Documentation"));
    properties.insert(
        "description".to_string(),
        serde_json::json!("Main documentation folder"),
    );

    let mut translations = HashMap::new();
    translations.insert(
        "en".to_string(),
        serde_json::json!({"title": "Documentation"}),
    );
    translations.insert(
        "de".to_string(),
        serde_json::json!({"title": "Dokumentation"}),
    );

    workspace.initial_structure = Some(InitialNodeStructure {
        properties: None,
        children: Some(vec![InitialChild {
            name: "docs".to_string(),
            node_type: "raisin:Folder".to_string(),
            archetype: None,
            properties: Some(properties),
            translations: Some(translations),
            children: None,
        }]),
    });

    storage
        .workspaces()
        .put("tenant", "repo", workspace.clone())
        .await
        .unwrap();

    create_workspace_initial_structure(storage.clone(), "tenant", "repo", "content")
        .await
        .unwrap();

    let docs = storage
        .nodes()
        .get_by_path(
            StorageScope::new("tenant", "repo", "main", "content"),
            "/docs",
            None,
        )
        .await
        .unwrap()
        .expect("docs node should exist");

    assert!(docs.properties.contains_key("title"));
    assert!(docs.properties.contains_key("description"));

    assert!(docs.translations.is_some());
    let translations = docs.translations.unwrap();
    assert!(translations.contains_key("en"));
    assert!(translations.contains_key("de"));
}

#[tokio::test]
async fn test_initial_structure_with_archetype() {
    let storage = Arc::new(InMemoryStorage::default());

    let mut workspace = raisin_models::workspace::Workspace::new("content".to_string());
    workspace.allowed_node_types = vec!["raisin:Folder".to_string()];
    workspace.allowed_root_node_types = vec!["raisin:Folder".to_string()];
    workspace.initial_structure = Some(InitialNodeStructure {
        properties: None,
        children: Some(vec![InitialChild {
            name: "docs".to_string(),
            node_type: "raisin:Folder".to_string(),
            archetype: Some("raisin:DefaultFolder".to_string()),
            properties: None,
            translations: None,
            children: None,
        }]),
    });

    storage
        .workspaces()
        .put("tenant", "repo", workspace.clone())
        .await
        .unwrap();

    create_workspace_initial_structure(storage.clone(), "tenant", "repo", "content")
        .await
        .unwrap();

    let docs = storage
        .nodes()
        .get_by_path(
            StorageScope::new("tenant", "repo", "main", "content"),
            "/docs",
            None,
        )
        .await
        .unwrap()
        .expect("docs node should exist");

    assert_eq!(docs.archetype.as_deref(), Some("raisin:DefaultFolder"));
}
