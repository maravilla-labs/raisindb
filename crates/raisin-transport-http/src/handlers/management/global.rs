//! Global-level management operations
//!
//! These handlers manage instance-wide RocksDB operations

use axum::{extract::State, http::StatusCode, response::Json};
use serde::Serialize;

use crate::state::AppState;

/// Response for global operations
#[derive(Debug, Serialize)]
pub struct GlobalOpResponse {
    pub message: String,
    pub details: Option<serde_json::Value>,
}

/// Error response
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

// ============================================================================
// RocksDB Global Operations
// ============================================================================

/// Compact every column family through the bottommost level.
///
/// POST /api/admin/management/global/rocksdb/compact
///
/// Returns live SST bytes before and after. Compaction only returns space for
/// keys that were deleted — superseded revision history is live data until
/// history GC removes it (`POST /api/admin/management/global/gc`).
#[cfg(feature = "storage-rocksdb")]
pub async fn compact_rocksdb(
    State(state): State<AppState>,
) -> Result<Json<GlobalOpResponse>, (StatusCode, Json<ErrorResponse>)> {
    let storage = rocksdb_storage(&state)?;
    tracing::warn!("Global RocksDB compaction requested (admin action)");
    let started = std::time::Instant::now();
    let (before, after) = tokio::task::spawn_blocking(move || {
        let db = storage.db();
        let before = live_sst_bytes(db);
        for cf in raisin_rocksdb::all_column_family_names() {
            raisin_rocksdb::management::history_gc::compact_column_family(db, cf);
        }
        (before, live_sst_bytes(db))
    })
    .await
    .map_err(|e| internal(format!("compaction task failed: {e}")))?;

    Ok(Json(GlobalOpResponse {
        message: "Compaction complete".to_string(),
        details: Some(serde_json::json!({
            "live_sst_bytes_before": before,
            "live_sst_bytes_after": after,
            "reclaimed_bytes": before.saturating_sub(after),
            "duration_ms": started.elapsed().as_millis() as u64,
        })),
    }))
}

/// Request body for an instance-wide GC run.
#[cfg(feature = "storage-rocksdb")]
#[derive(Debug, Default, serde::Deserialize)]
pub struct GlobalGcRequest {
    /// Report what would be removed without removing anything.
    #[serde(default)]
    pub dry_run: bool,
    /// Compact afterwards (default true).
    #[serde(default)]
    pub compact: Option<bool>,
}

/// Run history GC, the orphaned-blob sweep, job-history cleanup and (with
/// replication off) the operation-log purge across the whole instance, each
/// branch under its stored or default retention policy.
///
/// POST /api/admin/management/global/gc
///
/// Deliberately takes no retention override: an instance-wide run must not let
/// one caller shorten every other tenant's history. Per-repository overrides go
/// through `POST /api/admin/management/database/{tenant}/{repo}/history/gc`.
#[cfg(feature = "storage-rocksdb")]
pub async fn run_global_history_gc(
    State(state): State<AppState>,
    body: Option<Json<GlobalGcRequest>>,
) -> Result<
    Json<raisin_rocksdb::management::history_gc::GcRunOutcome>,
    (StatusCode, Json<ErrorResponse>),
> {
    use raisin_rocksdb::management::history_gc;

    let storage = rocksdb_storage(&state)?;
    let req = body.map(|Json(b)| b).unwrap_or_default();
    let mut opts = history_gc::configured_options(&storage);
    opts.dry_run = req.dry_run;
    if let Some(compact) = req.compact {
        opts.compact = compact;
    }
    tracing::warn!(
        dry_run = opts.dry_run,
        "Global history GC requested (admin action)"
    );
    history_gc::run_gc_and_sweep_blobs(storage, state.bin.as_ref(), opts)
        .await
        .map(Json)
        .map_err(|e| internal(format!("History GC failed: {e}")))
}

