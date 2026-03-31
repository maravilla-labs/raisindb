//! ElementType DDL operations: CREATE, ALTER, DROP

use crate::physical_plan::executor::RowStream;
use raisin_error::Error;
use raisin_models::nodes::types::element::element_type::ElementType;
use raisin_sql::ast::ddl::{CreateElementType, DropElementType};
use raisin_storage::{CommitMetadata, ElementTypeRepository, Storage};
use std::sync::Arc;

use super::ddl_success_stream;

// =============================================================================
// CREATE ELEMENTTYPE
// =============================================================================

pub(crate) async fn execute_create_elementtype<S: Storage + 'static>(
    create: &CreateElementType,
    storage: Arc<S>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
) -> Result<RowStream, Error> {
    tracing::info!("Creating ElementType: {}", create.name);

    // For ElementTypes, fields are Vec<FieldSchema>
    // For DDL simplicity, we'll use an empty Vec for now
    let element_type = ElementType {
        id: nanoid::nanoid!(16),
        name: create.name.clone(),
        extends: None,
        strict: None,
        title: None,
        description: create.description.clone(),
        icon: create.icon.clone(),
        fields: Vec::new(), // FieldSchema is complex; DDL doesn't support full conversion yet
        initial_content: None,
        layout: None,
        meta: None,
        publishable: if create.publishable { Some(true) } else { None },
        version: Some(1),
        created_at: Some(chrono::Utc::now()),
        updated_at: Some(chrono::Utc::now()),
        published_at: None,
        published_by: None,
        previous_version: None,
    };

    let commit = CommitMetadata::system(format!("CREATE ELEMENTTYPE '{}'", create.name));

    storage
        .element_types()
        .create(
            raisin_storage::BranchScope::new(tenant_id, repo_id, branch),
            element_type,
            commit,
        )
        .await?;

    tracing::info!("✅ ElementType '{}' created successfully", create.name);

    ddl_success_stream(&format!("ElementType '{}' created", create.name))
}

// =============================================================================
// ALTER ELEMENTTYPE
// =============================================================================

pub(crate) async fn execute_alter_elementtype<S: Storage + 'static>(
    alter: &raisin_sql::ast::ddl::AlterElementType,
    storage: Arc<S>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
) -> Result<RowStream, Error> {
    tracing::info!("Altering ElementType: {}", alter.name);

    let existing = storage
        .element_types()
        .get(
            raisin_storage::BranchScope::new(tenant_id, repo_id, branch),
            &alter.name,
            None,
        )
        .await?
        .ok_or_else(|| Error::NotFound(format!("ElementType '{}' not found", alter.name)))?;

    let mut updated = existing.clone();

    for alteration in &alter.alterations {
        apply_elementtype_alteration(&mut updated, alteration)?;
    }

    updated.version = Some(updated.version.unwrap_or(1) + 1);
    updated.updated_at = Some(chrono::Utc::now());

    let commit = CommitMetadata::system(format!("ALTER ELEMENTTYPE '{}'", alter.name));

    storage
        .element_types()
        .update(
            raisin_storage::BranchScope::new(tenant_id, repo_id, branch),
            updated,
            commit,
        )
        .await?;

    tracing::info!("✅ ElementType '{}' altered successfully", alter.name);

    ddl_success_stream(&format!("ElementType '{}' altered", alter.name))
}

fn apply_elementtype_alteration(
    element_type: &mut ElementType,
    alteration: &raisin_sql::ast::ddl::ElementTypeAlteration,
) -> Result<(), Error> {
    use raisin_sql::ast::ddl::ElementTypeAlteration;

    match alteration {
        ElementTypeAlteration::AddField(_field_def) => {
            // FieldSchema conversion not supported yet
            tracing::warn!("ADD FIELD not fully supported for ElementTypes in DDL");
        }
        ElementTypeAlteration::DropField(_name) => {
            // FieldSchema manipulation not supported yet
            tracing::warn!("DROP FIELD not fully supported for ElementTypes in DDL");
        }
        ElementTypeAlteration::ModifyField(_field_def) => {
            // FieldSchema manipulation not supported yet
            tracing::warn!("MODIFY FIELD not fully supported for ElementTypes in DDL");
        }
        ElementTypeAlteration::SetDescription(desc) => {
            element_type.description = Some(desc.clone());
        }
        ElementTypeAlteration::SetIcon(icon) => {
            element_type.icon = Some(icon.clone());
        }
        ElementTypeAlteration::SetPublishable(v) => {
            element_type.publishable = Some(*v);
        }
    }

    Ok(())
}

// =============================================================================
// DROP ELEMENTTYPE
// =============================================================================

pub(crate) async fn execute_drop_elementtype<S: Storage + 'static>(
    drop: &DropElementType,
    storage: Arc<S>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
) -> Result<RowStream, Error> {
    tracing::info!(
        "Dropping ElementType: {} (cascade={})",
        drop.name,
        drop.cascade
    );

    let commit = CommitMetadata::system(format!("DROP ELEMENTTYPE '{}'", drop.name));

    storage
        .element_types()
        .delete(
            raisin_storage::BranchScope::new(tenant_id, repo_id, branch),
            &drop.name,
            commit,
        )
        .await?;

    tracing::info!("✅ ElementType '{}' dropped successfully", drop.name);

    ddl_success_stream(&format!("ElementType '{}' dropped", drop.name))
}
