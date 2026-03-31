// SPDX-License-Identifier: BSL-1.1

//! Package error types

use thiserror::Error;

/// Package operation errors
#[derive(Error, Debug)]
pub enum PackageError {
    #[error("Invalid package: {0}")]
    InvalidPackage(String),

    #[error("Manifest not found in package")]
    ManifestNotFound,

    #[error("Invalid manifest: {0}")]
    InvalidManifest(String),

    #[error("File not found in package: {0}")]
    FileNotFound(String),

    #[error("ZIP error: {0}")]
    ZipError(#[from] zip::result::ZipError),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("YAML parse error: {0}")]
    YamlError(#[from] serde_yaml::Error),

    #[error("JSON parse error: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("Package already installed: {0}")]
    AlreadyInstalled(String),

    #[error("Package not installed: {0}")]
    NotInstalled(String),

    #[error("Dependency not satisfied: {0} requires {1}")]
    DependencyNotSatisfied(String, String),

    #[error("Node type conflict: {0} already exists")]
    NodeTypeConflict(String),

    #[error("Workspace not found: {0}")]
    WorkspaceNotFound(String),

    #[error("Storage error: {0}")]
    StorageError(String),
}

/// Result type for package operations
pub type PackageResult<T> = Result<T, PackageError>;