/// Create a backup of the entire RocksDB instance
///
/// POST /api/admin/management/global/rocksdb/backup
#[cfg(feature = "storage-rocksdb")]
pub async fn backup_rocksdb(
    State(_state): State<AppState>,
) -> Result<Json<GlobalOpResponse>, (StatusCode, Json<ErrorResponse>)> {
    tracing::info!("Starting global RocksDB backup");

    // TODO: Implement RocksDB backup
    // This should use RocksDB's backup engine to create a full backup

    Err((
        StatusCode::NOT_IMPLEMENTED,
        Json(ErrorResponse {
            error: "Global RocksDB backup not yet implemented".to_string(),
        }),
    ))
}

/// Per-column-family size statistics.
///
/// GET /api/admin/management/global/rocksdb/stats
#[cfg(feature = "storage-rocksdb")]
pub async fn get_rocksdb_stats(
    State(state): State<AppState>,
) -> Result<Json<GlobalOpResponse>, (StatusCode, Json<ErrorResponse>)> {
    let storage = rocksdb_storage(&state)?;
    let details = tokio::task::spawn_blocking(move || {
        let db = storage.db();
        let mut families = serde_json::Map::new();
        let mut total_live = 0u64;
        let mut total_sst = 0u64;
        for cf in raisin_rocksdb::all_column_family_names() {
            let live = cf_int(db, cf, "rocksdb.live-sst-files-size");
            let sst = cf_int(db, cf, "rocksdb.total-sst-files-size");
            total_live += live;
            total_sst += sst;
            families.insert(
                cf.to_string(),
                serde_json::json!({
                    "live_sst_bytes": live,
                    "total_sst_bytes": sst,
                    "estimate_live_data_bytes": cf_int(db, cf, "rocksdb.estimate-live-data-size"),
                    "estimate_num_keys": cf_int(db, cf, "rocksdb.estimate-num-keys"),
                    "estimate_pending_compaction_bytes":
                        cf_int(db, cf, "rocksdb.estimate-pending-compaction-bytes"),
                    "memtable_bytes": cf_int(db, cf, "rocksdb.cur-size-all-mem-tables"),
                }),
            );
        }
        serde_json::json!({
            "live_sst_bytes": total_live,
            "total_sst_bytes": total_sst,
            "column_families": families,
        })
    })
    .await
    .map_err(|e| internal(format!("stats task failed: {e}")))?;

    Ok(Json(GlobalOpResponse {
        message: "RocksDB statistics".to_string(),
        details: Some(details),
    }))
}

#[cfg(feature = "storage-rocksdb")]
fn rocksdb_storage(
    state: &AppState,
) -> Result<std::sync::Arc<raisin_rocksdb::RocksDBStorage>, (StatusCode, Json<ErrorResponse>)> {
    state
        .rocksdb_storage
        .clone()
        .ok_or_else(|| internal("RocksDB storage not initialized".to_string()))
}

#[cfg(feature = "storage-rocksdb")]
fn internal(error: String) -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse { error }),
    )
}

#[cfg(feature = "storage-rocksdb")]
fn cf_int(db: &rocksdb::DB, cf: &str, property: &str) -> u64 {
    db.cf_handle(cf)
        .and_then(|h| db.property_int_value_cf(h, property).ok().flatten())
        .unwrap_or(0)
}

#[cfg(feature = "storage-rocksdb")]
fn live_sst_bytes(db: &rocksdb::DB) -> u64 {
    raisin_rocksdb::all_column_family_names()
        .into_iter()
        .map(|cf| cf_int(db, cf, "rocksdb.live-sst-files-size"))
        .sum()
}

// Stub implementations for non-rocksdb feature
#[cfg(not(feature = "storage-rocksdb"))]
pub async fn compact_rocksdb() -> (StatusCode, &'static str) {
    (StatusCode::NOT_IMPLEMENTED, "RocksDB feature not enabled")
}

#[cfg(not(feature = "storage-rocksdb"))]
pub async fn backup_rocksdb() -> (StatusCode, &'static str) {
    (StatusCode::NOT_IMPLEMENTED, "RocksDB feature not enabled")
}

#[cfg(not(feature = "storage-rocksdb"))]
pub async fn get_rocksdb_stats() -> (StatusCode, &'static str) {
    (StatusCode::NOT_IMPLEMENTED, "RocksDB feature not enabled")
}
