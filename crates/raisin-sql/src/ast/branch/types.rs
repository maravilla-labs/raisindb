//! Common branch types (scope, revision references)

use serde::{Deserialize, Serialize};

/// Scope for branch context setting
///
/// Determines how long the branch setting persists:
/// - Session: Persists for the connection lifetime (pgwire/WS) or batch (HTTP)
/// - Local: Affects only the next query, then reverts to session/default
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum BranchScope {
    /// Session scope - persists for connection (SET / USE BRANCH)
    #[default]
    Session,
    /// Local scope - single statement only (SET LOCAL / USE LOCAL BRANCH)
    Local,
}

impl std::fmt::Display for BranchScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BranchScope::Session => write!(f, "SESSION"),
            BranchScope::Local => write!(f, "LOCAL"),
        }
    }
}

/// Revision reference - can be absolute HLC or relative (Git-like)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RevisionRef {
    /// Absolute HLC: "1734567890123_42" (timestamp_counter format)
    Hlc(String),
    /// Relative to HEAD: HEAD~N
    HeadRelative(u32),
    /// Relative to branch: branch~N
    BranchRelative { branch: String, offset: u32 },
}

impl RevisionRef {
    /// Create an HLC revision reference
    pub fn hlc(hlc: impl Into<String>) -> Self {
        RevisionRef::Hlc(hlc.into())
    }

    /// Create a HEAD-relative reference
    pub fn head_relative(offset: u32) -> Self {
        RevisionRef::HeadRelative(offset)
    }

    /// Create a branch-relative reference
    pub fn branch_relative(branch: impl Into<String>, offset: u32) -> Self {
        RevisionRef::BranchRelative {
            branch: branch.into(),
            offset,
        }
    }
}

impl std::fmt::Display for RevisionRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RevisionRef::Hlc(hlc) => write!(f, "{}", hlc),
            RevisionRef::HeadRelative(n) => write!(f, "HEAD~{}", n),
            RevisionRef::BranchRelative { branch, offset } => write!(f, "{}~{}", branch, offset),
        }
    }
}
