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

//! Optional remote definition registry.
//!
//! A registry is an HTTP-reachable index of definitions and packages that an
//! operator can pull from without rebuilding the server. It is **opt-in and
//! never automatic**: nothing is fetched on startup, on a timer, or as a side
//! effect of any read. A fetch happens only when an operator asks for one, and
//! a fetched artifact is written into the overlay directory — from there the
//! ordinary pending/apply flow governs whether it reaches a repository.
//!
//! # Trust model
//!
//! The index declares a `sha256` for every artifact and the fetch refuses any
//! download whose bytes do not match. That protects against corruption and
//! against a swapped artifact behind a stable URL, but it does **not**
//! authenticate the index itself — trust ultimately rests on the operator
//! having configured a URL they trust and on TLS. There is no artifact signing
//! today. Treat a registry URL with the same care as a package you install by
//! hand.
//!
//! # Index format
//!
//! ```json
//! {
//!   "schema": 1,
//!   "entries": [
//!     { "name": "raisin:Package", "kind": "nodetype", "version": "2",
//!       "sha256": "…", "url": "https://…/raisin_package.yaml" },
//!     { "name": "raisin-auth", "kind": "package", "version": "0.3.0",
//!       "sha256": "…", "url": "https://…/raisin-auth.rap" }
//!   ]
//! }
//! ```

use super::config::RegistryConfig;
use raisin_error::{Error, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};

/// Kind of artifact an index entry describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegistryEntryKind {
    /// A single NodeType YAML. `nodetype` is accepted as an alias, matching
    /// `ResourceType`'s own `FromStr`, so an index can use either spelling.
    #[serde(alias = "nodetype")]
    NodeType,
    /// A single Workspace YAML.
    Workspace,
    /// A `.rap` package archive.
    Package,
}

/// One artifact offered by a registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryEntry {
    /// Resource name (`raisin:Package`, `raisin-auth`, …).
    pub name: String,
    /// What kind of artifact this is.
    pub kind: RegistryEntryKind,
    /// Display version. Informational only — the hash is what is verified.
    #[serde(default)]
    pub version: Option<String>,
    /// Expected SHA256 of the downloaded bytes, hex-encoded.
    pub sha256: String,
    /// Absolute URL of the artifact.
    pub url: String,
    /// Optional human-readable summary for the admin console.
    #[serde(default)]
    pub description: Option<String>,
}

/// A registry index document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryIndex {
    /// Index schema version. Only `1` is understood.
    #[serde(default = "default_schema")]
    pub schema: u32,
    /// Available artifacts.
    #[serde(default)]
    pub entries: Vec<RegistryEntry>,
}

fn default_schema() -> u32 {
    1
}

/// Client for one configured registry.
pub struct RegistryClient {
    config: RegistryConfig,
    http: reqwest::Client,
}

impl RegistryClient {
    /// Create a client for `config`.
    pub fn new(config: RegistryConfig) -> Self {
        Self {
            config,
            http: reqwest::Client::new(),
        }
    }

    /// The registry's configured name.
    pub fn name(&self) -> &str {
        &self.config.name
    }

    /// Fetch and parse the registry index.
    ///
    /// Errors if the registry is disabled — a disabled registry must not be
    /// contacted even by an explicit request.
    pub async fn fetch_index(&self) -> Result<RegistryIndex> {
        if !self.config.enabled {
            return Err(Error::invalid_state(format!(
                "Registry '{}' is disabled; enable it in [system_definitions] first",
                self.config.name
            )));
        }

        let mut req = self.http.get(&self.config.url);
        if let Some(token) = self.config.resolved_token() {
            req = req.bearer_auth(token);
        }

        let resp = req.send().await.map_err(|e| {
            Error::storage(format!(
                "Registry '{}' unreachable: {}",
                self.config.name, e
            ))
        })?;

        if !resp.status().is_success() {
            return Err(Error::storage(format!(
                "Registry '{}' returned {}",
                self.config.name,
                resp.status()
            )));
        }

        let index: RegistryIndex = resp.json().await.map_err(|e| {
            Error::storage(format!(
                "Registry '{}' index is not valid JSON: {}",
                self.config.name, e
            ))
        })?;

        if index.schema != 1 {
            return Err(Error::invalid_state(format!(
                "Registry '{}' uses unsupported index schema {} (this server understands 1)",
                self.config.name, index.schema
            )));
        }

        Ok(index)
    }

    /// Download the named entries into `overlay_dir`, verifying each hash.
    ///
    /// Returns the names actually written. Nothing is applied to any repository
    /// here — the fetched files become part of the overlay layer and surface as
    /// ordinary pending system updates.
    pub async fn fetch_entries(
        &self,
        index: &RegistryIndex,
        names: &[String],
        overlay_dir: &Path,
    ) -> Result<Vec<String>> {
        let mut written = Vec::new();

        for entry in &index.entries {
            if !names.is_empty() && !names.contains(&entry.name) {
                continue;
            }

            let bytes = self.download(entry).await?;
            let actual = format!("{:x}", Sha256::digest(&bytes));
            if actual != entry.sha256.to_lowercase() {
                return Err(Error::invalid_state(format!(
                    "Registry '{}': artifact '{}' hash mismatch (index says {}, downloaded {})",
                    self.config.name, entry.name, entry.sha256, actual
                )));
            }

            write_entry(entry, &bytes, overlay_dir)?;
            written.push(entry.name.clone());

            tracing::info!(
                registry = %self.config.name,
                artifact = %entry.name,
                kind = ?entry.kind,
                hash = %&actual[..8],
                "Fetched definition artifact into the overlay"
            );
        }

        Ok(written)
    }

