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

//! The `DefinitionSource` trait and the package payload abstraction.

use crate::package_init::BuiltinPackageInfo;
use include_dir::Dir;
use raisin_error::{Error, Result};
use raisin_models::nodes::types::NodeType;
use raisin_models::workspace::Workspace;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

/// Where a package's files physically live.
///
/// Built-in packages are compiled into the binary with `include_dir!`; overlay
/// and registry-fetched packages are plain directories on disk. Both must be
/// zippable into a `.rap` and must expose their `manifest.yaml`, so the install
/// path can treat them uniformly.
#[derive(Debug, Clone)]
pub enum PackageSource {
    /// Compiled into the server binary.
    Embedded(&'static Dir<'static>),
    /// A directory on the local filesystem.
    OnDisk(PathBuf),
}

impl PackageSource {
    /// Raw bytes of the package's `manifest.yaml`.
    pub fn manifest_bytes(&self) -> Result<Vec<u8>> {
        match self {
            PackageSource::Embedded(dir) => dir
                .get_file("manifest.yaml")
                .or_else(|| {
                    // include_dir path resolution can be surprising for nested
                    // roots; fall back to a filename scan.
                    dir.files().find(|f| {
                        f.path()
                            .file_name()
                            .map(|n| n == "manifest.yaml")
                            .unwrap_or(false)
                    })
                })
                .map(|f| f.contents().to_vec())
                .ok_or_else(|| Error::storage("Missing manifest.yaml in embedded package")),
            PackageSource::OnDisk(path) => std::fs::read(path.join("manifest.yaml"))
                .map_err(|e| Error::storage(format!("Missing manifest.yaml in {:?}: {}", path, e))),
        }
    }

    /// Build the `.rap` (ZIP) payload for this package.
    pub fn to_zip(&self) -> Result<Vec<u8>> {
        match self {
            PackageSource::Embedded(dir) => crate::package_init::create_package_zip(dir),
            PackageSource::OnDisk(path) => zip_dir_from_disk(path),
        }
    }
}

/// A package definition resolved from some layer of the definition stack.
#[derive(Debug, Clone)]
pub struct PackageDefinition {
    /// Manifest, content hash and directory name.
    pub info: BuiltinPackageInfo,
    /// Where its files come from.
    pub source: PackageSource,
}

/// One layer of built-in definitions.
///
/// Layers are stacked lowest-precedence first; the resolver merges them by
/// resource name so a higher layer can replace a definition compiled into the
/// binary without a rebuild.
pub trait DefinitionSource: Send + Sync {
    /// Human-readable layer name, used in logs and in the admin UI
    /// (`"embedded"`, `"overlay"`, or a registry name).
    fn layer_name(&self) -> &str;

    /// NodeType definitions with their content hashes.
    fn nodetypes(&self) -> Vec<(NodeType, String)>;

    /// Workspace definitions with their content hashes.
    fn workspaces(&self) -> Vec<(Workspace, String)>;

    /// Package definitions with their content hashes.
    fn packages(&self) -> Vec<PackageDefinition>;
}

/// Zip a package directory that lives on the filesystem.
///
/// Mirrors `package_init::create_package_zip` (which handles the embedded case)
/// including its deterministic ordering, so the same package produces the same
/// archive regardless of which layer served it.
fn zip_dir_from_disk(root: &Path) -> Result<Vec<u8>> {
    let mut buffer = Vec::new();
    {
        let mut zip = ZipWriter::new(Cursor::new(&mut buffer));
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        add_fs_dir_to_zip(&mut zip, root, root, options)?;
        zip.finish()
            .map_err(|e| Error::storage(format!("Failed to finish ZIP: {}", e)))?;
    }
    Ok(buffer)
}

fn add_fs_dir_to_zip<W: Write + std::io::Seek>(
    zip: &mut ZipWriter<W>,
    root: &Path,
    dir: &Path,
    options: SimpleFileOptions,
) -> Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| Error::storage(format!("Failed to read {:?}: {}", dir, e)))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect();
    entries.sort();

    for path in entries {
        if path.is_dir() {
            add_fs_dir_to_zip(zip, root, &path, options)?;
            continue;
        }

        let rel = path
            .strip_prefix(root)
            .map_err(|e| Error::storage(format!("Path escape in package dir: {}", e)))?
            .to_string_lossy()
            .replace('\\', "/");

        let contents = std::fs::read(&path)
            .map_err(|e| Error::storage(format!("Failed to read {:?}: {}", path, e)))?;

        zip.start_file(rel, options)
            .map_err(|e| Error::storage(format!("Failed to start ZIP entry: {}", e)))?;
        zip.write_all(&contents)
            .map_err(|e| Error::storage(format!("Failed to write ZIP entry: {}", e)))?;
    }

    Ok(())
}
