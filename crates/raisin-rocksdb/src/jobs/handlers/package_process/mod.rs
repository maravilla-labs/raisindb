//! Package processing job handler
//!
//! This module handles background processing of newly uploaded RaisinDB packages (.rap files).
//! After a package is uploaded via the unified endpoint, this job extracts the manifest.yaml
//! and updates the node properties (name, version, title, description, etc.).
//!
//! This is a simpler operation than package installation - it just extracts metadata
//! and updates the node, without installing content.

mod assets;
mod handler;
mod manifest;

pub use handler::{
    BinaryRetrievalCallback, BinaryStorageCallback, PackageProcessHandler, PackageProcessResult,
};
pub use manifest::{
    AllowedNodeTypesPatch, PackageDependency, PackageManifest, PackageProvides, WorkspacePatch,
};
