// SPDX-License-Identifier: BSL-1.1

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use raisin_models as models;
use raisin_models::auth::AuthContext;
use raisin_query as rquery;

use crate::{
    errors::internal_err,
    state::AppState,
    types::{Page, PageMeta, QueryRequest},
};

pub async fn post_query(
    State(state): State<AppState>,
    Path((repo, branch, ws)): Path<(String, String, String)>,
    auth: Option<Extension<AuthContext>>,
    Json(req): Json<QueryRequest>,
) -> Result<Json<Page<models::nodes::Node>>, (StatusCode, Json<crate::types::ErrorBody>)> {
    let tenant_id = "default"; // TODO: Extract from middleware/auth
    let auth_context = auth.map(|Extension(ctx)| ctx);
    let nodes_svc = state.node_service_for_context(tenant_id, &repo, &branch, &ws, auth_context);

    // Precedence: path (exact match) over other filters; if path present, ignore type/parent.
    let mut items: Vec<models::nodes::Node> = if let Some(path) = req.path {
        let maybe = nodes_svc
            .get_by_path(&path)
            .await
            .map_err(|_| internal_err("get_by_path failed"))?;
        maybe.into_iter().collect()
    } else {
        // Build base set using the most restrictive available filter
        match (req.parent.as_ref(), req.node_type.as_ref()) {
            (Some(parent), Some(node_type)) => {
                let mut v = nodes_svc
                    .list_by_parent(parent)
                    .await
                    .map_err(|_| internal_err("list_by_parent failed"))?;
                v.retain(|n| n.node_type == *node_type);
                v
            }
            (Some(parent), None) => nodes_svc
                .list_by_parent(parent)
                .await
                .map_err(|_| internal_err("list_by_parent failed"))?,
            (None, Some(node_type)) => nodes_svc
                .list_by_type(node_type)
                .await
                .map_err(|_| internal_err("list_by_type failed"))?,
            (None, None) => {
                let body = Json(crate::types::ErrorBody {
                    error: "BadRequest".into(),
                    message: "Provide one of: path, parent, nodeType".into(),
                });
                return Err((StatusCode::BAD_REQUEST, body));
            }
        }
    };

    // stable order by path then id to make pagination predictable
    items.sort_by(|a, b| a.path.cmp(&b.path).then(a.id.cmp(&b.id)));
    let offset = req.offset.unwrap_or(0);
    let limit = req.limit.unwrap_or(usize::MAX);
    let start = offset.min(items.len());
    let end = (start.saturating_add(limit)).min(items.len());
    let total = items.len();
    let slice = items[start..end].to_vec();
    let next_offset = if end < total { Some(end) } else { None };
    Ok(Json(Page {
        items: slice,
        page: PageMeta {
            total,
            limit,
            offset,
            next_offset,
        },
    }))
}

pub async fn post_query_dsl(
    State(state): State<AppState>,
    Path((repo, branch, ws)): Path<(String, String, String)>,
    auth: Option<Extension<AuthContext>>,
    Json(q): Json<rquery::NodeSearchQuery>,
) -> Result<Json<Page<models::nodes::Node>>, (StatusCode, Json<crate::types::ErrorBody>)> {
    let tenant_id = "default"; // TODO: Extract from middleware/auth
    let auth_context = auth.map(|Extension(ctx)| ctx);
    let nodes_svc = state.node_service_for_context(tenant_id, &repo, &branch, &ws, auth_context);

    // fetch all nodes in workspace (using list_root as list_all is deprecated)
    // TODO: Use deep_children_array("/", max_depth) for full tree traversal if needed
    let all = nodes_svc
        .list_root()
        .await
        .map_err(|_| internal_err("list_root failed"))?;

    let refs: Vec<_> = all.iter().collect();
    let matched = rquery::eval_query(refs.iter().copied(), &q);
    // convert &Node to owned Node
    let mut items: Vec<models::nodes::Node> = matched.into_iter().cloned().collect();

    // if order_by not specified, keep stable order by path,id for consistent pagination
    items.sort_by(|a, b| a.path.cmp(&b.path).then(a.id.cmp(&b.id)));
    let limit = q.limit.unwrap_or(usize::MAX);
    let offset = q.offset.unwrap_or(0);
    let total = items.len();
    let start = offset.min(total);
    let end = (start.saturating_add(limit)).min(total);
    let next_offset = if end < total { Some(end) } else { None };
    Ok(Json(Page {
        items: items[start..end].to_vec(),
        page: PageMeta {
            total,
            limit,
            offset,
            next_offset,
        },
    }))
}
