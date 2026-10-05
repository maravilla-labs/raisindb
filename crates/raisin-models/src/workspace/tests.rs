//! Workspace model tests.

use super::*;
use chrono::{TimeZone, Utc};
use serde_json::json;

#[test]
fn deserializes_workspace_from_map_initial_structure() {
    let value = json!({
        "name": "access_control",
        "allowed_node_types": ["raisin:User", "raisin:Role"],
        "allowed_root_node_types": ["raisin:User", "raisin:Role"],
        "depends_on": [],
        "initial_structure": {
            "children": [
                {"name": "Users", "node_type": "raisin:AclFolder"},
                {"name": "Roles", "node_type": "raisin:AclFolder"}
            ]
        },
        "config": {
            "default_branch": "main",
            "node_type_pins": {}
        }
    });

    let workspace: Workspace =
        serde_json::from_value(value).expect("map-based workspace should deserialize");

    let children = workspace
        .initial_structure
        .as_ref()
        .and_then(|s| s.children.as_ref())
        .expect("children must be present");

    assert_eq!(children.len(), 2);
    assert_eq!(children[0].name, "Users");
    assert_eq!(children[1].name, "Roles");
}

#[test]
fn deserializes_workspace_from_map_with_rfc3339_timestamps() {
    let value = json!({
        "name": "test_workspace",
        "allowed_node_types": ["raisin:User"],
        "allowed_root_node_types": ["raisin:User"],
        "depends_on": [],
        "created_at": "2023-11-14T22:13:20Z",
        "updated_at": "2023-11-14T22:21:40Z"
    });

    let workspace: Workspace =
        serde_json::from_value(value).expect("RFC3339 timestamps should deserialize");

    assert_eq!(workspace.name, "test_workspace");
    assert_eq!(workspace.created_at.timestamp(), 1_700_000_000);
    assert_eq!(
        workspace
            .updated_at
            .expect("expected updated_at")
            .timestamp(),
        1_700_000_500
    );
}

#[test]
fn serializes_workspace_with_rfc3339_timestamps() {
    let workspace = Workspace {
        name: "test_workspace".to_string(),
        description: None,
        allowed_node_types: vec!["raisin:User".to_string()],
        allowed_root_node_types: vec!["raisin:User".to_string()],
        depends_on: vec![],
        initial_structure: None,
        created_at: Utc.timestamp_opt(1_700_000_000, 0).unwrap().into(),
        updated_at: Some(Utc.timestamp_opt(1_700_000_500, 0).unwrap().into()),
        config: WorkspaceConfig::default(),
        compound_indexes: None,
    };

    let json = serde_json::to_value(&workspace).expect("should serialize");

    // Verify timestamps are RFC3339 strings (chrono's to_rfc3339 uses +00:00 format)
    assert!(json["created_at"].as_str().unwrap().contains("2023-11-14"));
    assert!(json["updated_at"].as_str().unwrap().contains("2023-11-14"));
}

