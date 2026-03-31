//! Storage scope structs for reducing parameter repetition.
//!
//! These structs encapsulate the common (tenant, repo, branch, workspace) tuples
//! that appear in nearly every storage trait method. They are introduced additively
//! in Phase 0 — existing APIs are unchanged.
//!
//! # Scope Hierarchy
//!
//! ```text
//! RepoScope     { tenant_id, repo_id }
//!   └─ BranchScope  { tenant_id, repo_id, branch }
//!        └─ StorageScope { tenant_id, repo_id, branch, workspace }
//! ```
//!
//! # Example
//!
//! ```
//! use raisin_storage::scope::{StorageScope, BranchScope, RepoScope};
//!
//! let scope = StorageScope::new("tenant1", "repo1", "main", "default");
//! let branch: BranchScope = scope.branch_scope();
//! let repo: RepoScope = branch.repo_scope();
//!
//! assert_eq!(scope.tenant_id, "tenant1");
//! assert_eq!(branch.branch, "main");
//! assert_eq!(repo.repo_id, "repo1");
//! ```

use std::fmt;

// ---------------------------------------------------------------------------
// Borrowed (zero-copy) scopes
// ---------------------------------------------------------------------------

/// Full workspace-level scope: tenant + repo + branch + workspace.
///
/// This is the most common scope, used by `NodeRepository`, `PropertyIndexRepository`,
/// `TranslationRepository`, and most other storage traits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StorageScope<'a> {
    pub tenant_id: &'a str,
    pub repo_id: &'a str,
    pub branch: &'a str,
    pub workspace: &'a str,
}

/// Branch-level scope: tenant + repo + branch.
///
/// Used by `NodeTypeRepository`, `ArchetypeRepository`, `ElementTypeRepository`,
/// and other branch-scoped traits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BranchScope<'a> {
    pub tenant_id: &'a str,
    pub repo_id: &'a str,
    pub branch: &'a str,
}

/// Repository-level scope: tenant + repo.
///
/// Used by `BranchRepository`, `TagRepository`, and repo-level management operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RepoScope<'a> {
    pub tenant_id: &'a str,
    pub repo_id: &'a str,
}

// ---------------------------------------------------------------------------
// Constructors
// ---------------------------------------------------------------------------

impl<'a> StorageScope<'a> {
    pub fn new(tenant_id: &'a str, repo_id: &'a str, branch: &'a str, workspace: &'a str) -> Self {
        Self {
            tenant_id,
            repo_id,
            branch,
            workspace,
        }
    }
}

impl<'a> BranchScope<'a> {
    pub fn new(tenant_id: &'a str, repo_id: &'a str, branch: &'a str) -> Self {
        Self {
            tenant_id,
            repo_id,
            branch,
        }
    }
}

impl<'a> RepoScope<'a> {
    pub fn new(tenant_id: &'a str, repo_id: &'a str) -> Self {
        Self { tenant_id, repo_id }
    }
}

// ---------------------------------------------------------------------------
// Narrowing / widening conversions
// ---------------------------------------------------------------------------

impl<'a> StorageScope<'a> {
    /// Narrow to branch-level scope (drop workspace).
    pub fn branch_scope(&self) -> BranchScope<'a> {
        BranchScope {
            tenant_id: self.tenant_id,
            repo_id: self.repo_id,
            branch: self.branch,
        }
    }

    /// Narrow to repo-level scope (drop branch + workspace).
    pub fn repo_scope(&self) -> RepoScope<'a> {
        RepoScope {
            tenant_id: self.tenant_id,
            repo_id: self.repo_id,
        }
    }
}

impl<'a> BranchScope<'a> {
    /// Narrow to repo-level scope (drop branch).
    pub fn repo_scope(&self) -> RepoScope<'a> {
        RepoScope {
            tenant_id: self.tenant_id,
            repo_id: self.repo_id,
        }
    }

    /// Widen to workspace-level scope by adding a workspace.
    pub fn with_workspace(&self, workspace: &'a str) -> StorageScope<'a> {
        StorageScope {
            tenant_id: self.tenant_id,
            repo_id: self.repo_id,
            branch: self.branch,
            workspace,
        }
    }
}

