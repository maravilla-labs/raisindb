use super::*;
use crate::analyzer::types::DataType;

#[test]
fn test_default_nodes_schema() {
    let catalog = StaticCatalog::default_nodes_schema();
    let nodes = catalog.get_table("nodes").expect("nodes table exists");

    assert_eq!(nodes.name, "nodes");
    assert!(!nodes.columns.is_empty());
    assert_eq!(nodes.primary_key, vec!["path"]);
}

#[test]
fn test_get_column() {
    let catalog = StaticCatalog::default_nodes_schema();
    let nodes = catalog.get_table("nodes").unwrap();

    // Regular column
    let id_col = nodes.get_column("id").expect("id column exists");
    assert_eq!(id_col.name, "id");
    assert_eq!(id_col.data_type, DataType::Text);
    assert!(!id_col.nullable);
    assert!(id_col.generated.is_none());

    // Nullable column
    let archetype = nodes.get_column("archetype").expect("archetype exists");
    assert!(archetype.nullable);

    // Generated column
    let depth = nodes.get_column("depth").expect("depth exists");
    assert_eq!(depth.data_type, DataType::Int);
    assert_eq!(depth.generated, Some(GeneratedExpr::Depth));
    assert!(!depth.nullable);

    let parent_path = nodes.get_column("parent_path").expect("parent_path exists");
    assert_eq!(parent_path.data_type, DataType::Path);
    assert_eq!(parent_path.generated, Some(GeneratedExpr::ParentPath));
    assert!(parent_path.nullable);
}

#[test]
fn test_missing_column() {
    let catalog = StaticCatalog::default_nodes_schema();
    let nodes = catalog.get_table("nodes").unwrap();
    assert!(nodes.get_column("nonexistent").is_none());
}

#[test]
fn test_missing_table() {
    let catalog = StaticCatalog::default_nodes_schema();
    assert!(catalog.get_table("nonexistent").is_none());
}

#[test]
fn test_list_tables() {
    let catalog = StaticCatalog::default_nodes_schema();
    let tables = catalog.list_tables();
    // Should include: nodes, NodeTypes, Archetypes, ElementTypes, and default workspace
    assert_eq!(tables.len(), 5);
    assert!(tables.contains(&"nodes"));
    assert!(tables.contains(&"NodeTypes"));
    assert!(tables.contains(&"Archetypes"));
    assert!(tables.contains(&"ElementTypes"));
    assert!(tables.contains(&"default"));
}

#[test]
fn test_schema_tables() {
    let catalog = StaticCatalog::default_nodes_schema();

    // Test NodeTypes table
    let node_types = catalog
        .get_table("NodeTypes")
        .expect("NodeTypes table exists");
    assert_eq!(node_types.name, "NodeTypes");
    assert_eq!(node_types.primary_key, vec!["name"]);
    assert!(node_types.get_column("name").is_some());
    assert!(node_types.get_column("properties").is_some());
    assert!(node_types.get_column("allowed_children").is_some());
    assert!(node_types.get_column("__branch").is_some());

    // Verify JSONB columns
    let props = node_types.get_column("properties").unwrap();
    assert_eq!(props.data_type, DataType::JsonB);

    // Test Archetypes table
    let archetypes = catalog
        .get_table("Archetypes")
        .expect("Archetypes table exists");
    assert_eq!(archetypes.name, "Archetypes");
    assert!(archetypes.get_column("fields").is_some());
    assert!(archetypes.get_column("base_node_type").is_some());

    // Test ElementTypes table
    let element_types = catalog
        .get_table("ElementTypes")
        .expect("ElementTypes table exists");
    assert_eq!(element_types.name, "ElementTypes");
    assert!(element_types.get_column("layout").is_some());

    // Verify fields is not nullable for ElementTypes
    let fields = element_types.get_column("fields").unwrap();
    assert!(!fields.nullable);
}

#[test]
fn test_is_schema_table() {
    // Test case-insensitive matching
    assert!(is_schema_table("NodeTypes"));
    assert!(is_schema_table("nodetypes"));
    assert!(is_schema_table("NODETYPES"));
    assert!(is_schema_table("Archetypes"));
    assert!(is_schema_table("ElementTypes"));
    assert!(!is_schema_table("nodes"));
    assert!(!is_schema_table("default"));
    assert!(!is_schema_table("random"));
}

#[test]
fn test_schema_table_kind() {
    assert_eq!(
        SchemaTableKind::from_table_name("NodeTypes"),
        Some(SchemaTableKind::NodeTypes)
    );
    assert_eq!(
        SchemaTableKind::from_table_name("nodetypes"),
        Some(SchemaTableKind::NodeTypes)
    );
    assert_eq!(
        SchemaTableKind::from_table_name("Archetypes"),
        Some(SchemaTableKind::Archetypes)
    );
    assert_eq!(
        SchemaTableKind::from_table_name("ElementTypes"),
        Some(SchemaTableKind::ElementTypes)
    );
    assert_eq!(SchemaTableKind::from_table_name("nodes"), None);
    assert_eq!(SchemaTableKind::from_table_name("random"), None);

    // Test table_name()
    assert_eq!(SchemaTableKind::NodeTypes.table_name(), "NodeTypes");
    assert_eq!(SchemaTableKind::Archetypes.table_name(), "Archetypes");
    assert_eq!(SchemaTableKind::ElementTypes.table_name(), "ElementTypes");
}

