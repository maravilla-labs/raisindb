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

//! Graph relations carried by a package: `relations.yaml` at the archive root.
//!
//! Relations are stored by node id, and node ids do not survive a package
//! round trip (content YAML written by hand has none, and an install may keep
//! the id of a node already at that path). So a package names both endpoints
//! by `(workspace, path)` and the installer resolves them on the target
//! repository after the content is installed:
//!
//! ```yaml
//! version: 1
//! relations:
//!   - from: { workspace: "raisin:access_control", path: /users/internal/u_… }
//!     to: { workspace: "raisin:access_control", path: /users/internal/u_… }
//!     type: FRIENDS_WITH
//!     weight: 1.0
//! ```
//!
//! A user home's path is its stable key (`/users/internal/<user_key>`), so a
//! relation between users needs nothing beyond its paths.

use serde::{Deserialize, Serialize};

use crate::error::{PackageError, PackageResult};

/// Archive path of the relations file.
pub const RELATIONS_FILE: &str = "relations.yaml";

/// Format version this release writes and the newest it reads.
pub const RELATIONS_FORMAT_VERSION: u32 = 1;

/// One end of a relation, addressed by workspace and path.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RelationEndpoint {
    /// Workspace of the node.
    pub workspace: String,
    /// Absolute path of the node in that workspace.
    pub path: String,
}

/// One relation of a package.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackageRelation {
    /// Source node.
    pub from: RelationEndpoint,
    /// Target node.
    pub to: RelationEndpoint,
    /// Relation type (e.g. `FRIENDS_WITH`).
    #[serde(rename = "type")]
    pub relation_type: String,
    /// Optional weight.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight: Option<f32>,
}

/// The `relations.yaml` document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackageRelations {
    /// Format version (see [`RELATIONS_FORMAT_VERSION`]).
    pub version: u32,
    /// The relations, in a stable order.
    #[serde(default)]
    pub relations: Vec<PackageRelation>,
}

impl PackageRelations {
    /// A document of the current version, sorted for a reproducible archive.
    pub fn new(mut relations: Vec<PackageRelation>) -> Self {
        relations.sort_by(|a, b| {
            (
                &a.from.workspace,
                &a.from.path,
                &a.relation_type,
                &a.to.workspace,
                &a.to.path,
            )
                .cmp(&(
                    &b.from.workspace,
                    &b.from.path,
                    &b.relation_type,
                    &b.to.workspace,
                    &b.to.path,
                ))
        });
        relations.dedup_by(|a, b| {
            a.from == b.from && a.to == b.to && a.relation_type == b.relation_type
        });
        Self {
            version: RELATIONS_FORMAT_VERSION,
            relations,
        }
    }

    /// Serialize to YAML.
    pub fn to_yaml(&self) -> PackageResult<String> {
        serde_yaml::to_string(self).map_err(PackageError::from)
    }

    /// Parse `relations.yaml`, refusing a newer format and malformed entries
    /// (a relative path, an empty workspace or type).
    pub fn parse(yaml: &str) -> PackageResult<Self> {
        let doc: PackageRelations = serde_yaml::from_str(yaml).map_err(PackageError::from)?;
        if doc.version > RELATIONS_FORMAT_VERSION {
            return Err(PackageError::InvalidManifest(format!(
                "{RELATIONS_FILE} has format version {}, this release reads up to {}",
                doc.version, RELATIONS_FORMAT_VERSION
            )));
        }
        for (i, r) in doc.relations.iter().enumerate() {
            for end in [&r.from, &r.to] {
                if end.workspace.trim().is_empty() || !end.path.starts_with('/') {
                    return Err(PackageError::InvalidManifest(format!(
                        "{RELATIONS_FILE}: relation {} has an endpoint without a workspace or \
                         an absolute path",
                        i + 1
                    )));
                }
            }
            if r.relation_type.trim().is_empty() {
                return Err(PackageError::InvalidManifest(format!(
                    "{RELATIONS_FILE}: relation {} has no type",
                    i + 1
                )));
            }
        }
        Ok(doc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn end(path: &str) -> RelationEndpoint {
        RelationEndpoint {
            workspace: "raisin:access_control".into(),
            path: path.into(),
        }
    }

    fn rel(from: &str, to: &str, ty: &str) -> PackageRelation {
        PackageRelation {
            from: end(from),
            to: end(to),
            relation_type: ty.into(),
            weight: Some(1.0),
        }
    }

    #[test]
    fn round_trips_sorted_and_deduplicated() {
        let doc = PackageRelations::new(vec![
            rel("/b", "/a", "FRIENDS_WITH"),
            rel("/a", "/b", "FRIENDS_WITH"),
            rel("/a", "/b", "FRIENDS_WITH"),
        ]);
        assert_eq!(doc.relations.len(), 2);
        assert_eq!(doc.relations[0].from.path, "/a");
        let yaml = doc.to_yaml().unwrap();
        assert!(yaml.contains("type: FRIENDS_WITH"), "{yaml}");
        assert_eq!(PackageRelations::parse(&yaml).unwrap(), doc);
    }

    #[test]
    fn hand_written_yaml_without_weight_parses() {
        let doc = PackageRelations::parse(
            "version: 1\nrelations:\n  - from: { workspace: ws, path: /a }\n    to: { workspace: ws, path: /b }\n    type: KNOWS\n",
        )
        .unwrap();
        assert_eq!(doc.relations[0].weight, None);
    }

    #[test]
    fn newer_versions_and_bad_endpoints_are_refused() {
        assert!(PackageRelations::parse("version: 2\nrelations: []\n").is_err());
        assert!(PackageRelations::parse(
            "version: 1\nrelations:\n  - from: { workspace: ws, path: a }\n    to: { workspace: ws, path: /b }\n    type: K\n"
        )
        .is_err());
        assert!(PackageRelations::parse(
            "version: 1\nrelations:\n  - from: { workspace: ws, path: /a }\n    to: { workspace: '', path: /b }\n    type: K\n"
        )
        .is_err());
    }
}
