//! What this server can actually DO with a binary asset.
//!
//! # Why this endpoint exists
//!
//! A rejected plugin leaves a server that boots perfectly and has silently lost
//! `raisin.media.*` for every tenant. The only previous signal was one WARN line
//! in the journal at startup, which no deploy asserts on and nobody reads weeks
//! later when a customer's search comes back empty. An ABI bump between the
//! server and a plugin lib produces exactly that, and it is invisible from every
//! surface an operator actually looks at.
//!
//! So the same three things the startup report logs are served here: which
//! plugins loaded and what methods they service, which plugin FILES the loader
//! refused and why, and the resolved per-media-kind capability table. All read
//! from `raisin_functions`' registry — the one the dispatcher itself reads, so
//! this cannot disagree with what a call will do.
//!
//! # It is process-wide, not repo-scoped
//!
//! Plugins are loaded once per process, before any function runs, and
//! registration is append-only into a process-global. The answer therefore
//! cannot change without a restart, and it is identical for every tenant and
//! repository on the box. Presenting it under a repo would imply a per-repo
//! knob that does not exist.

use axum::Json;
use serde::Serialize;

use crate::error::ApiError;

/// One plugin file the loader refused, and why.
#[derive(Debug, Serialize)]
pub struct PluginRejectionResponse {
    /// Path of the shared library that was skipped.
    pub path: String,
    /// ABI mismatch, missing symbol, bad methods JSON, dlopen error…
    pub reason: String,
}

/// The plugin/capability report.
#[derive(Debug, Serialize)]
pub struct PluginsResponse {
    /// Plugins that loaded, with the fully-qualified methods each services.
    pub plugins: Vec<raisin_functions::plugin::PluginManifestEntry>,
    /// Every method name serviced by some registered plugin, sorted.
    pub methods: Vec<String>,
    /// Plugin files the loader REFUSED. Non-empty here is the failure this
    /// endpoint exists for: the server is up, and a capability the estate
    /// assumes it has is gone.
    pub rejected: Vec<PluginRejectionResponse>,
    /// Per media kind: plugin, core, or unsupported — resolved against the
    /// registry above rather than restated, so the two cannot drift.
    pub capabilities: Vec<raisin_functions::media_capabilities::CapabilityRow>,
    /// Whether the process installed its capability probe at startup.
    ///
    /// False means the task planner is answering "no" to everything by
    /// default rather than by observation — a wiring fault, not an empty
    /// plugin directory, and the two look identical from the plan alone.
    pub capability_probe_installed: bool,
}

/// Report the loaded plugins, the refused ones, and the resolved capabilities.
///
/// GET /api/admin/management/plugins
#[axum::debug_handler]
pub async fn list_plugins() -> Result<Json<PluginsResponse>, ApiError> {
    let rejected = raisin_functions::plugin::rejected_plugins()
        .into_iter()
        .map(|r| PluginRejectionResponse {
            path: r.path,
            reason: r.reason,
        })
        .collect();

    Ok(Json(PluginsResponse {
        plugins: raisin_functions::plugin::plugin_manifest(),
        methods: raisin_functions::plugin::registered_plugin_methods(),
        rejected,
        capabilities: raisin_functions::media_capabilities::capability_report(),
        capability_probe_installed: raisin_ai::capability_probe_installed(),
    }))
}
