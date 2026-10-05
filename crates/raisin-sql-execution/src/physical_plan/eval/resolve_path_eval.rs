//! `RESOLVE_PATH(workspace, locale, path) -> id` (plan Phase 12): a node id by
//! its localized URL path, through the storage's localized name lookup (the
//! same one `NodeService::resolve_localized_path` and the HTTP / WS surfaces
//! call). NULL when nothing resolves, the node is hidden in the locale, or
//! the caller cannot read it — the same answer for all three.

use crate::physical_plan::executor::{ExecutionContext, Row};
use raisin_error::Error;
use raisin_models::permissions::PermissionScope;
use raisin_sql::analyzer::{Literal, TypedExpr};
use raisin_storage::{NodeRepository, Storage, StorageScope};

use super::core::eval_expr;

fn text_arg(args: &[TypedExpr], i: usize, row: &Row) -> Result<Option<String>, Error> {
    match eval_expr(&args[i], row)? {
        Literal::Text(s) => Ok(Some(s)),
        Literal::Null => Ok(None),
        other => Err(Error::Validation(format!(
            "RESOLVE_PATH argument {} must be text, got {:?}",
            i + 1,
            other
        ))),
    }
}

pub(super) async fn eval_resolve_path<S: Storage>(
    args: &[TypedExpr],
    row: &Row,
    ctx: &ExecutionContext<S>,
) -> Result<Literal, Error> {
    if args.len() != 3 {
        return Err(Error::Validation(
            "RESOLVE_PATH requires (workspace, locale, path)".to_string(),
        ));
    }
    let (Some(workspace), Some(locale), Some(path)) = (
        text_arg(args, 0, row)?,
        text_arg(args, 1, row)?,
        text_arg(args, 2, row)?,
    ) else {
        return Ok(Literal::Null);
    };
    let source = ctx.storage.localized_names().ok_or_else(|| {
        Error::Validation("RESOLVE_PATH is not supported by this storage backend".to_string())
    })?;
    let snapshot = ctx.statement_snapshot().await?;
    let scope = StorageScope::new(&ctx.tenant_id, &ctx.repo_id, &ctx.branch, &workspace);
    let Some(found) = source.resolve(scope, &locale, &path, Some(&snapshot))? else {
        return Ok(Literal::Null);
    };
    if let Some(auth) = &ctx.auth_context {
        let Some(node) = ctx
            .storage
            .nodes()
            .get(scope, &found.node_id, Some(&snapshot))
            .await?
        else {
            return Ok(Literal::Null);
        };
        let permission_scope = PermissionScope::new(workspace.as_str(), &*ctx.branch);
        let readable = raisin_core::services::rls_filter::filter_node_with_graph(
            &*ctx.storage,
            node,
            auth,
            &permission_scope,
            raisin_storage::BranchScope::new(&ctx.tenant_id, &ctx.repo_id, &ctx.branch),
            &snapshot,
        )
        .await;
        if readable.is_none() {
            return Ok(Literal::Null);
        }
    }
    Ok(Literal::Text(found.node_id))
}