impl<'a> RepoScope<'a> {
    /// Widen to branch-level scope by adding a branch.
    pub fn with_branch(&self, branch: &'a str) -> BranchScope<'a> {
        BranchScope {
            tenant_id: self.tenant_id,
            repo_id: self.repo_id,
            branch,
        }
    }
}

// ---------------------------------------------------------------------------
// Display impls (useful for logging / key prefixes)
// ---------------------------------------------------------------------------

impl fmt::Display for StorageScope<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}/{}/{}/{}",
            self.tenant_id, self.repo_id, self.branch, self.workspace
        )
    }
}

impl fmt::Display for BranchScope<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}/{}", self.tenant_id, self.repo_id, self.branch)
    }
}

impl fmt::Display for RepoScope<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.tenant_id, self.repo_id)
    }
}

// ---------------------------------------------------------------------------
// Owned variants (for caches, job queues, async contexts)
// ---------------------------------------------------------------------------

/// Owned version of [`StorageScope`] for storing in caches or crossing async boundaries.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OwnedStorageScope {
    pub tenant_id: String,
    pub repo_id: String,
    pub branch: String,
    pub workspace: String,
}

/// Owned version of [`BranchScope`] for storing in caches or crossing async boundaries.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OwnedBranchScope {
    pub tenant_id: String,
    pub repo_id: String,
    pub branch: String,
}

/// Owned version of [`RepoScope`] for storing in caches or crossing async boundaries.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OwnedRepoScope {
    pub tenant_id: String,
    pub repo_id: String,
}

// ---------------------------------------------------------------------------
// Owned constructors
// ---------------------------------------------------------------------------

impl OwnedStorageScope {
    pub fn new(
        tenant_id: impl Into<String>,
        repo_id: impl Into<String>,
        branch: impl Into<String>,
        workspace: impl Into<String>,
    ) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            repo_id: repo_id.into(),
            branch: branch.into(),
            workspace: workspace.into(),
        }
    }
}

impl OwnedBranchScope {
    pub fn new(
        tenant_id: impl Into<String>,
        repo_id: impl Into<String>,
        branch: impl Into<String>,
    ) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            repo_id: repo_id.into(),
            branch: branch.into(),
        }
    }
}

impl OwnedRepoScope {
    pub fn new(tenant_id: impl Into<String>, repo_id: impl Into<String>) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            repo_id: repo_id.into(),
        }
    }
}

// ---------------------------------------------------------------------------
// as_ref: Owned -> Borrowed
// ---------------------------------------------------------------------------

impl OwnedStorageScope {
    pub fn as_ref(&self) -> StorageScope<'_> {
        StorageScope {
            tenant_id: &self.tenant_id,
            repo_id: &self.repo_id,
            branch: &self.branch,
            workspace: &self.workspace,
        }
    }
}

impl OwnedBranchScope {
    pub fn as_ref(&self) -> BranchScope<'_> {
        BranchScope {
            tenant_id: &self.tenant_id,
            repo_id: &self.repo_id,
            branch: &self.branch,
        }
    }
}

impl OwnedRepoScope {
    pub fn as_ref(&self) -> RepoScope<'_> {
        RepoScope {
            tenant_id: &self.tenant_id,
            repo_id: &self.repo_id,
        }
    }
}

// ---------------------------------------------------------------------------
// to_owned: Borrowed -> Owned
// ---------------------------------------------------------------------------

impl StorageScope<'_> {
    pub fn to_owned(&self) -> OwnedStorageScope {
        OwnedStorageScope {
            tenant_id: self.tenant_id.to_string(),
            repo_id: self.repo_id.to_string(),
            branch: self.branch.to_string(),
            workspace: self.workspace.to_string(),
        }
    }
}

