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

//! Builtin package initialization
//!
//! This module provides functionality to load and track builtin packages
//! that are automatically installed when repositories are created.
//! Packages are embedded from the plugins directory at compile time.

use include_dir::{include_dir, Dir, DirEntry};
use raisin_error::Result;
use raisin_packages::Manifest;
use sha2::{Digest, Sha256};
use std::io::{Cursor, Write};
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

/// Embedded directory containing builtin packages
static BUILTIN_PACKAGES_DIR: Dir = include_dir!("$CARGO_MANIFEST_DIR/../../builtin-packages");

/// Information about a builtin package
#[derive(Debug, Clone)]
pub struct BuiltinPackageInfo {
    /// Package manifest
    pub manifest: Manifest,
    /// SHA256 hash of the manifest.yaml content
    pub content_hash: String,
    /// Name of the package directory (e.g., "raisin-auth")
    pub dir_name: String,
}

/// Calculate SHA256 hash of entire package directory contents
///
/// This ensures that any file change in the package triggers an update,
/// not just manifest.yaml changes.
fn calculate_package_hash(dir: &Dir<'static>) -> String {
    let mut hasher = Sha256::new();
    hash_dir_contents(&mut hasher, dir);
    format!("{:x}", hasher.finalize())
}

/// Recursively hash directory contents in a deterministic order
fn hash_dir_contents(hasher: &mut Sha256, dir: &Dir<'static>) {
    // Sort entries for consistent ordering across builds
    let mut entries: Vec<_> = dir.entries().iter().collect();
    entries.sort_by_key(|e| match e {
        DirEntry::Dir(d) => d.path().to_string_lossy().to_string(),
        DirEntry::File(f) => f.path().to_string_lossy().to_string(),
    });

    for entry in entries {
        match entry {
            DirEntry::Dir(subdir) => {
                // Include directory path in hash for structure changes
                hasher.update(subdir.path().to_string_lossy().as_bytes());
                hash_dir_contents(hasher, subdir);
            }
            DirEntry::File(file) => {
                // Include file path and contents
                hasher.update(file.path().to_string_lossy().as_bytes());
                hasher.update(file.contents());
            }
        }
    }
}

/// Load all builtin packages with their content hashes
///
/// This function scans the embedded plugins directory for packages
/// that have `builtin: true` in their manifest and returns them
/// along with their content hashes for version tracking.
///
/// # Returns
/// A vector of `BuiltinPackageInfo` containing manifest and hash
pub fn load_builtin_packages_with_hashes() -> Vec<BuiltinPackageInfo> {
    let mut packages = Vec::new();

    // Debug: Log all entries in the embedded plugins directory
    let entries = BUILTIN_PACKAGES_DIR.entries();
    tracing::debug!("BUILTIN_PACKAGES_DIR contains {} entries", entries.len());
    for entry in entries {
        match entry {
            include_dir::DirEntry::Dir(d) => {
                tracing::debug!("  Dir: {:?}", d.path());
            }
            include_dir::DirEntry::File(f) => {
                tracing::debug!("  File: {:?}", f.path());
            }
        }
    }

    // Iterate over subdirectories in the plugins directory
    for entry in BUILTIN_PACKAGES_DIR.entries() {
        if let include_dir::DirEntry::Dir(subdir) = entry {
            // Diagnostic: List ALL files in this subdirectory
            let file_paths: Vec<_> = subdir
                .files()
                .map(|f| f.path().display().to_string())
                .collect();
            tracing::debug!("Files in '{}': {:?}", subdir.path().display(), file_paths);

            // Diagnostic: Count entries
            tracing::debug!(
                "Entries in '{}': {}",
                subdir.path().display(),
                subdir.entries().len()
            );

            tracing::debug!(
                "Checking subdir '{}' for manifest.yaml",
                subdir.path().display()
            );

            // Try finding manifest.yaml using files() iterator as fallback
            let manifest_file = subdir.get_file("manifest.yaml").or_else(|| {
                subdir.files().find(|f| {
                    let path = f.path();
                    path.file_name()
                        .map(|n| n == "manifest.yaml")
                        .unwrap_or(false)
                })
            });

            // Look for manifest.yaml in the subdirectory
            if let Some(manifest_file) = manifest_file {
                tracing::debug!("Found manifest.yaml in '{}'", subdir.path().display());

                if let Some(content) = manifest_file.contents_utf8() {
                    // Hash entire package directory, not just manifest
                    let content_hash = calculate_package_hash(subdir);

                    match Manifest::from_bytes(content.as_bytes()) {
                        Ok(manifest) => {
                            tracing::debug!(
                                "Parsed manifest for '{}', builtin={}",
                                manifest.name,
                                manifest.builtin.unwrap_or(false)
                            );

                            // Only include packages marked as builtin
                            if manifest.builtin.unwrap_or(false) {
                                let dir_name = subdir
                                    .path()
                                    .file_name()
                                    .and_then(|n| n.to_str())
                                    .unwrap_or("unknown")
                                    .to_string();

                                tracing::debug!(
                                    "Loaded builtin package '{}' version {} (hash: {})",
                                    manifest.name,
                                    manifest.version,
                                    &content_hash[..8]
                                );

                                packages.push(BuiltinPackageInfo {
                                    manifest,
                                    content_hash,
                                    dir_name,
                                });
                            }
                        }
                        Err(e) => {
                            tracing::error!(
                                "Failed to parse package manifest from {}: {}",
                                subdir.path().display(),
                                e
                            );
                        }
                    }
                } else {
                    tracing::warn!(
                        "manifest.yaml in '{}' is not valid UTF-8",
                        subdir.path().display()
                    );
                }
            } else {
                tracing::debug!("No manifest.yaml in '{}'", subdir.path().display());
            }
        }
    }

    tracing::debug!("Loaded {} builtin packages total", packages.len());
    packages
}

