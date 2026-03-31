//! NodeType DDL operations: CREATE, ALTER, DROP

use crate::physical_plan::executor::RowStream;
use raisin_error::Error;
use raisin_models::nodes::types::node_type::NodeType;
use raisin_sql::ast::ddl::{CreateNodeType, DropNodeType};
use raisin_storage::{CommitMetadata, NodeTypeRepository, Storage};
use std::sync::Arc;

use super::conversions::{convert_compound_indexes, convert_properties, convert_property};
use super::ddl_success_stream;
use super::nested_properties::{add_nested_property, drop_nested_property, modify_nested_property};

// =============================================================================
// CREATE NODETYPE
// =============================================================================

pub(crate) async fn execute_create_nodetype<S: Storage + 'static>(
    create: &CreateNodeType,
    storage: Arc<S>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
) -> Result<RowStream, Error> {
    tracing::info!("Creating NodeType: {}", create.name);

    // Convert DDL AST to NodeType model
    let node_type = NodeType {
        id: Some(nanoid::nanoid!(16)),
        name: create.name.clone(),
        extends: create.extends.clone(),
        mixins: create.mixins.clone(),
        overrides: None,
        description: create.description.clone(),
        icon: create.icon.clone(),
        properties: if create.properties.is_empty() {
            None
        } else {
            Some(convert_properties(&create.properties)?)
        },
        allowed_children: create.allowed_children.clone(),
        required_nodes: create.required_nodes.clone(),
        initial_structure: None, // DDL doesn't support initial_structure yet
        versionable: if create.versionable { Some(true) } else { None },
        publishable: if create.publishable { Some(true) } else { None },
        auditable: if create.auditable { Some(true) } else { None },
        indexable: if create.indexable { Some(true) } else { None },
        index_types: None, // Use defaults
        strict: if create.strict { Some(true) } else { None },
        version: Some(1),
        created_at: Some(chrono::Utc::now()),
        updated_at: Some(chrono::Utc::now()),
        published_at: None,
        published_by: None,
        previous_version: None,
        compound_indexes: if create.compound_indexes.is_empty() {
            None
        } else {
            Some(convert_compound_indexes(&create.compound_indexes))
        },
        is_mixin: None,
    };

    // Create commit metadata
    let commit = CommitMetadata::system(format!("CREATE NODETYPE '{}'", create.name));

    // Create in storage
    storage
        .node_types()
        .create(
            raisin_storage::BranchScope::new(tenant_id, repo_id, branch),
            node_type,
            commit,
        )
        .await?;

    tracing::info!("✅ NodeType '{}' created successfully", create.name);

    // Return success result
    ddl_success_stream(&format!("NodeType '{}' created", create.name))
}

// =============================================================================
// ALTER NODETYPE
// =============================================================================

pub(crate) async fn execute_alter_nodetype<S: Storage + 'static>(
    alter: &raisin_sql::ast::ddl::AlterNodeType,
    storage: Arc<S>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
) -> Result<RowStream, Error> {
    tracing::info!("Altering NodeType: {}", alter.name);

    // Get existing NodeType using the `get` method (by name)
    let existing = storage
        .node_types()
        .get(
            raisin_storage::BranchScope::new(tenant_id, repo_id, branch),
            &alter.name,
            None,
        )
        .await?
        .ok_or_else(|| Error::NotFound(format!("NodeType '{}' not found", alter.name)))?;

    // Apply alterations
    let mut updated = existing.clone();

    for alteration in &alter.alterations {
        apply_nodetype_alteration(&mut updated, alteration)?;
    }

    // Update version and timestamp
    updated.version = Some(updated.version.unwrap_or(1) + 1);
    updated.updated_at = Some(chrono::Utc::now());

    // Create commit metadata
    let commit = CommitMetadata::system(format!("ALTER NODETYPE '{}'", alter.name));

    // Update in storage
    storage
        .node_types()
        .update(
            raisin_storage::BranchScope::new(tenant_id, repo_id, branch),
            updated,
            commit,
        )
        .await?;

    tracing::info!("✅ NodeType '{}' altered successfully", alter.name);

    ddl_success_stream(&format!("NodeType '{}' altered", alter.name))
}