/// Phase 13e: a workspace written before `compound_indexes` existed still
/// decodes (named and compact msgpack), one without indexes serializes as
/// before, and owned definitions live in the workspace keyspace.
#[test]
fn workspace_compound_indexes_are_backward_compatible_and_owned() {
    use crate::nodes::properties::schema::{
        CompoundColumnType, CompoundIndexColumn, CompoundIndexOwner,
    };
    let plain = Workspace::new("content".to_string());
    let named = rmp_serde::to_vec_named(&plain).unwrap();
    let compact = rmp_serde::to_vec(&plain).unwrap();
    assert_eq!(rmp_serde::from_slice::<Workspace>(&named).unwrap(), plain);
    assert_eq!(rmp_serde::from_slice::<Workspace>(&compact).unwrap(), plain);
    assert!(!serde_json::to_string(&plain)
        .unwrap()
        .contains("compound_indexes"));

    let mut ws = plain.clone();
    ws.compound_indexes = Some(vec![CompoundIndexDefinition {
        name: "folder_time".to_string(),
        columns: vec![CompoundIndexColumn {
            property: "__parent_path".to_string(),
            ascending: None,
            column_type: CompoundColumnType::String,
        }],
        has_order_column: false,
        owner: None,
    }]);
    let back: Workspace = rmp_serde::from_slice(&rmp_serde::to_vec_named(&ws).unwrap()).unwrap();
    assert_eq!(back, ws);
    let owned = ws.owned_compound_indexes();
    // The declared index, then the built-in one (on by default).
    assert_eq!(owned.len(), 2);
    assert_eq!(owned[0].name, "@folder_time");
    assert_eq!(owned[1].name, "@__children_by_created_at");
    assert!(CompoundIndexDefinition::is_workspace_index_name(
        &owned[0].name
    ));
    assert_eq!(
        owned[0].owner,
        Some(CompoundIndexOwner::Workspace("content".to_string()))
    );
}

/// Plan Phase 13f: every workspace carries the built-in
/// `(__parent_path, __created_at)` index unless its config opts out; the
/// switch serializes only when set, a reserved user name is ignored, and
/// YAML/JSON can say `builtin_indexes: { children_by_created_at: false }`.
#[test]
fn builtin_children_by_created_at_is_on_by_default_and_opt_out() {
    use crate::nodes::properties::schema::CompoundIndexOwner;
    let ws = Workspace::new("content".to_string());
    let owned = ws.owned_compound_indexes();
    assert_eq!(owned.len(), 1);
    assert_eq!(
        owned[0].name,
        builtin_indexes::children_by_created_at_stored_name()
    );
    assert!(builtin_indexes::is_builtin_stored_name(&owned[0].name));
    assert_eq!(
        owned[0].owner,
        Some(CompoundIndexOwner::Workspace("content".to_string()))
    );
    let columns: Vec<&str> = owned[0]
        .columns
        .iter()
        .map(|c| c.property.as_str())
        .collect();
    assert_eq!(columns, ["__parent_path", "__created_at"]);
    assert!(owned[0].has_order_column);
    // Unset serializes to nothing (no stored workspace is rewritten).
    assert!(!serde_json::to_string(&ws)
        .unwrap()
        .contains("builtin_indexes"));

    // Opt out over JSON (the HTTP API and package YAML decode the model).
    let opted: Workspace = serde_json::from_value(json!({
        "name": "logs",
        "allowed_node_types": [],
        "allowed_root_node_types": [],
        "config": { "builtin_indexes": { "children_by_created_at": false } }
    }))
    .unwrap();
    assert!(opted.owned_compound_indexes().is_empty());
    let back: Workspace = rmp_serde::from_slice(&rmp_serde::to_vec_named(&opted).unwrap()).unwrap();
    assert_eq!(back, opted);
    // An empty switch block keeps the default (on).
    let empty: Workspace = serde_json::from_value(json!({
        "name": "x",
        "allowed_node_types": [],
        "allowed_root_node_types": [],
        "config": { "builtin_indexes": {} }
    }))
    .unwrap();
    assert_eq!(empty.owned_compound_indexes().len(), 1);

    // A user declaration may not take a reserved name: ignored by the
    // derivation (the write refuses it, `reserved_compound_index_names`).
    let mut sneaky = Workspace::new("content".to_string());
    let mut fake = builtin_indexes::children_by_created_at();
    fake.columns.truncate(1);
    sneaky.compound_indexes = Some(vec![fake]);
    assert_eq!(
        sneaky.reserved_compound_index_names(),
        [builtin_indexes::CHILDREN_BY_CREATED_AT]
    );
    let owned = sneaky.owned_compound_indexes();
    assert_eq!(owned.len(), 1);
    assert_eq!(
        owned[0].columns.len(),
        2,
        "the real built-in, not the user's"
    );
}
