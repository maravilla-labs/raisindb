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

//! Merges the definition layers into one resolved view.

use super::source::{DefinitionSource, PackageDefinition};
use raisin_models::nodes::types::NodeType;
use raisin_models::workspace::Workspace;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Which layer a resolved definition came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefinitionOrigin {
    /// Resource name (e.g. `raisin:Package`).
    pub name: String,
    /// Winning layer (`"embedded"`, `"overlay"`, a registry name).
    pub layer: String,
    /// Layers this definition shadowed, lowest first.
    pub shadowed: Vec<String>,
}

/// The stacked definition sources for this server.
///
/// Layers are held lowest-precedence first; `EmbeddedSource` is always index 0
/// so the binary's built-ins remain the guaranteed baseline. Resolution is
/// by resource **name**: the highest layer that defines a name wins outright
/// (no field-level merging — a partial override would produce a schema nobody
/// wrote and could not be reasoned about).
#[derive(Clone)]
pub struct DefinitionResolver {
    layers: Vec<Arc<dyn DefinitionSource>>,
}

impl DefinitionResolver {
    /// Build a resolver from layers ordered lowest-precedence first.
    pub fn new(layers: Vec<Arc<dyn DefinitionSource>>) -> Self {
        Self { layers }
    }

    /// A resolver with only the binary's embedded definitions.
    pub fn embedded_only() -> Self {
        Self::new(vec![Arc::new(super::EmbeddedSource)])
    }

    /// Layer names, lowest precedence first.
    pub fn layer_names(&self) -> Vec<&str> {
        self.layers.iter().map(|l| l.layer_name()).collect()
    }

    /// Resolved NodeTypes with their content hashes.
    pub fn nodetypes(&self) -> Vec<(NodeType, String)> {
        self.resolve(|layer| {
            layer
                .nodetypes()
                .into_iter()
                .map(|(nt, hash)| (nt.name.clone(), (nt, hash)))
                .collect()
        })
    }

    /// Resolved Workspaces with their content hashes.
    pub fn workspaces(&self) -> Vec<(Workspace, String)> {
        self.resolve(|layer| {
            layer
                .workspaces()
                .into_iter()
                .map(|(ws, hash)| (ws.name.clone(), (ws, hash)))
                .collect()
        })
    }

    /// Resolved packages with their sources.
    pub fn packages(&self) -> Vec<PackageDefinition> {
        self.resolve_values(|layer| {
            layer
                .packages()
                .into_iter()
                .map(|pkg| (pkg.info.manifest.name.clone(), pkg))
                .collect()
        })
    }

    /// Where each resolved definition came from — for logging and for the
    /// admin console's system-updates view.
    pub fn origins(&self) -> Vec<DefinitionOrigin> {
        let mut by_name: BTreeMap<String, Vec<String>> = BTreeMap::new();

        for layer in &self.layers {
            let name = layer.layer_name().to_string();
            for (nt, _) in layer.nodetypes() {
                by_name.entry(nt.name).or_default().push(name.clone());
            }
            for (ws, _) in layer.workspaces() {
                by_name.entry(ws.name).or_default().push(name.clone());
            }
            for pkg in layer.packages() {
                by_name
                    .entry(pkg.info.manifest.name)
                    .or_default()
                    .push(name.clone());
            }
        }

        by_name
            .into_iter()
            .map(|(name, mut layers)| {
                let layer = layers.pop().unwrap_or_default();
                DefinitionOrigin {
                    name,
                    layer,
                    shadowed: layers,
                }
            })
            .collect()
    }

    /// Log every definition an upper layer overrides. Called once after the
    /// resolver is built (and again after a reload) so the effective schema of
    /// a running server is always visible in the logs.
    pub fn log_overrides(&self) {
        for origin in self.origins() {
            if origin.shadowed.is_empty() {
                continue;
            }
            tracing::info!(
                definition = %origin.name,
                layer = %origin.layer,
                shadows = ?origin.shadowed,
                "Definition overridden by a higher layer"
            );
        }
    }

