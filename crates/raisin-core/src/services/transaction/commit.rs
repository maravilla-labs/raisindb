//! Transaction commit logic including initial structure creation and operation application.

use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_storage::{
    scope::BranchScope, transactional::TransactionalStorage, BranchRepository, NodeTypeRepository,
    Storage,
};
use std::collections::HashMap;

use super::Transaction;

impl<S: TransactionalStorage> Transaction<S> {
    /// Helper to create initial children from NodeType definition within a transaction
    pub(super) async fn create_initial_structure_children(
        &self,
        ctx: &dyn raisin_storage::transactional::TransactionalContext,
        parent_node: &Node,
        actor: &str,
    ) -> Result<()> {
        let node_type = self
            .storage
            .node_types()
            .get(
                BranchScope::new(&self.tenant_id, &self.repo_id, &self.branch),
                &parent_node.node_type,
                None,
            )
            .await?;

        if let Some(node_type) = node_type {
            if let Some(initial_structure) = &node_type.initial_structure {
                if let Some(children_defs) = &initial_structure.children {
                    for child_def in children_defs {
                        self.create_initial_child_recursive(ctx, parent_node, child_def, actor)
                            .await?;
                    }
                }
            }
        }

        Ok(())
    }

    /// Recursively creates a single initial child and its nested children
    pub(super) fn create_initial_child_recursive<'a>(
        &'a self,
        ctx: &'a dyn raisin_storage::transactional::TransactionalContext,
        parent_node: &'a Node,
        child_def: &'a raisin_models::nodes::types::initial_structure::InitialChild,
        actor: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Node>> + Send + 'a>> {
        Box::pin(async move {
            let properties = if let Some(props) = &child_def.properties {
                props
                    .iter()
                    .map(|(k, v)| {
                        let property_value = match v {
                            serde_json::Value::String(s) => PropertyValue::String(s.clone()),
                            serde_json::Value::Number(n) => {
                                if let Some(i) = n.as_i64() {
                                    PropertyValue::Integer(i)
                                } else if let Some(f) = n.as_f64() {
                                    PropertyValue::Float(f)
                                } else {
                                    PropertyValue::String(n.to_string())
                                }
                            }
                            serde_json::Value::Bool(b) => PropertyValue::Boolean(*b),
                            serde_json::Value::Array(_) => {
                                PropertyValue::String(serde_json::to_string(v).unwrap_or_default())
                            }
                            serde_json::Value::Object(_) => {
                                PropertyValue::String(serde_json::to_string(v).unwrap_or_default())
                            }
                            serde_json::Value::Null => PropertyValue::String(String::new()),
                        };
                        (k.clone(), property_value)
                    })
                    .collect()
            } else {
                HashMap::new()
            };

            let translations = child_def.translations.as_ref().map(|trans| {
                trans
                    .iter()
                    .map(|(lang, val)| {
                        let property_value = match val {
                            serde_json::Value::String(s) => PropertyValue::String(s.clone()),
                            serde_json::Value::Number(n) => {
                                if let Some(i) = n.as_i64() {
                                    PropertyValue::Integer(i)
                                } else if let Some(f) = n.as_f64() {
                                    PropertyValue::Float(f)
                                } else {
                                    PropertyValue::String(n.to_string())
                                }
                            }
                            serde_json::Value::Bool(b) => PropertyValue::Boolean(*b),
                            _ => PropertyValue::String(
                                serde_json::to_string(val).unwrap_or_default(),
                            ),
                        };
                        (lang.clone(), property_value)
                    })
                    .collect()
            });

            let sanitized_name = crate::sanitize_name(&child_def.name)?;

            let child_node = Node {
                id: nanoid::nanoid!(),
                name: sanitized_name.clone(),
                path: format!("{}/{}", parent_node.path, sanitized_name),
                node_type: child_def.node_type.clone(),
                archetype: child_def.archetype.clone(),
                properties,
                children: vec![],
                order_key: String::new(),
                has_children: None,
                parent: None,
                version: 1,
                created_at: Some(chrono::Utc::now()),
                created_by: Some(actor.to_string()),
                updated_at: None,
                published_at: None,
                published_by: None,
                updated_by: None,
                translations,
                tenant_id: None,
                workspace: Some(self.workspace_id.clone()),
                owner_id: None,
                relations: Vec::new(),
            };

            ctx.add_node(&self.workspace_id, &child_node).await?;

            self.create_initial_structure_children(ctx, &child_node, actor)
                .await?;

            if let Some(nested_children) = &child_def.children {
                for nested_child_def in nested_children {
                    self.create_initial_child_recursive(ctx, &child_node, nested_child_def, actor)
                        .await?;
                }
            }

            Ok(child_node)
        })
    }

    /// Commit transaction, creating a new repository revision
    ///
    /// All operations are applied atomically. If any operation fails,
    /// the entire transaction is rolled back.
    ///
    /// # Arguments
    /// * `message` - Commit message describing the changes
    /// * `actor` - User/system identifier performing the commit
    ///
    /// # Returns
    /// The new revision (HLC timestamp)
    pub async fn commit(self, message: impl Into<String>, actor: impl Into<String>) -> Result<HLC> {
        if self.operations.is_empty() {
            return Err(raisin_error::Error::Validation(
                "Cannot commit empty transaction".into(),
            ));
        }

        let message = message.into();
        let actor = actor.into();

        tracing::info!(
            "Committing transaction: {} operations, message: '{}', actor: '{}'",
            self.operations.len(),
            message,
            actor
        );

        let ctx = self.storage.begin_context().await?;

        ctx.set_tenant_repo(&self.tenant_id, &self.repo_id)?;
        ctx.set_branch(&self.branch)?;
        ctx.set_message(&message)?;
        ctx.set_actor(&actor)?;

        if let Some(auth) = &self.auth_context {
            ctx.set_auth_context(auth.clone())?;
        }

        for op in &self.operations {
            self.apply_operation(ctx.as_ref(), op, &actor).await?;
        }

        ctx.commit().await?;

        let branches = self.storage.branches();
        let revision = if let Some(branch_info) = branches
            .get_branch(&self.tenant_id, &self.repo_id, &self.branch)
            .await?
        {
            branch_info.head
        } else {
            HLC::new(1, 0)
        };

        tracing::info!(
            "Transaction committed successfully: revision {}, {} operations",
            revision,
            self.operations.len()
        );

        Ok(revision)
    }

    /// Apply a single transaction operation to the context
    async fn apply_operation(
        &self,
        ctx: &dyn raisin_storage::transactional::TransactionalContext,
        op: &super::TxOperation,
        actor: &str,
    ) -> Result<()> {
        match op {
            super::TxOperation::Create { node } => {
                ctx.add_node(&self.workspace_id, node.as_ref()).await?;
                self.create_initial_structure_children(ctx, node.as_ref(), actor)
                    .await?;
            }
            super::TxOperation::Update {
                node_id,
                properties,
            } => {
                let mut node = ctx
                    .get_node(&self.workspace_id, node_id)
                    .await?
                    .ok_or_else(|| {
                        raisin_error::Error::NotFound(format!("Node {} not found", node_id))
                    })?;

                if let Some(props) = properties.as_object() {
                    for (key, value) in props {
                        let prop_value = serde_json::from_value(value.clone())
                            .map_err(|e| raisin_error::Error::Validation(e.to_string()))?;
                        node.properties.insert(key.clone(), prop_value);
                    }
                }

                node.updated_at = Some(chrono::Utc::now());
                node.updated_by = Some(actor.to_string());

                ctx.put_node(&self.workspace_id, &node).await?;
            }
            super::TxOperation::Delete { node_id } => {
                ctx.delete_node(&self.workspace_id, node_id).await?;
            }
            super::TxOperation::Move { node_id, new_path } => {
                self.apply_move(ctx, node_id, new_path, actor).await?;
            }
            super::TxOperation::Rename { node_id, new_name } => {
                self.apply_rename(ctx, node_id, new_name, actor).await?;
            }
            super::TxOperation::Copy {
                source_path,
                target_parent,
                new_name,
            } => {
                self.apply_copy(ctx, source_path, target_parent, new_name.as_deref(), actor)
                    .await?;
            }
            super::TxOperation::CopyTree {
                source_path,
                target_parent,
                new_name,
            } => {
                tracing::debug!(
                    "Copying tree: {} -> {} (new_name: {:?})",
                    source_path,
                    target_parent,
                    new_name
                );

                ctx.copy_node_tree(
                    &self.workspace_id,
                    source_path,
                    target_parent,
                    new_name.as_deref(),
                    actor,
                )
                .await?;

                tracing::info!("Copied tree from '{}' to '{}'", source_path, target_parent);
            }
        }

        Ok(())
    }

    /// Apply a move operation
    async fn apply_move(
        &self,
        ctx: &dyn raisin_storage::transactional::TransactionalContext,
        node_id: &str,
        new_path: &str,
        actor: &str,
    ) -> Result<()> {
        let node = ctx
            .get_node(&self.workspace_id, node_id)
            .await?
            .ok_or_else(|| raisin_error::Error::NotFound(format!("Node {} not found", node_id)))?;

        let old_path = node.path.clone();

        ctx.delete_path_index(&self.workspace_id, &old_path).await?;

        tracing::debug!("Moving node '{}': {} -> {}", node_id, old_path, new_path);

        let mut updated_node = node.clone();
        updated_node.path = new_path.to_string();
        updated_node.parent = raisin_models::nodes::Node::extract_parent_name_from_path(new_path);
        updated_node.updated_at = Some(chrono::Utc::now());
        updated_node.updated_by = Some(actor.to_string());

        ctx.put_node(&self.workspace_id, &updated_node).await?;

        if !node.children.is_empty() {
            tracing::debug!(
                "Moving descendants: node has {} children",
                node.children.len()
            );

            move_descendants(ctx, &self.workspace_id, &old_path, new_path, actor).await?;
        }

        Ok(())
    }

    /// Apply a rename operation
    async fn apply_rename(
        &self,
        ctx: &dyn raisin_storage::transactional::TransactionalContext,
        node_id: &str,
        new_name: &str,
        actor: &str,
    ) -> Result<()> {
        let node = ctx
            .get_node(&self.workspace_id, node_id)
            .await?
            .ok_or_else(|| raisin_error::Error::NotFound(format!("Node {} not found", node_id)))?;

        if new_name.is_empty() || new_name.contains('/') {
            return Err(raisin_error::Error::Validation(
                "Invalid name: cannot be empty or contain '/'".into(),
            ));
        }

        let old_path = node.path.clone();

        let new_path = if let Some(parent_pos) = old_path.rfind('/') {
            if parent_pos == 0 {
                format!("/{}", new_name)
            } else {
                format!("{}/{}", &old_path[..parent_pos], new_name)
            }
        } else {
            format!("/{}", new_name)
        };

        ctx.delete_path_index(&self.workspace_id, &old_path).await?;

        tracing::debug!(
            "Renaming node '{}': '{}' -> '{}' (path: {} -> {})",
            node_id,
            node.name,
            new_name,
            old_path,
            new_path
        );

        let mut updated_node = node.clone();
        updated_node.name = new_name.to_string();
        updated_node.path = new_path.clone();
        updated_node.updated_at = Some(chrono::Utc::now());
        updated_node.updated_by = Some(actor.to_string());

        ctx.put_node(&self.workspace_id, &updated_node).await?;

        if !node.children.is_empty() {
            tracing::debug!(
                "Renaming descendants: node has {} children",
                node.children.len()
            );

            rename_descendants(ctx, &self.workspace_id, &old_path, &new_path, actor).await?;
        }

        Ok(())
    }

    /// Apply a single node copy operation
    async fn apply_copy(
        &self,
        ctx: &dyn raisin_storage::transactional::TransactionalContext,
        source_path: &str,
        target_parent: &str,
        new_name: Option<&str>,
        actor: &str,
    ) -> Result<()> {
        let source_node = ctx
            .get_node_by_path(&self.workspace_id, source_path)
            .await?
            .ok_or_else(|| {
                raisin_error::Error::NotFound(format!(
                    "Source node not found at path: {}",
                    source_path
                ))
            })?;

        tracing::debug!(
            "Copying node '{}': {} -> {} (new_name: {:?})",
            source_node.id,
            source_path,
            target_parent,
            new_name
        );

        let copied_name = new_name.unwrap_or(&source_node.name);

        let target_path = if target_parent == "/" || target_parent.is_empty() {
            format!("/{}", copied_name)
        } else {
            format!("{}/{}", target_parent, copied_name)
        };

        if ctx
            .get_node_by_path(&self.workspace_id, &target_path)
            .await?
            .is_some()
        {
            return Err(raisin_error::Error::Validation(format!(
                "A node already exists at path: {}",
                target_path
            )));
        }

        let mut copied_node = source_node.clone();
        copied_node.id = nanoid::nanoid!();
        copied_node.name = copied_name.to_string();
        copied_node.path = target_path.clone();
        copied_node.parent =
            raisin_models::nodes::Node::extract_parent_name_from_path(&target_path);
        copied_node.created_at = Some(chrono::Utc::now());
        copied_node.created_by = Some(actor.to_string());
        copied_node.updated_at = Some(chrono::Utc::now());
        copied_node.updated_by = Some(actor.to_string());
        copied_node.published_at = None;
        copied_node.published_by = None;

        ctx.add_node(&self.workspace_id, &copied_node).await?;

        // Copy translations
        tracing::debug!("Copying translations for node '{}'", source_node.id);
        let source_locales = ctx
            .list_translations_for_node(&self.workspace_id, &source_node.id)
            .await?;

        for locale in source_locales {
            if ctx
                .get_translation(&self.workspace_id, &copied_node.id, &locale)
                .await?
                .is_none()
            {
                if let Some(overlay) = ctx
                    .get_translation(&self.workspace_id, &source_node.id, &locale)
                    .await?
                {
                    ctx.store_translation(&self.workspace_id, &copied_node.id, &locale, overlay)
                        .await?;

                    tracing::debug!(
                        "Copied translation: locale={}, source={}, target={}",
                        locale,
                        source_node.id,
                        copied_node.id
                    );
                }
            } else {
                tracing::debug!(
                    "Skipping existing translation: locale={}, target={}",
                    locale,
                    copied_node.id
                );
            }
        }

        Ok(())
    }
}

