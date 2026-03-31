//! MERGE BRANCH statement and related types

use serde::{Deserialize, Serialize};

/// Merge strategy for MERGE BRANCH
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum MergeStrategy {
    /// Fast-forward merge (only if target is ancestor of source)
    FastForward,
    /// Three-way merge with conflict detection
    #[default]
    ThreeWay,
}

impl std::fmt::Display for MergeStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MergeStrategy::FastForward => write!(f, "FAST_FORWARD"),
            MergeStrategy::ThreeWay => write!(f, "THREE_WAY"),
        }
    }
}

/// Resolution type for SQL conflict resolution
///
/// Used in MERGE BRANCH ... RESOLVE CONFLICTS clause to specify how each conflict should be resolved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SqlResolutionType {
    /// Keep the target branch version (ours)
    KeepOurs,
    /// Keep the source branch version (theirs)
    KeepTheirs,
    /// Delete the node (accept deletion)
    Delete,
    /// Use a custom value (manual merge)
    UseValue(serde_json::Value),
}

impl std::fmt::Display for SqlResolutionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SqlResolutionType::KeepOurs => write!(f, "KEEP_OURS"),
            SqlResolutionType::KeepTheirs => write!(f, "KEEP_THEIRS"),
            SqlResolutionType::Delete => write!(f, "DELETE"),
            SqlResolutionType::UseValue(v) => write!(f, "USE_VALUE '{}'", v),
        }
    }
}

/// Single conflict resolution in SQL MERGE statement
///
/// ```sql
/// -- Node conflict
/// ('node-uuid', KEEP_OURS)
/// -- Translation conflict
/// ('node-uuid', 'en', KEEP_THEIRS)
/// -- Manual value
/// ('node-uuid', USE_VALUE '{"name": "merged"}')
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SqlConflictResolution {
    /// ID of the conflicted node
    pub node_id: String,
    /// Translation locale (None for base node conflicts)
    pub translation_locale: Option<String>,
    /// Resolution type
    pub resolution: SqlResolutionType,
}

impl std::fmt::Display for SqlConflictResolution {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(locale) = &self.translation_locale {
            write!(f, "('{}', '{}', {})", self.node_id, locale, self.resolution)
        } else {
            write!(f, "('{}', {})", self.node_id, self.resolution)
        }
    }
}

/// MERGE BRANCH statement
///
/// ```sql
/// MERGE BRANCH 'feature/x' INTO 'main'
/// MERGE BRANCH 'feature/x' INTO 'main' USING FAST_FORWARD
/// MERGE BRANCH 'feature/x' INTO 'main' USING THREE_WAY MESSAGE 'Merge feature'
/// MERGE BRANCH 'feature/x' INTO 'main' MESSAGE 'Merge' RESOLVE CONFLICTS (
///     ('uuid1', KEEP_OURS),
///     ('uuid2', KEEP_THEIRS)
/// )
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergeBranch {
    /// Source branch to merge from
    pub source_branch: String,
    /// Target branch to merge into
    pub target_branch: String,
    /// Merge strategy (defaults to THREE_WAY if not specified)
    pub strategy: Option<MergeStrategy>,
    /// Commit message for the merge
    pub message: Option<String>,
    /// Conflict resolutions (for merging with known conflicts)
    pub resolutions: Vec<SqlConflictResolution>,
}

impl MergeBranch {
    /// Create a new MERGE BRANCH statement
    pub fn new(source: impl Into<String>, target: impl Into<String>) -> Self {
        Self {
            source_branch: source.into(),
            target_branch: target.into(),
            strategy: None,
            message: None,
            resolutions: Vec::new(),
        }
    }

    /// Set the merge strategy
    pub fn using(mut self, strategy: MergeStrategy) -> Self {
        self.strategy = Some(strategy);
        self
    }

    /// Set the commit message
    pub fn message(mut self, msg: impl Into<String>) -> Self {
        self.message = Some(msg.into());
        self
    }

    /// Add conflict resolutions
    pub fn resolve(mut self, resolutions: Vec<SqlConflictResolution>) -> Self {
        self.resolutions = resolutions;
        self
    }
}

impl std::fmt::Display for MergeBranch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "MERGE BRANCH '{}' INTO '{}'",
            self.source_branch, self.target_branch
        )?;

        if let Some(strategy) = &self.strategy {
            write!(f, " USING {}", strategy)?;
        }

        if let Some(msg) = &self.message {
            write!(f, " MESSAGE '{}'", msg)?;
        }

        if !self.resolutions.is_empty() {
            write!(f, " RESOLVE CONFLICTS (")?;
            for (i, res) in self.resolutions.iter().enumerate() {
                if i > 0 {
                    write!(f, ", ")?;
                }
                write!(f, "{}", res)?;
            }
            write!(f, ")")?;
        }

        Ok(())
    }
}
