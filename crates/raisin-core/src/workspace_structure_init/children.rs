//! Iterative child creation logic for workspace initial structure.

use raisin_error::Result;
use raisin_models as models;
use raisin_storage::transactional::TransactionalContext;
use std::vec::IntoIter;

use super::{convert_properties, convert_translations};

pub(super) struct Frame {
    pub parent_path: Option<String>,
    pub children: IntoIter<models::nodes::types::initial_structure::InitialChild>,
    pub previous_order_key: String,
}

pub(super) async fn create_children_iterative(
    ctx: &dyn TransactionalContext,
    tenant_id: &str,
    repository_id: &str,
    _branch: &str,
    workspace_name: &str,
    initial_children: &[models::nodes::types::initial_structure::InitialChild],
) -> Result<()> {
    if initial_children.is_empty() {
        return Ok(());
    }

    #[allow(clippy::unnecessary_to_owned)]
    let mut stack = vec![Frame {
        parent_path: None,
        children: initial_children.to_vec().into_iter(),
        previous_order_key: String::new(),
    }];

    while let Some(mut frame) = stack.pop() {
        if let Some(child_def) = frame.children.next() {
            let parent_path = frame.parent_path.clone();
            let order_key = models::fractional_index::next_key(&frame.previous_order_key);
            frame.previous_order_key = order_key.clone();

            let sanitized_name = crate::sanitize_name(&child_def.name)?;

            let node_path = parent_path
                .as_ref()
                .map(|path| format!("{}/{}", path.trim_end_matches('/'), sanitized_name))
                .unwrap_or_else(|| format!("/{}", sanitized_name));

            // Check if node already exists using read-your-writes semantics
            let existing_node = ctx.get_node_by_path(workspace_name, &node_path).await?;

            if existing_node.is_some() {
                tracing::debug!(
                    "Node '{}' already exists at '{}' in workspace {}/{}/{}, skipping creation",
                    child_def.name,
                    node_path,
                    tenant_id,
                    repository_id,
                    workspace_name
                );
            } else {
                tracing::debug!(
                    "Creating node '{}' at '{}' for workspace {}/{}/{}",
                    child_def.name,
                    node_path,
                    tenant_id,
                    repository_id,
                    workspace_name
                );

                let properties = convert_properties(child_def.properties.as_ref());
                let translations = convert_translations(child_def.translations.as_ref());

                let parent_name = parent_path.as_ref().and_then(|path| {
                    let trimmed = path.trim_end_matches('/');
                    trimmed
                        .rsplit('/')
                        .find(|segment| !segment.is_empty())
                        .map(|segment| segment.to_string())
                });

                let node = models::nodes::Node {
                    id: nanoid::nanoid!(16),
                    name: sanitized_name.clone(),
                    path: node_path.clone(),
                    node_type: child_def.node_type.clone(),
                    archetype: child_def.archetype.clone(),
                    properties,
                    children: Vec::new(),
                    order_key: order_key.clone(),
                    has_children: None,
                    parent: parent_name,
                    version: 1,
                    created_at: None,
                    updated_at: None,
                    published_at: None,
                    published_by: None,
                    updated_by: None,
                    created_by: None,
                    translations,
                    tenant_id: Some(tenant_id.to_string()),
                    workspace: Some(workspace_name.to_string()),
                    owner_id: None,
                    relations: Vec::new(),
                };

                // Use add_node since these are new nodes (optimized path)
                ctx.add_node(workspace_name, &node).await?;
            }

            let has_more_siblings = frame.children.len() > 0;

            if has_more_siblings {
                stack.push(Frame {
                    parent_path: frame.parent_path.clone(),
                    children: frame.children,
                    previous_order_key: frame.previous_order_key.clone(),
                });
            }

            if let Some(nested_children) = child_def.children {
                if !nested_children.is_empty() {
                    stack.push(Frame {
                        parent_path: Some(node_path),
                        children: nested_children.into_iter(),
                        previous_order_key: String::new(),
                    });
                }
            }
        }
    }

    Ok(())
}