// === Recursive helpers (free functions to avoid self-reference issues) ===

fn move_descendants<'a>(
    ctx: &'a dyn raisin_storage::transactional::TransactionalContext,
    workspace_id: &'a str,
    old_parent_path: &'a str,
    new_parent_path: &'a str,
    actor: &'a str,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), raisin_error::Error>> + Send + 'a>>
{
    Box::pin(async move {
        if let Some(parent) = ctx.get_node_by_path(workspace_id, new_parent_path).await? {
            for child_name in &parent.children {
                let old_child_path = format!("{}/{}", old_parent_path, child_name);
                let new_child_path = format!("{}/{}", new_parent_path, child_name);

                if let Some(mut child) = ctx.get_node_by_path(workspace_id, &old_child_path).await?
                {
                    tracing::debug!(
                        "  Moving descendant: {} -> {}",
                        old_child_path,
                        new_child_path
                    );

                    ctx.delete_path_index(workspace_id, &old_child_path).await?;

                    child.path = new_child_path.clone();
                    child.parent =
                        raisin_models::nodes::Node::extract_parent_name_from_path(&new_child_path);
                    child.updated_at = Some(chrono::Utc::now());
                    child.updated_by = Some(actor.to_string());

                    ctx.put_node(workspace_id, &child).await?;

                    if !child.children.is_empty() {
                        move_descendants(
                            ctx,
                            workspace_id,
                            &old_child_path,
                            &new_child_path,
                            actor,
                        )
                        .await?;
                    }
                }
            }
        }

        Ok(())
    })
}

