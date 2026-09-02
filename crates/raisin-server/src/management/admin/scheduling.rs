//! Per-tenant fair-share scheduling weight — the operator's only knob on
//! whose jobs run first.
//!
//! The weight is a RATIO of credit per scheduling round, not a precedence:
//! with weights 4 and 1 a round serves roughly four of the first tenant's jobs
//! per one of the second's, and the second still advances every round. See
//! `raisin_rocksdb::jobs::fair` for the guarantees and the incident behind
//! them.
//!
//! **This API takes a bare integer and nothing else.** RaisinDB is sold as a
//! standalone product, so it has no notion of a plan, a tier or a customer
//! segment; whoever operates the server decides what "4" means and pushes the
//! number. Adding a `tier` or `plan` field here would put one operator's
//! commercial model into everyone's database.
//!
//! Superadmin-only, because a weight is inherently cross-tenant: raising one
//! tenant's share lowers everyone else's, so it is not a decision a tenant can
//! be allowed to make about itself.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use raisin_transport_http::state::AppState;
use serde::{Deserialize, Serialize};

use crate::management::types::ApiResponse;

#[derive(Debug, Deserialize)]
pub struct SetSchedulingRequest {
    /// Credit per scheduling round, `1..=64`. Zero is refused, not clamped —
    /// see below.
    pub weight: u32,
}

#[derive(Debug, Serialize)]
pub struct SchedulingResponse {
    pub tenant_id: String,
    pub weight: u32,
    /// `false` when nothing was ever set for this tenant and `weight` is the
    /// default. The caller can tell "configured to 1" from "never configured",
    /// which the number alone cannot say.
    pub configured: bool,
}

/// `PUT /management/admin/tenants/{tenant_id}/scheduling`
///
/// Body `{"weight": 4}` → 200 `{"success":true,"data":{"tenant_id":…,
/// "weight":4,"configured":true}}`.
///
/// Idempotent: the same body twice is the same result. A weight outside
/// `1..=64` is a 400 naming the bound — in particular a weight of **0** is
/// refused rather than corrected, because a zero-weight queue is granted no
/// credit and is therefore served never: its jobs are accepted and then sit
/// forever, with nothing logged and nothing to see. The scheduler clamps
/// defensively as well, so a zero can never reach it by any route; this edge
/// refuses so the operator finds out they made a mistake.
///
/// The write persists BEFORE it takes effect. Storage failing means the
/// operator gets a 500 and the running scheduler is unchanged, which is
/// honest; the other order would run a weight no restart could reproduce.
pub async fn set_tenant_scheduling(
    State(app_state): State<AppState>,
    Path(tenant_id): Path<String>,
    Json(req): Json<SetSchedulingRequest>,
) -> (StatusCode, Json<ApiResponse<SchedulingResponse>>) {
    let weight = match raisin_rocksdb::validate_weight(req.weight) {
        Ok(w) => w,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ApiResponse::err(e.to_string())),
            );
        }
    };

    let db = app_state.storage().db();
    if let Err(e) = raisin_rocksdb::set_tenant_scheduling_weight(db, &tenant_id, weight) {
        tracing::error!(
            tenant_id = %tenant_id,
            error = %e,
            "Failed to persist tenant scheduling weight"
        );
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse::err(e.to_string())),
        );
    }

    tracing::warn!(
        tenant_id = %tenant_id,
        weight,
        "Superadmin set tenant scheduling weight"
    );

    (
        StatusCode::OK,
        Json(ApiResponse::ok(SchedulingResponse {
            tenant_id,
            weight,
            configured: true,
        })),
    )
}

/// `GET /management/admin/tenants/{tenant_id}/scheduling`
///
/// → 200 `{"success":true,"data":{"tenant_id":…,"weight":1,
/// "configured":false}}` for a tenant nobody has configured. Never 404s on an
/// unknown tenant: a weight is meaningful before the tenant has any data, and
/// the pushing side reads a 404 as "this server is too old to have the route",
/// which is a different thing entirely.
pub async fn get_tenant_scheduling(
    State(app_state): State<AppState>,
    Path(tenant_id): Path<String>,
) -> (StatusCode, Json<ApiResponse<SchedulingResponse>>) {
    let db = app_state.storage().db();
    match raisin_rocksdb::get_tenant_scheduling_weight(db, &tenant_id) {
        Ok(stored) => (
            StatusCode::OK,
            Json(ApiResponse::ok(SchedulingResponse {
                tenant_id,
                weight: stored.unwrap_or(raisin_rocksdb::DEFAULT_TENANT_WEIGHT),
                configured: stored.is_some(),
            })),
        ),
        Err(e) => {
            tracing::error!(
                tenant_id = %tenant_id,
                error = %e,
                "Failed to read tenant scheduling weight"
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse::err(e.to_string())),
            )
        }
    }
}
