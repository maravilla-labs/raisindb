//! Repository context types for repository-first architecture

mod branch;
mod config;
mod context;
mod workspace;

#[cfg(test)]
mod tests;

pub use branch::{
    Branch, BranchDiff, BranchDivergence, ConflictResolution, ConflictType, MergeConflict,
    MergeResult, MergeStrategy, NodeDiffInfo, ResolutionType, Tag,
};
pub use config::{RepositoryConfig, RepositoryInfo};
pub use context::RepositoryContext;
pub use workspace::{WorkspaceConfig, WorkspaceScope};
