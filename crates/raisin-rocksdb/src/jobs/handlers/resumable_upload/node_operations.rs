//! Node creation and update operations for resumable uploads.
//!
//! Handles creating or updating nodes with uploaded file resources,
//! including property mapping and user metadata merging.

use raisin_error::Result;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::value::{PropertyValue, Resource};
use raisin_models::nodes::Node;
use raisin_storage::jobs::JobContext;
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::upload_sessions::UploadSession;
use raisin_storage::Storage;
use std::collections::HashMap;

use super::handler::ResumableUploadHandler;

impl<S: Storage + TransactionalStorage> ResumableUploadHandler<S> {
    /// Create or update node with the uploaded file
    pub(super) async fn create_or_update_node(
        &self,
        session: &UploadSession,
        stored: &raisin_binary::StoredObject,
        context: &JobContext,
        commit_message: &Option<String>,
        commit_actor: &Option<String>,
    ) -> Result<String> {
        // Begin transaction with system auth context
        let tx = self.storage.begin_context().await?;
        tx.set_tenant_repo(&context.tenant_id, &context.repo_id)?;
        tx.set_branch(&context.branch)?;
        tx.set_actor(commit_actor.as_deref().unwrap_or("upload-handler"))?;
        tx.set_auth_context(AuthContext::system())?;
        tx.set_message(
            commit_message
                .as_deref()
                .unwrap_or("Complete resumable upload"),
        )?;

        let workspace = &session.workspace;

        // Build Resource property
        let resource = build_resource(session, stored);

        // Check if node already exists at this path
        let existing_node = tx.get_node_by_path(workspace, &session.path).await?;

        let node_id = if let Some(mut node) = existing_node {
            // Update existing node
            tracing::debug!(
                node_id = %node.id,
                path = %session.path,
                "Updating existing node with uploaded file"
            );

            apply_file_properties(&mut node.properties, session, stored, &resource);
            merge_user_metadata(&mut node.properties, session);

            tx.upsert_node(workspace, &node).await?;
            tx.commit().await?;

            node.id.clone()
        } else {
            // Create new node
            let node_name = session
                .path
                .rsplit('/')
                .next()
                .unwrap_or(&session.filename)
                .to_string();

            tracing::debug!(
                path = %session.path,
                node_name = %node_name,
                node_type = %session.node_type,
                "Creating new node for uploaded file"
            );

            let mut properties = HashMap::new();
            properties.insert(
                "title".to_string(),
                PropertyValue::String(node_name.clone()),
            );
            apply_file_properties(&mut properties, session, stored, &resource);
            merge_user_metadata(&mut properties, session);

            let node = Node {
                id: nanoid::nanoid!(),
                node_type: session.node_type.clone(),
                name: node_name,
                path: session.path.clone(),
                workspace: Some(workspace.clone()),
                parent: None, // Will be set by upsert_deep_node
                properties,
                ..Default::default()
            };

            tx.upsert_deep_node(workspace, &node, "raisin:Folder")
                .await?;
            tx.commit().await?;

            node.id.clone()
        };

        Ok(node_id)
    }
}

/// Build a Resource property value from session and stored object metadata
fn build_resource(session: &UploadSession, stored: &raisin_binary::StoredObject) -> Resource {
    let mut resource_metadata = HashMap::new();
    resource_metadata.insert(
        "storage_key".to_string(),
        PropertyValue::String(stored.key.clone()),
    );

    Resource {
        uuid: nanoid::nanoid!(),
        name: stored.name.clone(),
        size: Some(stored.size),
        mime_type: session.content_type.clone(),
        url: Some(stored.url.clone()),
        metadata: Some(resource_metadata),
        is_loaded: Some(true),
        is_external: Some(false),
        created_at: stored.created_at.into(),
        updated_at: stored.updated_at.into(),
    }
}

/// Apply file-related properties to a node's property map
fn apply_file_properties(
    properties: &mut HashMap<String, PropertyValue>,
    session: &UploadSession,
    stored: &raisin_binary::StoredObject,
    resource: &Resource,
) {
    properties.insert(
        "file".to_string(),
        PropertyValue::Resource(resource.clone()),
    );
    properties.insert(
        "file_type".to_string(),
        PropertyValue::String(
            session
                .content_type
                .clone()
                .unwrap_or_else(|| "application/octet-stream".to_string()),
        ),
    );
    properties.insert("file_size".to_string(), PropertyValue::Integer(stored.size));
}

/// Merge user-provided metadata into node properties
fn merge_user_metadata(properties: &mut HashMap<String, PropertyValue>, session: &UploadSession) {
    if !session.metadata.is_null() {
        if let serde_json::Value::Object(user_meta) = &session.metadata {
            for (key, value) in user_meta {
                if let Ok(prop_value) = serde_json::from_value(value.clone()) {
                    properties.insert(key.clone(), prop_value);
                }
            }
        }
    }
}
