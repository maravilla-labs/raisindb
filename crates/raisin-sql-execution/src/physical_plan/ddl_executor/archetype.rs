//! Archetype DDL operations: CREATE, ALTER, DROP

use crate::physical_plan::executor::RowStream;
use raisin_error::Error;
use raisin_models::nodes::types::archetype::Archetype;
use raisin_sql::ast::ddl::{CreateArchetype, DropArchetype};
use raisin_storage::{ArchetypeRepository, CommitMetadata, Storage};
use std::sync::Arc;

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

    // For Archetypes, fields are FieldSchema which is a tagged enum
    // For DDL simplicity, we'll store fields as None for now
    // A future enhancement could convert PropertyDef to FieldSchema
    let archetype = Archetype {
        id: nanoid::nanoid!(16),
        name: create.name.clone(),
        extends: create.extends.clone(),
        strict: None,
        base_node_type: create.base_node_type.clone(),
        title: create.title.clone(),
        description: create.description.clone(),
        icon: create.icon.clone(),
        fields: None, // FieldSchema is complex; DDL doesn't support full conversion yet
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

    match alteration {
        ArchetypeAlteration::AddField(_field_def) => {
            // FieldSchema conversion not supported yet
            tracing::warn!("ADD FIELD not fully supported for Archetypes in DDL");
        }
        ArchetypeAlteration::DropField(_name) => {
            // FieldSchema manipulation not supported yet
            tracing::warn!("DROP FIELD not fully supported for Archetypes in DDL");
        }
        ArchetypeAlteration::ModifyField(_field_def) => {
            // FieldSchema manipulation not supported yet
            tracing::warn!("MODIFY FIELD not fully supported for Archetypes in DDL");
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
