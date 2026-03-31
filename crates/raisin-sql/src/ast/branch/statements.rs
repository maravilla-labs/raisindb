//! Branch statement structs (CREATE, DROP, ALTER, MERGE)

use serde::{Deserialize, Serialize};

use super::types::RevisionRef;

/// CREATE BRANCH statement
///
/// ```sql
/// CREATE BRANCH 'feature/x' FROM 'main' AT REVISION HEAD~2 DESCRIPTION 'desc' PROTECTED UPSTREAM 'main' WITH HISTORY
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateBranch {
    /// The name of the new branch
    pub name: String,
    /// Source branch to create from (optional - creates orphan branch if None)
    pub from_branch: Option<String>,
    /// Specific revision to branch from (optional - uses HEAD if None)
    pub at_revision: Option<RevisionRef>,
    /// Branch description
    pub description: Option<String>,
    /// Whether the branch is protected from deletion
    pub protected: bool,
    /// Upstream branch for divergence tracking
    pub upstream: Option<String>,
    /// Whether to copy revision history from source branch
    pub with_history: bool,
}

impl CreateBranch {
    /// Create a new CREATE BRANCH statement with minimal options
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            from_branch: None,
            at_revision: None,
            description: None,
            protected: false,
            upstream: None,
            with_history: false,
        }
    }

    /// Create a branch from another branch
    pub fn from(name: impl Into<String>, from_branch: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            from_branch: Some(from_branch.into()),
            at_revision: None,
            description: None,
            protected: false,
            upstream: None,
            with_history: false,
        }
    }

    /// Set the revision to branch from
    pub fn at_revision(mut self, rev: RevisionRef) -> Self {
        self.at_revision = Some(rev);
        self
    }

    /// Set the branch description
    pub fn description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Mark the branch as protected
    pub fn protected(mut self) -> Self {
        self.protected = true;
        self
    }

    /// Set the upstream branch
    pub fn upstream(mut self, branch: impl Into<String>) -> Self {
        self.upstream = Some(branch.into());
        self
    }

    /// Enable history copying
    pub fn with_history(mut self) -> Self {
        self.with_history = true;
        self
    }
}

impl std::fmt::Display for CreateBranch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CREATE BRANCH '{}'", self.name)?;

        if let Some(ref from) = self.from_branch {
            write!(f, " FROM '{}'", from)?;
        }

        if let Some(ref rev) = self.at_revision {
            write!(f, " AT REVISION {}", rev)?;
        }

        if let Some(ref desc) = self.description {
            write!(f, " DESCRIPTION '{}'", desc)?;
        }

        if self.protected {
            write!(f, " PROTECTED")?;
        }

        if let Some(ref upstream) = self.upstream {
            write!(f, " UPSTREAM '{}'", upstream)?;
        }

        if self.with_history {
            write!(f, " WITH HISTORY")?;
        }

        Ok(())
    }
}

/// DROP BRANCH statement
///
/// ```sql
/// DROP BRANCH 'feature/x'
/// DROP BRANCH IF EXISTS 'feature/x'
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DropBranch {
    /// The name of the branch to drop
    pub name: String,
    /// Whether to suppress error if branch doesn't exist
    pub if_exists: bool,
}

impl DropBranch {
    /// Create a new DROP BRANCH statement
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            if_exists: false,
        }
    }

    /// Create a DROP BRANCH IF EXISTS statement
    pub fn if_exists(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            if_exists: true,
        }
    }
}

impl std::fmt::Display for DropBranch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.if_exists {
            write!(f, "DROP BRANCH IF EXISTS '{}'", self.name)
        } else {
            write!(f, "DROP BRANCH '{}'", self.name)
        }
    }
}

/// Alteration operation for ALTER BRANCH
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BranchAlteration {
    /// SET UPSTREAM 'branch'
    SetUpstream(String),
    /// UNSET UPSTREAM
    UnsetUpstream,
    /// SET PROTECTED TRUE/FALSE
    SetProtected(bool),
    /// SET DESCRIPTION 'description'
    SetDescription(String),
    /// RENAME TO 'new_name'
    RenameTo(String),
}

impl std::fmt::Display for BranchAlteration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BranchAlteration::SetUpstream(branch) => write!(f, "SET UPSTREAM '{}'", branch),
            BranchAlteration::UnsetUpstream => write!(f, "UNSET UPSTREAM"),
            BranchAlteration::SetProtected(val) => write!(f, "SET PROTECTED {}", val),
            BranchAlteration::SetDescription(desc) => write!(f, "SET DESCRIPTION '{}'", desc),
            BranchAlteration::RenameTo(name) => write!(f, "RENAME TO '{}'", name),
        }
    }
}

/// ALTER BRANCH statement
///
/// ```sql
/// ALTER BRANCH 'feature/x' SET UPSTREAM 'main'
/// ALTER BRANCH 'feature/x' SET PROTECTED TRUE
/// ALTER BRANCH 'old-name' RENAME TO 'new-name'
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlterBranch {
    /// The name of the branch to alter
    pub name: String,
    /// The alteration to apply
    pub alteration: BranchAlteration,
}

impl AlterBranch {
    /// Create a new ALTER BRANCH statement
    pub fn new(name: impl Into<String>, alteration: BranchAlteration) -> Self {
        Self {
            name: name.into(),
            alteration,
        }
    }

    /// Set upstream branch
    pub fn set_upstream(name: impl Into<String>, upstream: impl Into<String>) -> Self {
        Self::new(name, BranchAlteration::SetUpstream(upstream.into()))
    }

    /// Unset upstream branch
    pub fn unset_upstream(name: impl Into<String>) -> Self {
        Self::new(name, BranchAlteration::UnsetUpstream)
    }

    /// Set protected status
    pub fn set_protected(name: impl Into<String>, protected: bool) -> Self {
        Self::new(name, BranchAlteration::SetProtected(protected))
    }

    /// Set description
    pub fn set_description(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self::new(name, BranchAlteration::SetDescription(description.into()))
    }

    /// Rename branch
    pub fn rename_to(name: impl Into<String>, new_name: impl Into<String>) -> Self {
        Self::new(name, BranchAlteration::RenameTo(new_name.into()))
    }
}

impl std::fmt::Display for AlterBranch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ALTER BRANCH '{}' {}", self.name, self.alteration)
    }
}
