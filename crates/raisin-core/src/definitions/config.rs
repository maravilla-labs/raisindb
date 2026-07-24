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

//! Configuration for the system-definition stack (`[system_definitions]`).

use crate::system_updates::AutoApplyPolicy;
use serde::{Deserialize, Serialize};

/// `[system_definitions]` — how built-in NodeTypes, Workspaces and packages are
/// sourced and rolled out.
///
/// Every field has a default that reproduces the pre-existing behaviour of a
/// server with no such section: embedded definitions only, no registries, and
/// non-breaking changes applied on startup.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SystemDefinitionsConfig {
    /// Whether changed built-in definitions are written into existing
    /// repositories on startup, and how aggressively.
    #[serde(default)]
    pub auto_apply: AutoApplyPolicy,

    /// Directory holding definitions that override the binary's built-ins.
    /// Defaults to `<data_dir>/system-definitions` when unset. A missing
    /// directory is not an error.
    #[serde(default)]
    pub overlay_dir: Option<String>,

    /// Optional remote definition registries. Empty by default; a registry is
    /// only ever contacted on an explicit operator action.
    #[serde(default)]
    pub registries: Vec<RegistryConfig>,
}

impl SystemDefinitionsConfig {
    /// Resolve the overlay directory, falling back to `<data_dir>/system-definitions`.
    pub fn resolved_overlay_dir(&self, data_dir: &str) -> std::path::PathBuf {
        match &self.overlay_dir {
            Some(dir) => std::path::PathBuf::from(dir),
            None => std::path::Path::new(data_dir).join("system-definitions"),
        }
    }

    /// Registries that are enabled for use.
    pub fn enabled_registries(&self) -> impl Iterator<Item = &RegistryConfig> {
        self.registries.iter().filter(|r| r.enabled)
    }
}

/// A remote source of definitions and packages.
///
/// The URL is entirely operator-supplied — no host is hardcoded anywhere in the
/// server — so a deployment can point at the public index, at a private mirror,
/// or at nothing at all.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryConfig {
    /// Short identifier, used as the definition layer name and in API paths.
    pub name: String,

    /// URL of the registry index JSON.
    pub url: String,

    /// Whether this registry may be contacted. Defaults to `false`: a registry
    /// must be switched on deliberately.
    #[serde(default)]
    pub enabled: bool,

    /// Optional bearer token for a private registry, or the name of an
    /// environment variable holding it when prefixed with `env:`.
    #[serde(default)]
    pub token: Option<String>,
}

impl RegistryConfig {
    /// Resolve the bearer token, following an `env:VAR` indirection.
    pub fn resolved_token(&self) -> Option<String> {
        match self.token.as_deref() {
            Some(v) => match v.strip_prefix("env:") {
                Some(var) => std::env::var(var).ok(),
                None => Some(v.to_string()),
            },
            None => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_defaults_preserve_existing_behaviour() {
        let cfg = SystemDefinitionsConfig::default();
        assert_eq!(cfg.auto_apply, AutoApplyPolicy::NonBreaking);
        assert!(cfg.registries.is_empty());
        assert_eq!(
            cfg.resolved_overlay_dir("/data/raisindb"),
            std::path::PathBuf::from("/data/raisindb/system-definitions")
        );
    }

    #[test]
    fn test_explicit_overlay_dir_wins() {
        let cfg = SystemDefinitionsConfig {
            overlay_dir: Some("/etc/raisindb/defs".into()),
            ..Default::default()
        };
        assert_eq!(
            cfg.resolved_overlay_dir("/data/raisindb"),
            std::path::PathBuf::from("/etc/raisindb/defs")
        );
    }

    #[test]
    fn test_registries_are_opt_in() {
        let cfg: SystemDefinitionsConfig = toml::from_str(
            r#"
            [[registries]]
            name = "public"
            url = "https://example.invalid/index.json"
            "#,
        )
        .unwrap();
        assert_eq!(cfg.registries.len(), 1);
        assert!(!cfg.registries[0].enabled);
        assert_eq!(cfg.enabled_registries().count(), 0);
    }
}
