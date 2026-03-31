//! Mixin DDL operations: CREATE, ALTER, DROP
//!
//! Mixins are stored as NodeTypes with `is_mixin: Some(true)`.

use crate::physical_plan::executor::RowStream;
use raisin_error::Error;
use raisin_models::nodes::types::node_type::NodeType;
use raisin_sql::ast::ddl::{CreateMixin, DropMixin, MixinAlteration};
use raisin_storage::{CommitMetadata, NodeTypeRepository, Storage};
use std::sync::Arc;

use super::conversions::{convert_properties, convert_property};
use super::ddl_success_stream;

// =============================================================================
// CREATE MIXIN
// =============================================================================

pub(crate) async fn execute_create_mixin<S: Storage + 'static>(
    create: &CreateMixin,
    storage: Arc<S>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
) -> Result<RowStream, Error> {
    tracing::info!("Creating Mixin: {}", create.name);

    let node_type = NodeType {
        id: Some(nanoid::nanoid!(16)),
        name: create.name.clone(),
        extends: None,
        mixins: Vec::new(),
        overrides: None,
        description: create.description.clone(),
        icon: create.icon.clone(),
        properties: if create.properties.is_empty() {
            None
        } else {
            Some(convert_properties(&create.properties)?)
        },
        allowed_children: Vec::new(),
        required_nodes: Vec::new(),
        initial_structure: None,
        versionable: None,
        publishable: None,
        auditable: None,
        indexable: None,
        index_types: None,
        strict: None,
        version: Some(1),
        created_at: Some(chrono::Utc::now()),
        updated_at: Some(chrono::Utc::now()),
        published_at: None,
        published_by: None,
        previous_version: None,
        compound_indexes: None,
        is_mixin: Some(true),
    };

    let commit = CommitMetadata::system(format!("CREATE MIXIN '{}'", create.name));

    storage
        .node_types()
        .create(
            raisin_storage::BranchScope::new(tenant_id, repo_id, branch),
            node_type,
            commit,
        )
        .await?;

    tracing::info!("Mixin '{}' created successfully", create.name);

    ddl_success_stream(&format!("Mixin '{}' created", create.name))
}

// =============================================================================
// ALTER MIXIN
// =============================================================================

pub(crate) async fn execute_alter_mixin<S: Storage + 'static>(
    alter: &raisin_sql::ast::ddl::AlterMixin,
    storage: Arc<S>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
) -> Result<RowStream, Error> {
    tracing::info!("Altering Mixin: {}", alter.name);

    let existing = storage
        .node_types()
        .get(
            raisin_storage::BranchScope::new(tenant_id, repo_id, branch),
            &alter.name,
            None,
        )
        .await?
        .ok_or_else(|| Error::NotFound(format!("Mixin '{}' not found", alter.name)))?;

    if existing.is_mixin != Some(true) {
        return Err(Error::Validation(format!(
            "'{}' is not a mixin, use ALTER NODETYPE instead",
            alter.name
        )));
    }

    let mut updated = existing.clone();

    for alteration in &alter.alterations {
        apply_mixin_alteration(&mut updated, alteration)?;
    }

    updated.version = Some(updated.version.unwrap_or(1) + 1);
    updated.updated_at = Some(chrono::Utc::now());

    let commit = CommitMetadata::system(format!("ALTER MIXIN '{}'", alter.name));

    storage
        .node_types()
        .update(
            raisin_storage::BranchScope::new(tenant_id, repo_id, branch),
            updated,
            commit,
        )
        .await?;

    tracing::info!("Mixin '{}' altered successfully", alter.name);

    ddl_success_stream(&format!("Mixin '{}' altered", alter.name))
}

fn apply_mixin_alteration(
    node_type: &mut NodeType,
    alteration: &MixinAlteration,
) -> Result<(), Error> {
    match alteration {
        MixinAlteration::AddProperty(prop_def) => {
            let prop = convert_property(prop_def)?;
            let props = node_type.properties.get_or_insert_with(Vec::new);
            props.push(prop);
        }
        MixinAlteration::DropProperty(name) => {
            if let Some(ref mut props) = node_type.properties {
                props.retain(|p| p.name.as_deref() != Some(name.as_str()));
            }
        }
        MixinAlteration::ModifyProperty(prop_def) => {
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
        MixinAlteration::SetDescription(desc) => {
            node_type.description = Some(desc.clone());
        }
        MixinAlteration::SetIcon(icon) => {
            node_type.icon = Some(icon.clone());
        }
    }

    Ok(())
}

// =============================================================================
// DROP MIXIN
// =============================================================================

pub(crate) async fn execute_drop_mixin<S: Storage + 'static>(
    drop: &DropMixin,
    storage: Arc<S>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
) -> Result<RowStream, Error> {
    tracing::info!("Dropping Mixin: {} (cascade={})", drop.name, drop.cascade);

    let commit = CommitMetadata::system(format!("DROP MIXIN '{}'", drop.name));

    storage
        .node_types()
        .delete(
            raisin_storage::BranchScope::new(tenant_id, repo_id, branch),
            &drop.name,
            commit,
        )
        .await?;

    tracing::info!("Mixin '{}' dropped successfully", drop.name);

    ddl_success_stream(&format!("Mixin '{}' dropped", drop.name))
}
