//! Copy and publishing workflow methods for NodeService
//!
//! This module handles:
//! - Copying nodes (single or entire trees)
//! - Publishing/unpublishing nodes
//! - Publishing/unpublishing entire trees

mod publish;

use raisin_error::Result;
use raisin_models as models;
use raisin_models::permissions::Operation;
use raisin_storage::{NodeRepository, Storage};

use super::NodeService;

impl<S: Storage + raisin_storage::transactional::TransactionalStorage> NodeService<S> {
    /// Copies a single node to a new parent location.
    ///
    /// Uses a transaction to atomically copy both the node and all its translations.
    /// Does not copy children - use `copy_node_tree` for recursive copy.
    pub async fn copy_node(
        &self,
        source_path: &str,
        target_parent: &str,
        new_name: Option<&str>,
    ) -> Result<models::nodes::Node> {
        use raisin_storage::transactional::TransactionalContext;

        let actor = self
            .auth_context
            .as_ref()
            .map(|ctx| ctx.actor_id())
            .unwrap_or_else(|| "system".to_string());

        let ctx = self.storage.begin_context().await?;

        if let Some(auth) = &self.auth_context {
            ctx.set_auth_context(auth.clone())?;
        }

        ctx.set_tenant_repo(&self.tenant_id, &self.repo_id)?;
        ctx.set_branch(&self.branch)?;
        ctx.set_actor(&actor)?;
        ctx.set_message(&format!(
            "Copy node from {} to {}",
            source_path, target_parent
        ))?;

        let source_node = ctx
            .get_node_by_path(&self.workspace_id, source_path)
            .await?
            .ok_or_else(|| {
                raisin_error::Error::NotFound(format!(
                    "Source node not found at path: {}",
                    source_path
                ))
            })?;

        if !self.check_rls_permission(&source_node, Operation::Read) {
            return Err(raisin_error::Error::PermissionDenied(format!(
                "Permission denied: cannot read source node at path '{}'",
                source_path
            )));
        }

        let copied_name = new_name.unwrap_or(&source_node.name);
        let target_path = if target_parent == "/" || target_parent.is_empty() {
            format!("/{}", copied_name)
        } else {
            format!("{}/{}", target_parent, copied_name)
        };

        if !self.check_rls_create_permission(&target_path, &source_node.node_type) {
            return Err(raisin_error::Error::PermissionDenied(format!(
                "Permission denied: cannot create node at path '{}'",
                target_path
            )));
        }

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
        copied_node.parent = models::nodes::Node::extract_parent_name_from_path(&target_path);
        copied_node.created_at = Some(chrono::Utc::now());
        copied_node.created_by = Some(actor.clone());
        copied_node.updated_at = Some(chrono::Utc::now());
        copied_node.updated_by = Some(actor.clone());
        copied_node.published_at = None;
        copied_node.published_by = None;
        copied_node.has_children = None;

        ctx.add_node(&self.workspace_id, &copied_node).await?;

        let source_locales = ctx
            .list_translations_for_node(&self.workspace_id, &source_node.id)
            .await?;

        for locale in source_locales {
            if let Some(overlay) = ctx
                .get_translation(&self.workspace_id, &source_node.id, &locale)
                .await?
            {
                ctx.store_translation(&self.workspace_id, &copied_node.id, &locale, overlay)
                    .await?;
            }
        }

        ctx.commit().await?;

        Ok(copied_node)
    }

