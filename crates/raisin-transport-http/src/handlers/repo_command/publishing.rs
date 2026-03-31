// SPDX-License-Identifier: BSL-1.1
//! Publishing operations: publish, unpublish, publish_tree, unpublish_tree.

use raisin_storage::Storage;

use super::common::{CommandContext, CommandResult};

/// Handle the publish command.
pub async fn handle_publish<S: Storage>(ctx: &mut CommandContext<'_, S>) -> CommandResult {
    ctx.nodes_svc.publish(ctx.path).await?;
    CommandContext::<S>::ok_empty()
}

/// Handle the publish_tree command.
pub async fn handle_publish_tree<S: Storage>(ctx: &mut CommandContext<'_, S>) -> CommandResult {
    ctx.nodes_svc.publish_tree(ctx.path).await?;
    CommandContext::<S>::ok_empty()
}

/// Handle the unpublish command.
pub async fn handle_unpublish<S: Storage>(ctx: &mut CommandContext<'_, S>) -> CommandResult {
    ctx.nodes_svc.unpublish(ctx.path).await?;
    CommandContext::<S>::ok_empty()
}

/// Handle the unpublish_tree command.
pub async fn handle_unpublish_tree<S: Storage>(ctx: &mut CommandContext<'_, S>) -> CommandResult {
    ctx.nodes_svc.unpublish_tree(ctx.path).await?;
    CommandContext::<S>::ok_empty()
}
