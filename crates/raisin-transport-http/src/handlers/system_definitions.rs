// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! Management endpoints for the system-definition stack.
//!
//! These endpoints let an operator change which built-in NodeTypes, Workspaces
//! and packages a *running* server offers, without rebuilding or restarting it:
//!
//! - `GET  /api/management/system-definitions` — which layer each definition resolves from
//! - `POST /api/management/system-definitions/reload` — re-read the overlay directory
//! - `GET  /api/management/system-definitions/registries` — configured registries
//! - `GET  /api/management/system-definitions/registries/{name}` — that registry's catalog
//! - `POST /api/management/system-definitions/registries/{name}/fetch` — download artifacts
//!
//! None of these write into a repository. Fetching lands artifacts in the
//! overlay directory; from there they surface as ordinary pending system
//! updates and are applied through the existing system-updates endpoints. That
//! separation is deliberate: pulling from a remote and mutating tenant schemas
//! should never be one irreversible step.

use axum::{
    extract::{Path, State},
    Json,
};
use raisin_core::definitions::{RegistryClient, RegistryEntry};
use serde::{Deserialize, Serialize};

use crate::{error::ApiError, state::AppState};

/// Where one resolved definition comes from.
#[derive(Debug, Serialize)]
pub struct DefinitionOriginInfo {
    /// Resource name.
    pub name: String,
    /// Winning layer.
    pub layer: String,
    /// Layers this definition shadows, lowest first.
    pub shadowed: Vec<String>,
}

/// The server's current definition stack.
#[derive(Debug, Serialize)]
pub struct SystemDefinitionsResponse {
    /// Layer names, lowest precedence first.
    pub layers: Vec<String>,
    /// Resolved overlay directory (may not exist).
    pub overlay_dir: String,
    /// Whether that directory currently exists.
    pub overlay_present: bool,
    /// Startup auto-apply policy.
    pub auto_apply: String,
    /// Every resolved definition and its winning layer.
    pub definitions: Vec<DefinitionOriginInfo>,
}

/// A configured registry, without its credentials.
#[derive(Debug, Serialize)]
pub struct RegistryInfo {
    /// Registry name.
    pub name: String,
    /// Index URL.
    pub url: String,
    /// Whether it may be contacted.
    pub enabled: bool,
}

/// Result of a registry fetch.
#[derive(Debug, Serialize)]
pub struct FetchResponse {
    /// Artifacts written into the overlay.
    pub fetched: Vec<String>,
    /// Human-readable summary.
    pub message: String,
}

/// Which artifacts to fetch. An empty list means "everything in the index".
#[derive(Debug, Default, Deserialize)]
pub struct FetchRequest {
    /// Artifact names to download.
    #[serde(default)]
    pub resources: Vec<String>,
}

/// `GET /api/management/system-definitions`
pub async fn get_system_definitions(
    State(state): State<AppState>,
) -> Result<Json<SystemDefinitionsResponse>, ApiError> {
    let resolver = state.definitions().await;
    let (config, overlay_dir) = state.system_definitions_config().await;

    Ok(Json(SystemDefinitionsResponse {
        layers: resolver
            .layer_names()
            .into_iter()
            .map(String::from)
            .collect(),
        overlay_present: overlay_dir.is_dir(),
        overlay_dir: overlay_dir.display().to_string(),
        auto_apply: format!("{:?}", config.auto_apply),
        definitions: resolver
            .origins()
            .into_iter()
            .map(|o| DefinitionOriginInfo {
                name: o.name,
                layer: o.layer,
                shadowed: o.shadowed,
            })
            .collect(),
    }))
}

/// `POST /api/management/system-definitions/reload`
///
/// Re-reads the overlay directory. Nothing is written to any repository — the
/// caller then reviews pending updates per repo and applies them.
pub async fn reload_system_definitions(
    State(state): State<AppState>,
) -> Result<Json<SystemDefinitionsResponse>, ApiError> {
    state.reload_system_definitions().await;
    tracing::info!("System definitions reloaded from overlay directory");
    get_system_definitions(State(state)).await
}

/// `GET /api/management/system-definitions/registries`
pub async fn list_registries(
    State(state): State<AppState>,
) -> Result<Json<Vec<RegistryInfo>>, ApiError> {
    let (config, _) = state.system_definitions_config().await;
    Ok(Json(
        config
            .registries
            .iter()
            .map(|r| RegistryInfo {
                name: r.name.clone(),
                url: r.url.clone(),
                enabled: r.enabled,
            })
            .collect(),
    ))
}

/// `GET /api/management/system-definitions/registries/{name}`
///
/// Fetches and returns the registry's catalog. This is the only read that
/// contacts the network, and only for an enabled registry.
pub async fn get_registry_catalog(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<Vec<RegistryEntry>>, ApiError> {
    let client = registry_client(&state, &name).await?;
    let index = client
        .fetch_index()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(index.entries))
}

/// `POST /api/management/system-definitions/registries/{name}/fetch`
///
/// Downloads the requested artifacts, verifies each declared SHA256, writes
/// them into the overlay directory and reloads the stack.
pub async fn fetch_from_registry(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(req): Json<FetchRequest>,
) -> Result<Json<FetchResponse>, ApiError> {
    let client = registry_client(&state, &name).await?;
    let (_, overlay_dir) = state.system_definitions_config().await;

    let index = client
        .fetch_index()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let fetched = client
        .fetch_entries(&index, &req.resources, &overlay_dir)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    // Make the freshly-downloaded definitions part of the live stack so they
    // show up as pending updates immediately.
    state.reload_system_definitions().await;

    Ok(Json(FetchResponse {
        message: format!(
            "Fetched {} artifact(s) from '{}' into {}. Review and apply them per repository \
             from system updates.",
            fetched.len(),
            name,
            overlay_dir.display()
        ),
        fetched,
    }))
}

async fn registry_client(state: &AppState, name: &str) -> Result<RegistryClient, ApiError> {
    let (config, _) = state.system_definitions_config().await;
    let registry = config
        .registries
        .iter()
        .find(|r| r.name == name)
        .cloned()
        .ok_or_else(|| {
            ApiError::not_found(format!("No registry named '{}' is configured", name))
        })?;
    Ok(RegistryClient::new(registry))
}