/// Load all builtin package manifests (without hashes)
pub fn load_builtin_packages() -> Vec<Manifest> {
    load_builtin_packages_with_hashes()
        .into_iter()
        .map(|info| info.manifest)
        .collect()
}

/// Get a specific builtin package by name
pub fn get_builtin_package(name: &str) -> Option<BuiltinPackageInfo> {
    load_builtin_packages_with_hashes()
        .into_iter()
        .find(|info| info.manifest.name == name)
}

/// Get the embedded directory for a builtin package by name
pub fn get_builtin_package_dir(name: &str) -> Option<&'static Dir<'static>> {
    for entry in BUILTIN_PACKAGES_DIR.entries() {
        if let include_dir::DirEntry::Dir(subdir) = entry {
            // Try get_file first, then fallback to files() iterator
            let manifest_file = subdir.get_file("manifest.yaml").or_else(|| {
                subdir.files().find(|f| {
                    f.path()
                        .file_name()
                        .map(|n| n == "manifest.yaml")
                        .unwrap_or(false)
                })
            });

            if let Some(manifest_file) = manifest_file {
                if let Some(content) = manifest_file.contents_utf8() {
                    if let Ok(manifest) = Manifest::from_bytes(content.as_bytes()) {
                        if manifest.name == name && manifest.builtin.unwrap_or(false) {
                            return Some(subdir);
                        }
                    }
                }
            }
        }
    }
    None
}

/// Create a ZIP archive from an embedded package directory
///
/// This function creates a valid ZIP file from the embedded package directory
/// that can be stored in binary storage and processed by the package installation job.
///
/// # Arguments
/// * `dir` - The embedded directory containing package files
///
/// # Returns
/// A `Vec<u8>` containing the ZIP file contents
pub fn create_package_zip(dir: &Dir<'static>) -> Result<Vec<u8>> {
    let mut buffer = Vec::new();
    {
        let mut zip = ZipWriter::new(Cursor::new(&mut buffer));
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

        add_dir_to_zip(&mut zip, dir, "", options)?;

        zip.finish()
            .map_err(|e| raisin_error::Error::storage(format!("Failed to finish ZIP: {}", e)))?;
    }
    Ok(buffer)
}

/// Recursively add directory contents to ZIP archive
fn add_dir_to_zip<W: Write + std::io::Seek>(
    zip: &mut ZipWriter<W>,
    dir: &Dir<'static>,
    prefix: &str,
    options: SimpleFileOptions,
) -> Result<()> {
    for entry in dir.entries() {
        match entry {
            DirEntry::Dir(subdir) => {
                let path = if prefix.is_empty() {
                    // At root level, use just the directory name, not the full path
                    subdir
                        .path()
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                        .to_string()
                } else {
                    format!(
                        "{}/{}",
                        prefix,
                        subdir.path().file_name().unwrap().to_string_lossy()
                    )
                };

                // Add directory entry
                zip.add_directory(format!("{}/", path), options)
                    .map_err(|e| {
                        raisin_error::Error::storage(format!("Failed to add directory: {}", e))
                    })?;

                // Recursively add contents
                add_dir_to_zip(zip, subdir, &path, options)?;
            }
            DirEntry::File(file) => {
                let path = if prefix.is_empty() {
                    // At root level, use just the filename, not the full path
                    file.path()
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                        .to_string()
                } else {
                    format!(
                        "{}/{}",
                        prefix,
                        file.path().file_name().unwrap().to_string_lossy()
                    )
                };

                zip.start_file(&path, options).map_err(|e| {
                    raisin_error::Error::storage(format!("Failed to start file: {}", e))
                })?;
                zip.write_all(file.contents()).map_err(|e| {
                    raisin_error::Error::storage(format!("Failed to write file: {}", e))
                })?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_builtin_packages_with_hashes() {
        let packages = load_builtin_packages_with_hashes();
        // Should have at least raisin-auth
        assert!(
            !packages.is_empty(),
            "Should load at least one builtin package"
        );

        for info in &packages {
            assert!(!info.manifest.name.is_empty(), "Package should have a name");
            assert!(
                !info.manifest.version.is_empty(),
                "Package should have a version"
            );
            assert_eq!(info.content_hash.len(), 64, "Hash should be 64 hex chars");
        }
    }

    #[test]
    fn test_get_builtin_package() {
        let package = get_builtin_package("raisin-auth");
        assert!(package.is_some(), "Should find raisin-auth package");

        let info = package.unwrap();
        assert_eq!(info.manifest.name, "raisin-auth");
    }
}