    /// Recursively copies a node and all its descendants.
    ///
    /// Uses a transaction to atomically copy the entire tree including all translations.
    pub async fn copy_node_tree(
        &self,
        source_path: &str,
        target_parent: &str,
        new_name: Option<&str>,
    ) -> Result<models::nodes::Node> {
        use raisin_storage::transactional::TransactionalContext;

        let actor = self
            .auth_context
            .as_ref()
            .map(|ctx| ctx.actor_id())
            .unwrap_or_else(|| "system".to_string());

        let ctx = self.storage.begin_context().await?;

        if let Some(auth) = &self.auth_context {
            ctx.set_auth_context(auth.clone())?;
        }

        ctx.set_tenant_repo(&self.tenant_id, &self.repo_id)?;
        ctx.set_branch(&self.branch)?;
        ctx.set_actor(&actor)?;
        ctx.set_message(&format!(
            "Copy tree from {} to {}",
            source_path, target_parent
        ))?;

        let source_node = ctx
            .get_node_by_path(&self.workspace_id, source_path)
            .await?
            .ok_or_else(|| {
                raisin_error::Error::NotFound(format!(
                    "Source node not found at path: {}",
                    source_path
                ))
            })?;

        if !self.check_rls_permission(&source_node, Operation::Read) {
            return Err(raisin_error::Error::PermissionDenied(format!(
                "Permission denied: cannot read source node at path '{}'",
                source_path
            )));
        }

        let copied_name = new_name.unwrap_or(&source_node.name);
        let target_path = if target_parent == "/" || target_parent.is_empty() {
            format!("/{}", copied_name)
        } else {
            format!("{}/{}", target_parent, copied_name)
        };

        if !self.check_rls_create_permission(&target_path, &source_node.node_type) {
            return Err(raisin_error::Error::PermissionDenied(format!(
                "Permission denied: cannot create node at path '{}'",
                target_path
            )));
        }

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

        // Pre-check all descendants for read permission (atomic copy semantics)
        let descendants = self
            .storage
            .nodes()
            .deep_children_flat(self.scope(), source_path, 100, self.revision.as_ref())
            .await?;

        for descendant in &descendants {
            if !self.check_rls_permission(descendant, Operation::Read) {
                return Err(raisin_error::Error::PermissionDenied(format!(
                    "Permission denied: cannot read descendant node at path '{}'",
                    descendant.path
                )));
            }
        }

        fn copy_tree_recursive<'a>(
            ctx: &'a dyn TransactionalContext,
            workspace_id: &'a str,
            source_node: &'a models::nodes::Node,
            target_path: &'a str,
            new_name: &'a str,
            actor: &'a str,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<models::nodes::Node>> + Send + 'a>,
        > {
            Box::pin(async move {
                let mut copied_node = source_node.clone();
                copied_node.id = nanoid::nanoid!();
                copied_node.name = new_name.to_string();
                copied_node.path = target_path.to_string();
                copied_node.parent =
                    models::nodes::Node::extract_parent_name_from_path(target_path);
                copied_node.created_at = Some(chrono::Utc::now());
                copied_node.created_by = Some(actor.to_string());
                copied_node.updated_at = Some(chrono::Utc::now());
                copied_node.updated_by = Some(actor.to_string());
                copied_node.published_at = None;
                copied_node.published_by = None;
                copied_node.has_children = None;

                ctx.add_node(workspace_id, &copied_node).await?;

                let source_locales = ctx
                    .list_translations_for_node(workspace_id, &source_node.id)
                    .await?;

                for locale in source_locales {
                    if let Some(overlay) = ctx
                        .get_translation(workspace_id, &source_node.id, &locale)
                        .await?
                    {
                        ctx.store_translation(workspace_id, &copied_node.id, &locale, overlay)
                            .await?;
                    }
                }

                let children = ctx.list_children(workspace_id, &source_node.path).await?;
                for source_child in children {
                    let target_child_path = format!("{}/{}", copied_node.path, source_child.name);

                    copy_tree_recursive(
                        ctx,
                        workspace_id,
                        &source_child,
                        &target_child_path,
                        &source_child.name,
                        actor,
                    )
                    .await?;
                }

                Ok(copied_node)
            })
        }

        let root_node = copy_tree_recursive(
            ctx.as_ref(),
            &self.workspace_id,
            &source_node,
            &target_path,
            copied_name,
            &actor,
        )
        .await?;

        ctx.commit().await?;

        Ok(root_node)
    }

    /// Flexible copy that handles both path modes.
    ///
    /// Mode 1 (explicit parent + name): `target_path` is parent, `new_name` provides name.
    /// Mode 2 (full path): `target_path` is full destination path, `new_name` is None.
    pub async fn copy_node_flexible(
        &self,
        source_path: &str,
        target_path: &str,
        new_name: Option<&str>,
    ) -> Result<models::nodes::Node> {
        let (target_parent, final_name) = if let Some(name) = new_name {
            (target_path.to_string(), Some(name.to_string()))
        } else {
            self.parse_target_path(target_path)?
        };

        self.copy_node(source_path, &target_parent, final_name.as_deref())
            .await
    }

    /// Flexible tree copy that handles both path modes.
    pub async fn copy_node_tree_flexible(
        &self,
        source_path: &str,
        target_path: &str,
        new_name: Option<&str>,
    ) -> Result<models::nodes::Node> {
        let (target_parent, final_name) = if let Some(name) = new_name {
            (target_path.to_string(), Some(name.to_string()))
        } else {
            self.parse_target_path(target_path)?
        };

        self.copy_node_tree(source_path, &target_parent, final_name.as_deref())
            .await
    }

    /// Parse target path into (parent, optional_name) tuple.
    fn parse_target_path(&self, target_path: &str) -> Result<(String, Option<String>)> {
        if let Some(idx) = target_path.rfind('/') {
            let parent = if idx == 0 {
                "/".to_string()
            } else {
                target_path[..idx].to_string()
            };
            let name = if idx < target_path.len() - 1 {
                Some(target_path[idx + 1..].to_string())
            } else {
                None
            };
            Ok((parent, name))
        } else {
            Ok(("/".to_string(), Some(target_path.to_string())))
        }
    }

    /// Public method to parse copy target path for transaction builders.
    pub fn parse_copy_target(&self, target_path: &str) -> Result<(String, Option<String>)> {
        self.parse_target_path(target_path)
    }
}
