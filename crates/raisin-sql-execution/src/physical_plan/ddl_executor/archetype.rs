//! Archetype DDL operations: CREATE, ALTER, DROP

use crate::physical_plan::executor::RowStream;
use raisin_error::Error;
use raisin_models::nodes::types::archetype::Archetype;
use raisin_sql::ast::ddl::{CreateArchetype, DropArchetype};
use raisin_storage::{ArchetypeRepository, CommitMetadata, Storage};
use std::sync::Arc;

use super::conversions::{convert_field, convert_fields};
use super::ddl_success_stream;

// =============================================================================
// CREATE ARCHETYPE
// =============================================================================

pub(crate) async fn execute_create_archetype<S: Storage + 'static>(
    create: &CreateArchetype,
    storage: Arc<S>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
) -> Result<RowStream, Error> {
    tracing::info!("Creating Archetype: {}", create.name);

    let fields = if create.fields.is_empty() {
        None
    } else {
        Some(convert_fields(&create.fields)?)
    };

    let archetype = Archetype {
        id: nanoid::nanoid!(16),
        name: create.name.clone(),
        extends: create.extends.clone(),
        strict: None,
        base_node_type: create.base_node_type.clone(),
        title: create.title.clone(),
        description: create.description.clone(),
        icon: create.icon.clone(),
        fields,
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

    let commit = CommitMetadata::system(format!("CREATE ARCHETYPE '{}'", create.name));

    storage
        .archetypes()
        .create(
            raisin_storage::BranchScope::new(tenant_id, repo_id, branch),
            archetype,
            commit,
        )
        .await?;

    tracing::info!("✅ Archetype '{}' created successfully", create.name);

    ddl_success_stream(&format!("Archetype '{}' created", create.name))
}

// =============================================================================
// ALTER ARCHETYPE
// =============================================================================

pub(crate) async fn execute_alter_archetype<S: Storage + 'static>(
    alter: &raisin_sql::ast::ddl::AlterArchetype,
    storage: Arc<S>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
) -> Result<RowStream, Error> {
    tracing::info!("Altering Archetype: {}", alter.name);

    let existing = storage
        .archetypes()
        .get(
            raisin_storage::BranchScope::new(tenant_id, repo_id, branch),
            &alter.name,
            None,
        )
        .await?
        .ok_or_else(|| Error::NotFound(format!("Archetype '{}' not found", alter.name)))?;

    let mut updated = existing.clone();

    for alteration in &alter.alterations {
        apply_archetype_alteration(&mut updated, alteration)?;
    }

    updated.version = Some(updated.version.unwrap_or(1) + 1);
    updated.updated_at = Some(chrono::Utc::now());

    let commit = CommitMetadata::system(format!("ALTER ARCHETYPE '{}'", alter.name));

    storage
        .archetypes()
        .update(
            raisin_storage::BranchScope::new(tenant_id, repo_id, branch),
            updated,
            commit,
        )
        .await?;

    tracing::info!("✅ Archetype '{}' altered successfully", alter.name);

    ddl_success_stream(&format!("Archetype '{}' altered", alter.name))
}

fn apply_archetype_alteration(
    archetype: &mut Archetype,
    alteration: &raisin_sql::ast::ddl::ArchetypeAlteration,
) -> Result<(), Error> {
    use raisin_sql::ast::ddl::ArchetypeAlteration;

    use raisin_models::nodes::types::element::field_types::FieldSchemaBase;

    match alteration {
        ArchetypeAlteration::AddField(field_def) => {
            let field = convert_field(field_def)?;
            // Re-declaring a field REPLACES it. See the ElementType executor
            // for why a duplicated name is not an option.
            let fields = archetype.fields.get_or_insert_with(Vec::new);
            fields.retain(|f| f.base_name() != &field_def.name);
            fields.push(field);
        }
        ArchetypeAlteration::DropField(name) => {
            let fields = archetype.fields.get_or_insert_with(Vec::new);
            let before = fields.len();
            fields.retain(|f| f.base_name() != name);
            if fields.len() == before {
                return Err(Error::NotFound(format!(
                    "Field '{}' not found on Archetype '{}'",
                    name, archetype.name
                )));
            }
        }
        ArchetypeAlteration::ModifyField(field_def) => {
            let field = convert_field(field_def)?;
            let slot = archetype
                .fields
                .as_mut()
                .and_then(|fs| fs.iter_mut().find(|f| f.base_name() == &field_def.name));
            let Some(slot) = slot else {
                return Err(Error::NotFound(format!(
                    "Field '{}' not found on Archetype '{}'; use ADD FIELD to create it",
                    field_def.name, archetype.name
                )));
            };
            *slot = field;
        }
        ArchetypeAlteration::SetDescription(desc) => {
            archetype.description = Some(desc.clone());
        }
        ArchetypeAlteration::SetTitle(title) => {
            archetype.title = Some(title.clone());
        }
        ArchetypeAlteration::SetIcon(icon) => {
            archetype.icon = Some(icon.clone());
        }
        ArchetypeAlteration::SetBaseNodeType(base) => {
            archetype.base_node_type = base.clone();
        }
        ArchetypeAlteration::SetExtends(extends) => {
            archetype.extends = extends.clone();
        }
        ArchetypeAlteration::SetPublishable(v) => {
            archetype.publishable = Some(*v);
        }
    }

    Ok(())
}

// =============================================================================
// DROP ARCHETYPE
// =============================================================================

pub(crate) async fn execute_drop_archetype<S: Storage + 'static>(
    drop: &DropArchetype,
    storage: Arc<S>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
) -> Result<RowStream, Error> {
    tracing::info!(
        "Dropping Archetype: {} (cascade={})",
        drop.name,
        drop.cascade
    );

    let commit = CommitMetadata::system(format!("DROP ARCHETYPE '{}'", drop.name));

    storage
        .archetypes()
        .delete(
            raisin_storage::BranchScope::new(tenant_id, repo_id, branch),
            &drop.name,
            commit,
        )
        .await?;

    tracing::info!("✅ Archetype '{}' dropped successfully", drop.name);

    ddl_success_stream(&format!("Archetype '{}' dropped", drop.name))
}
