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

//! Filesystem overlay layer — the offline out-of-band update channel.
//!
//! An operator drops corrected definitions into a directory on the server and
//! they override the binary's built-ins, with no rebuild and no redeploy:
//!
//! ```text
//! <overlay_dir>/
//!   nodetypes/raisin_package.yaml     # same shape as crates/raisin-core/global_nodetypes/
//!   workspaces/content.yaml           # same shape as crates/raisin-core/global_workspaces/
//!   packages/raisin-auth/manifest.yaml  # same shape as builtin-packages/<name>/
//! ```
//!
//! The directory layout deliberately mirrors the source tree so a fix can be
//! copied straight out of a checkout. A missing or empty directory is not an
//! error — it simply means the deployment runs on embedded definitions only.

use super::source::{DefinitionSource, PackageDefinition, PackageSource};
use crate::nodetype_init::calculate_content_hash;
use crate::package_init::BuiltinPackageInfo;
use raisin_models::nodes::types::NodeType;
use raisin_models::workspace::Workspace;
use raisin_packages::Manifest;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// Definitions read from a directory on the local filesystem.
#[derive(Debug, Clone)]
pub struct OverlaySource {
    root: PathBuf,
    layer_name: String,
}

impl OverlaySource {
    /// Create an overlay rooted at `root`. The directory need not exist.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            layer_name: "overlay".to_string(),
        }
    }

    /// Create an overlay with a custom layer name (used by registry caches, so
    /// the admin UI can show which registry a definition came from).
    pub fn with_layer_name(root: impl Into<PathBuf>, layer_name: impl Into<String>) -> Self {
        Self {
            root: root.into(),
            layer_name: layer_name.into(),
        }
    }

    /// The directory this overlay reads from.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Whether the overlay directory exists at all.
    pub fn exists(&self) -> bool {
        self.root.is_dir()
    }
}

impl DefinitionSource for OverlaySource {
    fn layer_name(&self) -> &str {
        &self.layer_name
    }

    fn nodetypes(&self) -> Vec<(NodeType, String)> {
        load_yaml_dir(&self.root.join("nodetypes"), "NodeType")
    }

    fn workspaces(&self) -> Vec<(Workspace, String)> {
        load_yaml_dir(&self.root.join("workspaces"), "Workspace")
    }

    fn packages(&self) -> Vec<PackageDefinition> {
        let dir = self.root.join("packages");
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return Vec::new();
        };

        let mut paths: Vec<PathBuf> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        paths.sort();

        let mut packages = Vec::new();
        for path in paths {
            let manifest_path = path.join("manifest.yaml");
            let Ok(bytes) = std::fs::read(&manifest_path) else {
                tracing::warn!(path = %manifest_path.display(), "Overlay package has no manifest.yaml, skipping");
                continue;
            };
            let manifest = match Manifest::from_bytes(&bytes) {
                Ok(m) => m,
                Err(e) => {
                    tracing::error!(path = %manifest_path.display(), error = %e, "Failed to parse overlay package manifest");
                    continue;
                }
            };

            let dir_name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();
            let content_hash = hash_dir_on_disk(&path);

            tracing::info!(
                package = %manifest.name,
                layer = %self.layer_name,
                hash = %&content_hash[..8],
                "Loaded package from definition overlay"
            );

            packages.push(PackageDefinition {
                info: BuiltinPackageInfo {
                    manifest,
                    content_hash,
                    dir_name,
                },
                source: PackageSource::OnDisk(path),
            });
        }

        packages
    }
}

/// Load and hash every `*.yaml` in `dir`, parsing each into `T`.
///
/// Uses the same SHA256-of-file-content hash as the embedded loaders, so an
/// overlay definition slots into the existing `system_updates` hash tracking
/// with no special-casing.
fn load_yaml_dir<T: serde::de::DeserializeOwned>(dir: &Path, kind: &str) -> Vec<(T, String)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut paths: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("yaml"))
        .collect();
    paths.sort();

    let mut loaded = Vec::new();
    for path in paths {
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                tracing::error!(path = %path.display(), error = %e, "Failed to read overlay {} YAML", kind);
                continue;
            }
        };

        match serde_yaml::from_str::<T>(&content) {
            Ok(parsed) => loaded.push((parsed, calculate_content_hash(&content))),
            Err(e) => {
                // A malformed overlay file must never take down the server or
                // silently shadow a valid embedded definition — skip it loudly
                // and keep the built-in.
                tracing::error!(
                    path = %path.display(),
                    error = %e,
                    "Failed to parse overlay {} YAML, keeping embedded definition",
                    kind
                );
            }
        }
    }

    loaded
}