fn apply_nodetype_alteration(
    node_type: &mut NodeType,
    alteration: &raisin_sql::ast::ddl::NodeTypeAlteration,
) -> Result<(), Error> {
    use raisin_sql::ast::ddl::NodeTypeAlteration;

    match alteration {
        NodeTypeAlteration::AddProperty(prop_def) => {
            if prop_def.is_nested_path() {
                // Handle nested path: navigate structure and add property
                add_nested_property(node_type, prop_def)?;
            } else {
                // Simple top-level property
                let prop = convert_property(prop_def)?;
                let props = node_type.properties.get_or_insert_with(Vec::new);
                props.push(prop);
            }
        }
        NodeTypeAlteration::DropProperty(name) => {
            if name.contains('.') {
                // Handle nested path: navigate structure and drop property
                drop_nested_property(node_type, name)?;
            } else {
                // Simple top-level drop
                if let Some(ref mut props) = node_type.properties {
                    props.retain(|p| p.name.as_deref() != Some(name.as_str()));
                }
            }
        }
        NodeTypeAlteration::ModifyProperty(prop_def) => {
            if prop_def.is_nested_path() {
                // Handle nested path: navigate structure and modify property
                modify_nested_property(node_type, prop_def)?;
            } else {
                // Simple top-level modify
                let prop = convert_property(prop_def)?;
                if let Some(ref mut props) = node_type.properties {
                    if let Some(existing) = props.iter_mut().find(|p| p.name == prop.name) {
                        *existing = prop;
                    } else {
                        props.push(prop);
                    }
                } else {
                    node_type.properties = Some(vec![prop]);
                }
            }
        }
        NodeTypeAlteration::SetDescription(desc) => {
            node_type.description = Some(desc.clone());
        }
        NodeTypeAlteration::SetIcon(icon) => {
            node_type.icon = Some(icon.clone());
        }
        NodeTypeAlteration::SetExtends(extends) => {
            node_type.extends = extends.clone();
        }
        NodeTypeAlteration::SetAllowedChildren(children) => {
            node_type.allowed_children = children.clone();
        }
        NodeTypeAlteration::SetRequiredNodes(required) => {
            node_type.required_nodes = required.clone();
        }
        NodeTypeAlteration::AddMixin(mixin) => {
            if !node_type.mixins.contains(mixin) {
                node_type.mixins.push(mixin.clone());
            }
        }
        NodeTypeAlteration::DropMixin(mixin) => {
            node_type.mixins.retain(|m| m != mixin);
        }
        NodeTypeAlteration::SetVersionable(v) => {
            node_type.versionable = Some(*v);
        }
        NodeTypeAlteration::SetPublishable(v) => {
            node_type.publishable = Some(*v);
        }
        NodeTypeAlteration::SetAuditable(v) => {
            node_type.auditable = Some(*v);
        }
        NodeTypeAlteration::SetIndexable(v) => {
            node_type.indexable = Some(*v);
        }
        NodeTypeAlteration::SetStrict(v) => {
            node_type.strict = Some(*v);
        }
    }

    Ok(())
}

// =============================================================================
// DROP NODETYPE
// =============================================================================

pub(crate) async fn execute_drop_nodetype<S: Storage + 'static>(
    drop: &DropNodeType,
    storage: Arc<S>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
) -> Result<RowStream, Error> {
    tracing::info!(
        "Dropping NodeType: {} (cascade={})",
        drop.name,
        drop.cascade
    );

    // Create commit metadata
    let commit = CommitMetadata::system(format!("DROP NODETYPE '{}'", drop.name));

    // Delete from storage
    storage
        .node_types()
        .delete(
            raisin_storage::BranchScope::new(tenant_id, repo_id, branch),
            &drop.name,
            commit,
        )
        .await?;

    tracing::info!("✅ NodeType '{}' dropped successfully", drop.name);

    ddl_success_stream(&format!("NodeType '{}' dropped", drop.name))
}