impl BranchScope<'_> {
    pub fn to_owned(&self) -> OwnedBranchScope {
        OwnedBranchScope {
            tenant_id: self.tenant_id.to_string(),
            repo_id: self.repo_id.to_string(),
            branch: self.branch.to_string(),
        }
    }
}

impl RepoScope<'_> {
    pub fn to_owned(&self) -> OwnedRepoScope {
        OwnedRepoScope {
            tenant_id: self.tenant_id.to_string(),
            repo_id: self.repo_id.to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// Display for owned variants
// ---------------------------------------------------------------------------

impl fmt::Display for OwnedStorageScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}/{}/{}/{}",
            self.tenant_id, self.repo_id, self.branch, self.workspace
        )
    }
}

impl fmt::Display for OwnedBranchScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}/{}", self.tenant_id, self.repo_id, self.branch)
    }
}

impl fmt::Display for OwnedRepoScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.tenant_id, self.repo_id)
    }
}

// ---------------------------------------------------------------------------
// From conversions between owned variants (narrowing)
// ---------------------------------------------------------------------------

impl From<OwnedStorageScope> for OwnedBranchScope {
    fn from(s: OwnedStorageScope) -> Self {
        Self {
            tenant_id: s.tenant_id,
            repo_id: s.repo_id,
            branch: s.branch,
        }
    }
}

impl From<OwnedStorageScope> for OwnedRepoScope {
    fn from(s: OwnedStorageScope) -> Self {
        Self {
            tenant_id: s.tenant_id,
            repo_id: s.repo_id,
        }
    }
}

impl From<OwnedBranchScope> for OwnedRepoScope {
    fn from(s: OwnedBranchScope) -> Self {
        Self {
            tenant_id: s.tenant_id,
            repo_id: s.repo_id,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_storage_scope_display() {
        let scope = StorageScope::new("t1", "r1", "main", "default");
        assert_eq!(scope.to_string(), "t1/r1/main/default");
    }

    #[test]
    fn test_branch_scope_display() {
        let scope = BranchScope::new("t1", "r1", "main");
        assert_eq!(scope.to_string(), "t1/r1/main");
    }

    #[test]
    fn test_repo_scope_display() {
        let scope = RepoScope::new("t1", "r1");
        assert_eq!(scope.to_string(), "t1/r1");
    }

    #[test]
    fn test_narrowing() {
        let scope = StorageScope::new("t1", "r1", "main", "ws1");
        let branch = scope.branch_scope();
        assert_eq!(branch.tenant_id, "t1");
        assert_eq!(branch.repo_id, "r1");
        assert_eq!(branch.branch, "main");

        let repo = branch.repo_scope();
        assert_eq!(repo.tenant_id, "t1");
        assert_eq!(repo.repo_id, "r1");
    }

    #[test]
    fn test_widening() {
        let repo = RepoScope::new("t1", "r1");
        let branch = repo.with_branch("dev");
        assert_eq!(branch.branch, "dev");

        let full = branch.with_workspace("ws1");
        assert_eq!(full.workspace, "ws1");
        assert_eq!(full.tenant_id, "t1");
    }

    #[test]
    fn test_owned_roundtrip() {
        let scope = StorageScope::new("t1", "r1", "main", "ws1");
        let owned = scope.to_owned();
        let back = owned.as_ref();
        assert_eq!(scope, back);
    }

    #[test]
    fn test_owned_narrowing_from() {
        let owned = OwnedStorageScope::new("t1", "r1", "main", "ws1");
        let branch: OwnedBranchScope = owned.clone().into();
        assert_eq!(branch.tenant_id, "t1");
        assert_eq!(branch.branch, "main");

        let repo: OwnedRepoScope = branch.into();
        assert_eq!(repo.tenant_id, "t1");
        assert_eq!(repo.repo_id, "r1");
    }
}
