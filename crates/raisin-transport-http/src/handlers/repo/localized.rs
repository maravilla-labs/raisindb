// SPDX-License-Identifier: BSL-1.1

//! `GET /api/repository/{repo}/{branch}/head/{ws}/by-localized-path/{locale}/{*path}`
//! (plan Phase 12): a node by its localized URL, through the one core lookup
//! (`NodeService::resolve_localized_path`).
//!
//! Answers the translated node with `canonical_path`,
//! `canonical_localized_path`, `alternates` (hreflang; hidden and unreadable
//! locales omitted) and `redirect` — set when the request did not use the
//! canonical localized path, so a delivery layer can answer 301. Missing,
//! forbidden and hidden-in-locale are the same 404.

use axum::extract::{Extension, Json, Path, State};
use raisin_core::LocalizedNode;
use raisin_models::auth::AuthContext;
use raisin_storage::{BranchRepository, Storage};

use crate::{error::ApiError, middleware::TenantInfo, state::AppState};

pub async fn repo_get_by_localized_path(
    State(state): State<AppState>,
    Extension(tenant_info): Extension<TenantInfo>,
    Path((repo, branch, ws, locale, path)): Path<(String, String, String, String, String)>,
    auth: Option<Extension<AuthContext>>,
) -> Result<Json<LocalizedNode>, ApiError> {
    let tenant_id = tenant_info.tenant_id.as_str();
    let auth_context = auth.map(|Extension(ctx)| ctx);
    let mut nodes_svc =
        state.node_service_for_context(tenant_id, &repo, &branch, &ws, auth_context);
    // Bound to the branch HEAD for snapshot isolation, like the other reads.
    if let Some(info) = state
        .storage()
        .branches()
        .get_branch(tenant_id, &repo, &branch)
        .await?
    {
        nodes_svc = nodes_svc.at_revision(info.head);
    }
    let found = nodes_svc
        .resolve_localized_path(&locale, &path)
        .await?
        .ok_or_else(|| ApiError::node_not_found(format!("/{locale}/{path}")))?;
    Ok(Json(found))
}
