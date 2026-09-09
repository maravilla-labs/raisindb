//! ElementType DDL operations: CREATE, ALTER, DROP

use crate::physical_plan::executor::RowStream;
use raisin_error::Error;
use raisin_models::nodes::types::element::element_type::ElementType;
use raisin_sql::ast::ddl::{CreateElementType, DropElementType};
use raisin_storage::{CommitMetadata, ElementTypeRepository, Storage};
use std::sync::Arc;

use super::conversions::{convert_field, convert_fields};
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
        fields: convert_fields(&create.fields)?,
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

    use raisin_models::nodes::types::element::field_types::FieldSchemaBase;

    match alteration {
        ElementTypeAlteration::AddField(field_def) => {
            let field = convert_field(field_def)?;
            // Re-declaring a field REPLACES it rather than duplicating the
            // name. Two entries with one name is a schema no resolver can read
            // consistently, and the author's intent when they re-state a field
            // is the newer definition.
            element_type
                .fields
                .retain(|f| f.base_name() != &field_def.name);
            element_type.fields.push(field);
        }
        ElementTypeAlteration::DropField(name) => {
            let before = element_type.fields.len();
            element_type.fields.retain(|f| f.base_name() != name);
            if element_type.fields.len() == before {
                return Err(Error::NotFound(format!(
                    "Field '{}' not found on ElementType '{}'",
                    name, element_type.name
                )));
            }
        }
        ElementTypeAlteration::ModifyField(field_def) => {
            let field = convert_field(field_def)?;
            let Some(slot) = element_type
                .fields
                .iter_mut()
                .find(|f| f.base_name() == &field_def.name)
            else {
                return Err(Error::NotFound(format!(
                    "Field '{}' not found on ElementType '{}'; use ADD FIELD to create it",
                    field_def.name, element_type.name
                )));
            };
            *slot = field;
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