#[test]
fn test_column_names() {
    let catalog = StaticCatalog::default_nodes_schema();
    let nodes = catalog.get_table("nodes").unwrap();
    let names = nodes.column_names();

    assert!(names.contains(&"id"));
    assert!(names.contains(&"path"));
    assert!(names.contains(&"depth"));
    assert!(names.contains(&"parent_path"));
}

#[test]
fn test_jsonb_columns() {
    let catalog = StaticCatalog::default_nodes_schema();
    let nodes = catalog.get_table("nodes").unwrap();

    let properties = nodes.get_column("properties").unwrap();
    assert_eq!(properties.data_type, DataType::JsonB);
    assert!(!properties.nullable);

    let translations = nodes.get_column("translations").unwrap();
    assert_eq!(translations.data_type, DataType::JsonB);
    assert!(translations.nullable);
}

#[test]
fn test_custom_catalog() {
    let mut catalog = StaticCatalog::new();
    assert!(catalog.list_tables().is_empty());

    catalog.add_table(TableDef {
        name: "test".into(),
        columns: vec![ColumnDef {
            name: "col1".into(),
            data_type: DataType::Int,
            nullable: false,
            generated: None,
        }],
        primary_key: vec!["col1".into()],
        indexes: vec![],
    });

    assert_eq!(catalog.list_tables(), vec!["test"]);
    assert!(catalog.get_table("test").is_some());
}

#[test]
fn test_workspace_to_table_name() {
    // Test with colon separator
    assert_eq!(
        workspace_to_table_name("raisin:access_control"),
        "RaisinAccessControl"
    );
    assert_eq!(workspace_to_table_name("raisin:user"), "RaisinUser");

    // Test with underscore separator
    assert_eq!(workspace_to_table_name("my_workspace"), "MyWorkspace");

    // Test with hyphen separator
    assert_eq!(workspace_to_table_name("my-workspace"), "MyWorkspace");

    // Test with space separator
    assert_eq!(workspace_to_table_name("my workspace"), "MyWorkspace");

    // Test with single word
    assert_eq!(workspace_to_table_name("default"), "Default");

    // Test with mixed separators
    assert_eq!(
        workspace_to_table_name("raisin:my_workspace-test"),
        "RaisinMyWorkspaceTest"
    );
}

#[test]
fn test_workspace_registration_with_mapping() {
    let mut catalog = StaticCatalog::new();

    // Register workspace with special characters
    catalog.register_workspace("raisin:access_control".to_string());

    // Check that workspace is registered
    assert!(catalog
        .workspaces()
        .contains(&"raisin:access_control".to_string()));

    // Check that CamelCase table name is recognized
    assert!(catalog.is_workspace("RaisinAccessControl"));

    // Check that original workspace name is still recognized
    assert!(catalog.is_workspace("raisin:access_control"));

    // Check that we can get workspace table using CamelCase name
    let table_def = catalog
        .get_workspace_table("RaisinAccessControl")
        .expect("Table should exist");
    assert_eq!(table_def.name, "RaisinAccessControl");

    // Verify we can resolve back to the original workspace name
    assert_eq!(
        catalog.resolve_workspace_name("RaisinAccessControl"),
        Some("raisin:access_control".to_string())
    );
}

#[test]
fn test_multiple_workspace_registrations() {
    let mut catalog = StaticCatalog::new();

    // Register multiple workspaces
    catalog.register_workspace("raisin:user".to_string());
    catalog.register_workspace("raisin:group".to_string());
    catalog.register_workspace("default".to_string());

    // Check all workspaces are recognized by their CamelCase names
    assert!(catalog.is_workspace("RaisinUser"));
    assert!(catalog.is_workspace("RaisinGroup"));
    assert!(catalog.is_workspace("Default"));

    // Check all workspaces are recognized by their original names
    assert!(catalog.is_workspace("raisin:user"));
    assert!(catalog.is_workspace("raisin:group"));
    assert!(catalog.is_workspace("default"));

    // Verify table definitions use the table names (CamelCase)
    assert_eq!(
        catalog.get_workspace_table("RaisinUser").unwrap().name,
        "RaisinUser"
    );
    assert_eq!(
        catalog.get_workspace_table("RaisinGroup").unwrap().name,
        "RaisinGroup"
    );

    // Verify we can resolve back to original workspace names
    assert_eq!(
        catalog.resolve_workspace_name("RaisinUser"),
        Some("raisin:user".to_string())
    );
    assert_eq!(
        catalog.resolve_workspace_name("RaisinGroup"),
        Some("raisin:group".to_string())
    );
}