    async fn download(&self, entry: &RegistryEntry) -> Result<Vec<u8>> {
        let mut req = self.http.get(&entry.url);
        if let Some(token) = self.config.resolved_token() {
            req = req.bearer_auth(token);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| Error::storage(format!("Failed to download '{}': {}", entry.name, e)))?;

        if !resp.status().is_success() {
            return Err(Error::storage(format!(
                "Failed to download '{}': HTTP {}",
                entry.name,
                resp.status()
            )));
        }

        resp.bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| Error::storage(format!("Failed to read '{}': {}", entry.name, e)))
    }
}

/// Place a verified artifact in the overlay directory in the layout
/// `OverlaySource` expects.
fn write_entry(entry: &RegistryEntry, bytes: &[u8], overlay_dir: &Path) -> Result<()> {
    match entry.kind {
        RegistryEntryKind::NodeType => {
            write_yaml(overlay_dir.join("nodetypes"), &entry.name, bytes)
        }
        RegistryEntryKind::Workspace => {
            write_yaml(overlay_dir.join("workspaces"), &entry.name, bytes)
        }
        RegistryEntryKind::Package => unpack_rap(overlay_dir.join("packages"), &entry.name, bytes),
    }
}

fn write_yaml(dir: PathBuf, name: &str, bytes: &[u8]) -> Result<()> {
    std::fs::create_dir_all(&dir)
        .map_err(|e| Error::storage(format!("Failed to create {:?}: {}", dir, e)))?;
    let path = dir.join(format!("{}.yaml", file_stem(name)));
    std::fs::write(&path, bytes)
        .map_err(|e| Error::storage(format!("Failed to write {:?}: {}", path, e)))
}

/// Extract a `.rap` into `<overlay>/packages/<name>/`, replacing any previous
/// copy so a re-fetch cannot leave stale files behind.
fn unpack_rap(dir: PathBuf, name: &str, bytes: &[u8]) -> Result<()> {
    let target = dir.join(file_stem(name));
    if target.exists() {
        std::fs::remove_dir_all(&target)
            .map_err(|e| Error::storage(format!("Failed to clear {:?}: {}", target, e)))?;
    }
    std::fs::create_dir_all(&target)
        .map_err(|e| Error::storage(format!("Failed to create {:?}: {}", target, e)))?;

    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|e| Error::storage(format!("'{}' is not a valid package archive: {}", name, e)))?;

    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| Error::storage(format!("Corrupt archive entry in '{}': {}", name, e)))?;

        // Reject absolute paths and `..` traversal before touching the disk.
        let Some(rel) = file.enclosed_name() else {
            return Err(Error::invalid_state(format!(
                "Package '{}' contains an unsafe path entry",
                name
            )));
        };
        let out = target.join(rel);

        if file.is_dir() {
            std::fs::create_dir_all(&out)
                .map_err(|e| Error::storage(format!("Failed to create {:?}: {}", out, e)))?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| Error::storage(format!("Failed to create {:?}: {}", parent, e)))?;
        }

        let mut contents = Vec::new();
        file.read_to_end(&mut contents)
            .map_err(|e| Error::storage(format!("Failed to read archive entry: {}", e)))?;
        std::fs::write(&out, contents)
            .map_err(|e| Error::storage(format!("Failed to write {:?}: {}", out, e)))?;
    }

    Ok(())
}

/// Turn a resource name into a safe filename stem (`raisin:Package` → `raisin_package`).
fn file_stem(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>()
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_stem_is_filesystem_safe() {
        assert_eq!(file_stem("raisin:Package"), "raisin_package");
        assert_eq!(file_stem("raisin-auth"), "raisin-auth");
        assert_eq!(file_stem("../escape"), "___escape");
    }

    #[tokio::test]
    async fn test_disabled_registry_is_never_contacted() {
        let client = RegistryClient::new(RegistryConfig {
            name: "test".into(),
            // Deliberately unroutable: if the guard regressed, this would hang
            // or error with a network message instead of a validation message.
            url: "http://127.0.0.1:1/index.json".into(),
            enabled: false,
            token: None,
        });
        let err = client.fetch_index().await.unwrap_err();
        assert!(err.to_string().contains("disabled"), "got: {}", err);
    }

    #[test]
    fn test_index_parses() {
        let index: RegistryIndex = serde_json::from_str(
            r#"{"schema":1,"entries":[
                {"name":"raisin:Package","kind":"nodetype","sha256":"ab","url":"https://x/y.yaml"}
            ]}"#,
        )
        .unwrap();
        assert_eq!(index.entries.len(), 1);
        assert_eq!(index.entries[0].kind, RegistryEntryKind::NodeType);
    }
}
