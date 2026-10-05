//! NodeTypes the delta-writer tests register.

use super::env::{REPO, TENANT};
use raisin_error::Result;
use raisin_models::nodes::properties::schema::{
    CompoundIndexDefinition, PropertyType, PropertyValueSchema,
};
use raisin_models::nodes::types::node_type::NodeType;
use raisin_rocksdb::RocksDBStorage;
use raisin_storage::scope::BranchScope;
use raisin_storage::{CommitMetadata, NodeTypeRepository, Storage};

/// Register `name` on `branch`: `unique` names a `unique: true` property,
/// `versionable` is the NodeType flag.
pub(super) async fn register_type(
    storage: &RocksDBStorage,
    branch: &str,
    name: &str,
    unique: Option<&str>,
    versionable: Option<bool>,
) -> Result<()> {
    register_type_with(storage, branch, name, unique, versionable, None).await
}

/// [`register_type`] with compound index declarations.
pub(super) async fn register_type_with(
    storage: &RocksDBStorage,
    branch: &str,
    name: &str,
    unique: Option<&str>,
    versionable: Option<bool>,
    compound_indexes: Option<Vec<CompoundIndexDefinition>>,
) -> Result<()> {
    let properties = unique.map(|prop| {
        vec![PropertyValueSchema {
            name: Some(prop.to_string()),
            property_type: PropertyType::String,
            required: None,
            unique: Some(true),
            default: None,
            constraints: None,
            structure: None,
            items: None,
            value: None,
            meta: None,
            is_translatable: None,
            allow_additional_properties: None,
            index: None,
            spatial: None,
            encrypted: None,
        }]
    });
    let ty = NodeType {
        id: Some(name.to_string()),
        strict: Some(false),
        name: name.to_string(),
        extends: None,
        mixins: Vec::new(),
        overrides: None,
        description: None,
        icon: None,
        version: Some(1),
        properties,
        allowed_children: vec!["*".to_string()],
        required_nodes: Vec::new(),
        initial_structure: None,
        versionable,
        immutable: None,
        publishable: Some(true),
        auditable: Some(false),
        indexable: Some(true),
        index_types: None,
        created_at: Some(chrono::Utc::now()),
        updated_at: None,
        published_at: None,
        published_by: None,
        previous_version: None,
        compound_indexes,
        is_mixin: None,
    };
    storage
        .node_types()
        .upsert(
            BranchScope::new(TENANT, REPO, branch),
            ty,
            CommitMetadata::system("seed type"),
        )
        .await?;
    Ok(())
}