/// Deterministic content hash of a package directory on disk.
///
/// Mirrors `package_init::hash_dir_contents` (path + contents, sorted) so an
/// overlay copy of an embedded package that is byte-identical hashes the same
/// and therefore registers as "no update".
fn hash_dir_on_disk(root: &Path) -> String {
    let mut hasher = Sha256::new();
    hash_dir_rec(&mut hasher, root, root);
    format!("{:x}", hasher.finalize())
}

fn hash_dir_rec(hasher: &mut Sha256, root: &Path, dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    paths.sort();

    for path in paths {
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        hasher.update(rel.as_bytes());
        if path.is_dir() {
            hash_dir_rec(hasher, root, &path);
        } else if let Ok(bytes) = std::fs::read(&path) {
            hasher.update(&bytes);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::definitions::{build_resolver, SystemDefinitionsConfig};

    #[test]
    fn test_missing_overlay_dir_is_empty_not_an_error() {
        let overlay = OverlaySource::new("/nonexistent/raisin-overlay-test");
        assert!(!overlay.exists());
        assert!(overlay.nodetypes().is_empty());
        assert!(overlay.workspaces().is_empty());
        assert!(overlay.packages().is_empty());
    }

    /// A deployment with no overlay directory must behave exactly as it did
    /// before overlays existed.
    #[test]
    fn test_no_overlay_dir_resolves_to_embedded_only() {
        let tmp = tempfile::tempdir().unwrap();
        let resolver = build_resolver(
            &SystemDefinitionsConfig::default(),
            tmp.path().to_str().unwrap(),
        );
        assert_eq!(resolver.layer_names(), vec!["embedded"]);
    }

    /// The end-to-end promise of the overlay: drop a YAML on disk and it
    /// replaces the definition compiled into the binary, no rebuild involved.
    #[test]
    fn test_overlay_yaml_overrides_an_embedded_nodetype() {
        let tmp = tempfile::tempdir().unwrap();
        let overlay_dir = tmp.path().join("system-definitions");
        std::fs::create_dir_all(overlay_dir.join("nodetypes")).unwrap();
        std::fs::write(
            overlay_dir.join("nodetypes/raisin_folder.yaml"),
            "name: raisin:Folder\nversion: 1\ndescription: from the overlay\n",
        )
        .unwrap();

        let cfg = SystemDefinitionsConfig {
            overlay_dir: Some(overlay_dir.to_string_lossy().into_owned()),
            ..Default::default()
        };
        let resolver = build_resolver(&cfg, tmp.path().to_str().unwrap());
        assert_eq!(resolver.layer_names(), vec!["embedded", "overlay"]);

        let (folder, hash) = resolver
            .nodetypes()
            .into_iter()
            .find(|(nt, _)| nt.name == "raisin:Folder")
            .expect("raisin:Folder resolves");
        assert_eq!(folder.description.as_deref(), Some("from the overlay"));

        // The resolved hash must be the overlay file's, so the change registers
        // as a pending system update rather than looking already-applied.
        let embedded_hash = crate::nodetype_init::load_global_nodetypes_with_hashes()
            .into_iter()
            .find(|(nt, _)| nt.name == "raisin:Folder")
            .map(|(_, h)| h)
            .unwrap();
        assert_ne!(hash, embedded_hash);

        // Definitions the overlay does not mention still come from the binary.
        assert!(resolver
            .nodetypes()
            .iter()
            .any(|(nt, _)| nt.name == "raisin:Page"));
    }

    /// A malformed overlay file must not shadow the built-in it names, and must
    /// not take the server down.
    #[test]
    fn test_malformed_overlay_file_falls_back_to_embedded() {
        let tmp = tempfile::tempdir().unwrap();
        let overlay_dir = tmp.path().join("system-definitions");
        std::fs::create_dir_all(overlay_dir.join("nodetypes")).unwrap();
        std::fs::write(
            overlay_dir.join("nodetypes/broken.yaml"),
            "this: [is not: a nodetype\n",
        )
        .unwrap();

        let cfg = SystemDefinitionsConfig {
            overlay_dir: Some(overlay_dir.to_string_lossy().into_owned()),
            ..Default::default()
        };
        let resolver = build_resolver(&cfg, tmp.path().to_str().unwrap());
        assert!(resolver
            .nodetypes()
            .iter()
            .any(|(nt, _)| nt.name == "raisin:Folder"));
    }
}