fn rename_descendants<'a>(
    ctx: &'a dyn raisin_storage::transactional::TransactionalContext,
    workspace_id: &'a str,
    old_parent_path: &'a str,
    new_parent_path: &'a str,
    actor: &'a str,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), raisin_error::Error>> + Send + 'a>>
{
    Box::pin(async move {
        if let Some(parent) = ctx.get_node_by_path(workspace_id, new_parent_path).await? {
            for child_name in &parent.children {
                let old_child_path = format!("{}/{}", old_parent_path, child_name);
                let new_child_path = format!("{}/{}", new_parent_path, child_name);

                if let Some(mut child) = ctx.get_node_by_path(workspace_id, &old_child_path).await?
                {
                    tracing::debug!(
                        "  Renaming descendant: {} -> {}",
                        old_child_path,
                        new_child_path
                    );

                    ctx.delete_path_index(workspace_id, &old_child_path).await?;

                    child.path = new_child_path.clone();
                    child.updated_at = Some(chrono::Utc::now());
                    child.updated_by = Some(actor.to_string());

                    if let Some(old_parent_name) = old_parent_path.split('/').next_back() {
                        if child.parent.as_deref() == Some(old_parent_name) {
                            if let Some(new_parent_name) = new_parent_path.split('/').next_back() {
                                child.parent = Some(new_parent_name.to_string());
                            }
                        }
                    }

                    ctx.put_node(workspace_id, &child).await?;

                    if !child.children.is_empty() {
                        rename_descendants(
                            ctx,
                            workspace_id,
                            &old_child_path,
                            &new_child_path,
                            actor,
                        )
                        .await?;
                    }
                }
            }
        }

        Ok(())
    })
}
