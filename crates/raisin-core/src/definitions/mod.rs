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

//! Layered sources for built-in definitions (NodeTypes, Workspaces, packages).
//!
//! # Why layers
//!
//! Built-in definitions used to be reachable one way only: `include_dir!`-embedded
//! in the binary. A one-line schema fix — `raisin:Package` gaining `sync_policy`,
//! say — therefore cost a full release and a redeploy of every server.
//!
//! The layers here keep the embedded set as the guaranteed baseline (a server
//! with no config, no disk overlay and no network still has a complete,
//! self-consistent set of built-ins) and let higher layers override individual
//! definitions **by name**:
//!
//! | Layer | Precedence | Source |
//! |---|---|---|
//! | [`EmbeddedSource`] | lowest, always present | compiled into the binary |
//! | [`OverlaySource`] | above embedded | a directory on the server |
//! | registry cache | above embedded | artifacts fetched from a [`registry`] into the overlay dir |
//!
//! Resolution is whole-definition: the highest layer that defines a name wins
//! outright. There is no field-level merging, because a half-overridden schema
//! is one nobody wrote and nobody can reason about.
//!
//! A resolved definition carries the **winning layer's content hash**, so the
//! existing `system_updates` machinery (`check_pending_updates`, the admin
//! console's system-updates view, the apply endpoint) works on overlay
//! definitions with no special-casing.

mod config;
mod embedded;
mod overlay;
pub mod registry;
mod resolver;
mod source;

pub use config::{RegistryConfig, SystemDefinitionsConfig};
pub use embedded::EmbeddedSource;
pub use overlay::OverlaySource;
pub use registry::{RegistryClient, RegistryEntry, RegistryEntryKind, RegistryIndex};
pub use resolver::{DefinitionOrigin, DefinitionResolver};
pub use source::{DefinitionSource, PackageDefinition, PackageSource};

use std::sync::{Arc, RwLock};

/// The process-wide definition stack.
///
/// Several long-standing initialization paths — the `RepositoryCreated`
/// NodeType handler, workspace-service self-healing, the package install job —
/// call `nodetype_init::load_global_nodetypes()` directly and write whatever it
/// returns. If that kept reading the embedded YAML while the resync wrote
/// overlay definitions, those paths would silently *revert* an overlay
/// definition minutes after startup applied it. (Observed: the overlay landed
/// at boot and was overwritten by the embedded schema seven seconds later.)
///
/// Installing the resolver here makes the loaders every path already shares
/// return the resolved definitions, so there is exactly one answer to "what is
/// `raisin:Page`" in the process. Defaults to embedded-only until a server
/// installs its configured stack.
static GLOBAL_RESOLVER: RwLock<Option<Arc<DefinitionResolver>>> = RwLock::new(None);

/// The process-wide definition stack (embedded-only until one is installed).
pub fn global_resolver() -> Arc<DefinitionResolver> {
    if let Some(resolver) = GLOBAL_RESOLVER.read().ok().and_then(|g| g.clone()) {
        return resolver;
    }
    Arc::new(DefinitionResolver::embedded_only())
}

/// Install `resolver` as the process-wide stack. Called by [`build_resolver`].
pub fn install_global_resolver(resolver: Arc<DefinitionResolver>) {
    if let Ok(mut guard) = GLOBAL_RESOLVER.write() {
        *guard = Some(resolver);
    }
}

/// Build the definition stack and install it as the process-wide stack.
///
/// This is what a server calls: installing is what stops the legacy init paths
/// from reverting an overlay definition to the embedded one. Tests and tools
/// that just want to inspect a stack should use [`build_resolver`], which has
/// no process-wide effect.
pub fn build_and_install_resolver(
    cfg: &SystemDefinitionsConfig,
    data_dir: &str,
) -> DefinitionResolver {
    let resolver = build_resolver(cfg, data_dir);
    install_global_resolver(Arc::new(resolver.clone()));
    resolver
}

/// Build the definition stack for a server from its configuration.
///
/// The overlay layer is added whenever its directory exists; a missing
/// directory silently yields an embedded-only stack, which is exactly the
/// behaviour of every deployment that predates this module.
pub fn build_resolver(cfg: &SystemDefinitionsConfig, data_dir: &str) -> DefinitionResolver {
    let mut layers: Vec<Arc<dyn DefinitionSource>> = vec![Arc::new(EmbeddedSource)];

    let overlay_dir = cfg.resolved_overlay_dir(data_dir);
    let overlay = OverlaySource::new(&overlay_dir);
    if overlay.exists() {
        tracing::info!(dir = %overlay_dir.display(), "System definition overlay enabled");
        layers.push(Arc::new(overlay));
    } else {
        tracing::debug!(
            dir = %overlay_dir.display(),
            "No system definition overlay directory; using embedded definitions only"
        );
    }

    let resolver = DefinitionResolver::new(layers);
    resolver.log_overrides();
    resolver
}
