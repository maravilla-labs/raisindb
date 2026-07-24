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

//! The baseline definition layer: everything compiled into the binary.

use super::source::{DefinitionSource, PackageDefinition, PackageSource};
use crate::nodetype_init::load_embedded_nodetypes_with_hashes;
use crate::package_init::{get_builtin_package_dir, load_builtin_packages_with_hashes};
use crate::workspace_init::load_embedded_workspaces_with_hashes;
use raisin_models::nodes::types::NodeType;
use raisin_models::workspace::Workspace;

/// Definitions embedded in the server binary via `include_dir!`.
///
/// This layer is always present and always lowest precedence: it guarantees a
/// server has a complete, self-consistent set of built-ins even with no overlay
/// directory and no network.
#[derive(Debug, Default, Clone, Copy)]
pub struct EmbeddedSource;

impl DefinitionSource for EmbeddedSource {
    fn layer_name(&self) -> &str {
        "embedded"
    }

    fn nodetypes(&self) -> Vec<(NodeType, String)> {
        load_embedded_nodetypes_with_hashes()
    }

    fn workspaces(&self) -> Vec<(Workspace, String)> {
        load_embedded_workspaces_with_hashes()
    }

    fn packages(&self) -> Vec<PackageDefinition> {
        load_builtin_packages_with_hashes()
            .into_iter()
            .filter_map(|info| {
                get_builtin_package_dir(&info.manifest.name).map(|dir| PackageDefinition {
                    info,
                    source: PackageSource::Embedded(dir),
                })
            })
            .collect()
    }
}
