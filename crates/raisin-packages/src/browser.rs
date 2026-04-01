// SPDX-License-Identifier: BSL-1.1

//! Package content browser - browse ZIP contents without extracting

use std::io::{Cursor, Read};
use zip::ZipArchive;

use crate::error::{PackageError, PackageResult};
use crate::manifest::Manifest;

/// Entry type in the ZIP archive
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryType {
    /// A file
    File,
    /// A directory
    Directory,
}

/// An entry in the ZIP archive
#[derive(Debug, Clone)]
pub struct ZipEntry {
    /// Path within the archive
    pub path: String,

    /// Entry type (file or directory)
    pub entry_type: EntryType,

    /// File size in bytes (0 for directories)
    pub size: u64,

    /// Whether the file is compressed
    pub compressed: bool,

    /// Compressed size (if applicable)
    pub compressed_size: u64,
}

/// Browse package contents without extracting
pub struct PackageBrowser {
    data: Vec<u8>,
}

impl PackageBrowser {
    /// Create a new browser from ZIP data
    pub fn new(data: Vec<u8>) -> Self {
        Self { data }
    }

    /// Create a browser from a byte slice (copies the data)
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self::new(bytes.to_vec())
    }

    /// Get the manifest from the package
    pub fn manifest(&self) -> PackageResult<Manifest> {
        let cursor = Cursor::new(&self.data);
        let mut archive = ZipArchive::new(cursor)?;
        Manifest::from_zip(&mut archive)
    }

    /// List all entries in the package
    pub fn list_entries(&self) -> PackageResult<Vec<ZipEntry>> {
        let cursor = Cursor::new(&self.data);
        let mut archive = ZipArchive::new(cursor)?;

        let mut entries = Vec::new();
        for i in 0..archive.len() {
            let file = archive.by_index_raw(i)?;
            let path = file.name().to_string();

            let entry_type = if file.is_dir() {
                EntryType::Directory
            } else {
                EntryType::File
            };

            entries.push(ZipEntry {
                path,
                entry_type,
                size: file.size(),
                compressed: file.compression() != zip::CompressionMethod::Stored,
                compressed_size: file.compressed_size(),
            });
        }

        // Sort by path for consistent ordering
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(entries)
    }

    /// List entries in a specific directory
    pub fn list_directory(&self, dir_path: &str) -> PackageResult<Vec<ZipEntry>> {
        let all_entries = self.list_entries()?;

        // Normalize the directory path
        let normalized = if dir_path.is_empty() {
            String::new()
        } else if dir_path.ends_with('/') {
            dir_path.to_string()
        } else {
            format!("{}/", dir_path)
        };

        // Filter to direct children only
        let entries: Vec<ZipEntry> = all_entries
            .into_iter()
            .filter(|e| {
                if normalized.is_empty() {
                    // Root level: no slashes except possibly trailing
                    !e.path.contains('/')
                        || e.path.ends_with('/') && !e.path[..e.path.len() - 1].contains('/')
                } else {
                    // Starts with the directory path and is a direct child
                    if !e.path.starts_with(&normalized) {
                        return false;
                    }
                    let remainder = &e.path[normalized.len()..];
                    !remainder.is_empty()
                        && (!remainder.contains('/')
                            || (remainder.ends_with('/')
                                && !remainder[..remainder.len() - 1].contains('/')))
                }
            })
            .collect();

        Ok(entries)
    }

    /// Read a file from the package
    pub fn read_file(&self, path: &str) -> PackageResult<Vec<u8>> {
        let cursor = Cursor::new(&self.data);
        let mut archive = ZipArchive::new(cursor)?;

        let mut file = archive
            .by_name(path)
            .map_err(|_| PackageError::FileNotFound(path.to_string()))?;

        if file.is_dir() {
            return Err(PackageError::FileNotFound(format!(
                "{} is a directory",
                path
            )));
        }

        let mut contents = Vec::new();
        file.read_to_end(&mut contents)?;
        Ok(contents)
    }

    /// Read a file as UTF-8 string
    pub fn read_file_string(&self, path: &str) -> PackageResult<String> {
        let bytes = self.read_file(path)?;
        String::from_utf8(bytes)
            .map_err(|e| PackageError::InvalidPackage(format!("Invalid UTF-8 in {}: {}", path, e)))
    }

    /// Check if a path exists in the package
    pub fn exists(&self, path: &str) -> PackageResult<bool> {
        let cursor = Cursor::new(&self.data);
        let mut archive = ZipArchive::new(cursor)?;

        let exists = archive.by_name(path).is_ok();
        Ok(exists)
    }

    /// Get the list of node types in the package
    pub fn list_nodetypes(&self) -> PackageResult<Vec<String>> {
        let entries = self.list_directory("nodetypes")?;
        Ok(entries
            .into_iter()
            .filter(|e| e.entry_type == EntryType::File && e.path.ends_with(".yaml"))
            .map(|e| e.path)
            .collect())
    }

    /// Get the list of mixins in the package
    pub fn list_mixins(&self) -> PackageResult<Vec<String>> {
        let entries = self.list_directory("mixins")?;
        Ok(entries
            .into_iter()
            .filter(|e| e.entry_type == EntryType::File && e.path.ends_with(".yaml"))
            .map(|e| e.path)
            .collect())
    }

    /// Get the list of content directories
    pub fn list_content_workspaces(&self) -> PackageResult<Vec<String>> {
        let entries = self.list_directory("content")?;
        Ok(entries
            .into_iter()
            .filter(|e| e.entry_type == EntryType::Directory)
            .map(|e| {
                let dir_name = e
                    .path
                    .trim_start_matches("content/")
                    .trim_end_matches('/')
                    .to_string();
                crate::namespace_encoding::decode_namespace(&dir_name)
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    fn create_test_package() -> Vec<u8> {
        let mut buf = Vec::new();
        let cursor = Cursor::new(&mut buf);
        let mut zip = ZipWriter::new(cursor);

        let options = SimpleFileOptions::default();

        // Add manifest
        zip.start_file("manifest.yaml", options).unwrap();
        zip.write_all(
            br#"
name: test-package
version: 1.0.0
title: Test Package
"#,
        )
        .unwrap();

        // Add a nodetype
        zip.add_directory("nodetypes/", options).unwrap();
        zip.start_file("nodetypes/test_type.yaml", options).unwrap();
        zip.write_all(b"name: test:Type\n").unwrap();

        // Add content
        zip.add_directory("content/", options).unwrap();
        zip.add_directory("content/default/", options).unwrap();
        zip.start_file("content/default/node.yaml", options)
            .unwrap();
        zip.write_all(b"node_type: test:Type\n").unwrap();

        zip.finish().unwrap();
        buf
    }

    #[test]
    fn test_browse_package() {
        let data = create_test_package();
        let browser = PackageBrowser::new(data);

        let manifest = browser.manifest().unwrap();
        assert_eq!(manifest.name, "test-package");

        let entries = browser.list_entries().unwrap();
        assert!(!entries.is_empty());

        let nodetypes = browser.list_nodetypes().unwrap();
        assert_eq!(nodetypes.len(), 1);
        assert!(nodetypes[0].contains("test_type.yaml"));
    }

    #[test]
    fn test_read_file() {
        let data = create_test_package();
        let browser = PackageBrowser::new(data);

        let content = browser.read_file_string("manifest.yaml").unwrap();
        assert!(content.contains("test-package"));

        assert!(browser.read_file("nonexistent.txt").is_err());
    }
}
