//! Branch, merge, and tag types for repository operations.

use raisin_hlc::HLC;
use serde::{Deserialize, Serialize};

/// Git-like branch information
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Branch {
    /// Branch name (e.g., "main", "develop", "production")
    pub name: String,

    /// Current HEAD revision (Hybrid Logical Clock)
    pub head: HLC,

    /// When the branch was created
    pub created_at: chrono::DateTime<chrono::Utc>,

    /// Who created this branch
    pub created_by: String,

    /// Revision this branch was created from (if branched from another)
    pub created_from: Option<HLC>,

    /// Upstream branch for divergence comparison (e.g., "main", "release/1.0")
    /// If None, defaults to the repository's default branch (usually "main")
    #[serde(default)]
    pub upstream_branch: Option<String>,

    /// Whether this branch is protected from deletion/force updates
    pub protected: bool,

    /// Human-readable description of the branch
    #[serde(default)]
    pub description: Option<String>,
}

/// Branch divergence information (similar to "N commits ahead/behind")
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchDivergence {
    /// Number of commits in the current branch not in the base branch
    pub ahead: u64,

    /// Number of commits in the base branch not in the current branch
    pub behind: u64,

    /// The common ancestor revision between the two branches (HLC)
    pub common_ancestor: HLC,
}

/// Merge strategy for branch merges
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum MergeStrategy {
    /// Fast-forward merge (only possible when target is ancestor of source)
    FastForward,
    /// Three-way merge creating a merge commit
    ThreeWay,
}

/// Type of merge conflict
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConflictType {
    /// Both branches modified the same node
    BothModified,
    /// Source branch deleted a node that target branch modified
    DeletedBySourceModifiedByTarget,
    /// Target branch deleted a node that source branch modified
    ModifiedBySourceDeletedByTarget,
    /// Both branches added a node at the same path
    BothAdded,
}

/// Information about a single merge conflict
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeConflict {
    /// Node ID involved in the conflict
    pub node_id: String,

    /// Path of the conflicting node
    pub path: String,

    /// Type of conflict
    pub conflict_type: ConflictType,

    /// Node state at common ancestor (base)
    pub base_properties: Option<serde_json::Value>,

    /// Node state in target branch (ours)
    pub target_properties: Option<serde_json::Value>,

    /// Node state in source branch (theirs)
    pub source_properties: Option<serde_json::Value>,

    /// Translation locale if this is a translation conflict (None = base node conflict)
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub translation_locale: Option<String>,
}

/// Result of a merge operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeResult {
    /// Whether the merge completed successfully
    pub success: bool,

    /// The revision number of the merge commit (if successful)
    pub revision: Option<u64>,

    /// List of conflicts that need resolution (if any)
    pub conflicts: Vec<MergeConflict>,

    /// Whether this was a fast-forward merge
    pub fast_forward: bool,

    /// Number of nodes changed in the merge
    pub nodes_changed: usize,
}

/// Type of resolution for a merge conflict
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ResolutionType {
    /// Keep the target branch version (ours)
    KeepOurs,
    /// Keep the source branch version (theirs)
    KeepTheirs,
    /// Use manually edited properties
    Manual,
}

/// Resolution for a single conflict
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictResolution {
    /// Node ID being resolved
    pub node_id: String,

    /// Type of resolution chosen
    pub resolution_type: ResolutionType,

    /// The resolved properties (should match the resolution type)
    pub resolved_properties: serde_json::Value,

    /// Translation locale being resolved (None = base node conflict)
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub translation_locale: Option<String>,
}

/// Git-like tag information (immutable revision marker)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tag {
    /// Tag name (e.g., "v1.0.0", "release-2024-01", "milestone-beta")
    pub name: String,

    /// Revision this tag points to (immutable HLC)
    pub revision: HLC,

    /// When the tag was created
    pub created_at: chrono::DateTime<chrono::Utc>,

    /// Actor who created the tag
    pub created_by: String,

    /// Optional annotation/message for the tag
    pub message: Option<String>,

    /// Whether this tag is protected from deletion
    pub protected: bool,
}