    /// Merge layers by name, dropping the key. Later layers win.
    fn resolve<T, F>(&self, extract: F) -> Vec<(T, String)>
    where
        F: Fn(&Arc<dyn DefinitionSource>) -> BTreeMap<String, (T, String)>,
    {
        let mut merged: BTreeMap<String, (T, String)> = BTreeMap::new();
        for layer in &self.layers {
            merged.extend(extract(layer));
        }
        merged.into_values().collect()
    }

    /// Merge layers by name for a single-value payload. Later layers win.
    fn resolve_values<T, F>(&self, extract: F) -> Vec<T>
    where
        F: Fn(&Arc<dyn DefinitionSource>) -> BTreeMap<String, T>,
    {
        let mut merged: BTreeMap<String, T> = BTreeMap::new();
        for layer in &self.layers {
            merged.extend(extract(layer));
        }
        merged.into_values().collect()
    }
}

impl std::fmt::Debug for DefinitionResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DefinitionResolver")
            .field("layers", &self.layer_names())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::definitions::EmbeddedSource;

    /// A test layer that serves exactly one NodeType.
    struct OneNodeType(NodeType, String, &'static str);

    impl DefinitionSource for OneNodeType {
        fn layer_name(&self) -> &str {
            self.2
        }
        fn nodetypes(&self) -> Vec<(NodeType, String)> {
            vec![(self.0.clone(), self.1.clone())]
        }
        fn workspaces(&self) -> Vec<(Workspace, String)> {
            vec![]
        }
        fn packages(&self) -> Vec<PackageDefinition> {
            vec![]
        }
    }

    #[test]
    fn test_embedded_only_resolves_builtins() {
        let resolver = DefinitionResolver::embedded_only();
        let nodetypes = resolver.nodetypes();
        assert!(nodetypes.iter().any(|(nt, _)| nt.name == "raisin:Folder"));
        assert_eq!(resolver.layer_names(), vec!["embedded"]);
    }

    #[test]
    fn test_higher_layer_overrides_embedded_by_name() {
        let embedded = DefinitionResolver::embedded_only();
        let (mut folder, _) = embedded
            .nodetypes()
            .into_iter()
            .find(|(nt, _)| nt.name == "raisin:Folder")
            .expect("raisin:Folder is embedded");
        folder.description = Some("overridden".to_string());

        let resolver = DefinitionResolver::new(vec![
            Arc::new(EmbeddedSource),
            Arc::new(OneNodeType(folder, "deadbeef".to_string(), "overlay")),
        ]);

        let resolved = resolver
            .nodetypes()
            .into_iter()
            .find(|(nt, _)| nt.name == "raisin:Folder")
            .expect("resolved raisin:Folder");
        assert_eq!(resolved.0.description.as_deref(), Some("overridden"));
        // The resolved hash is the WINNING layer's hash — that is what makes an
        // overlay edit show up as a normal pending system update.
        assert_eq!(resolved.1, "deadbeef");

        // Non-overridden built-ins survive.
        assert!(resolver
            .nodetypes()
            .iter()
            .any(|(nt, _)| nt.name == "raisin:Page"));
    }

    #[test]
    fn test_origins_report_shadowing() {
        let embedded = DefinitionResolver::embedded_only();
        let (folder, _) = embedded
            .nodetypes()
            .into_iter()
            .find(|(nt, _)| nt.name == "raisin:Folder")
            .unwrap();

        let resolver = DefinitionResolver::new(vec![
            Arc::new(EmbeddedSource),
            Arc::new(OneNodeType(folder, "hash".to_string(), "overlay")),
        ]);

        let origin = resolver
            .origins()
            .into_iter()
            .find(|o| o.name == "raisin:Folder")
            .unwrap();
        assert_eq!(origin.layer, "overlay");
        assert_eq!(origin.shadowed, vec!["embedded".to_string()]);
    }
}
