//! Graph Cache Management API
//!
//! HTTP endpoints for managing graph algorithm configurations and cache status:
//! - GET /management/graph-cache/{repo}/status - list all configs with status
//! - POST /management/graph-cache/{repo}/{config_id}/recompute - trigger immediate recomputation
//! - POST /management/graph-cache/{repo}/{config_id}/mark-stale - mark for next tick
//! - GET /management/graph-cache/{repo}/stream - SSE endpoint for live updates
//!
//! Note: Config CRUD (create/update/delete) is done via the standard nodes API
//! at /api/repository/{repo}/main/head/raisin:access_control/graph-config/{id}

mod handlers;
mod helpers;
mod state;
mod types;

pub use handlers::{
    get_graph_cache_status, graph_cache_events_stream, mark_stale, trigger_recompute,
};
pub use state::{start_graph_cache_background_task, GraphCacheState};
pub use types::{ApiResponse, ConfigStatusResponse, GraphCacheEvent};
