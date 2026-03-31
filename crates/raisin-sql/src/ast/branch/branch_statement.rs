//! BranchStatement enum (the top-level branch statement type)

use serde::{Deserialize, Serialize};

use super::merge::MergeBranch;
use super::statements::{AlterBranch, CreateBranch, DropBranch};
use super::types::BranchScope;

/// All branch-related statements
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BranchStatement {
    /// CREATE BRANCH statement
    Create(CreateBranch),
    /// DROP BRANCH statement
    Drop(DropBranch),
    /// ALTER BRANCH statement
    Alter(AlterBranch),
    /// MERGE BRANCH statement
    Merge(MergeBranch),
    /// USE BRANCH / CHECKOUT BRANCH / SET app.branch statement (sets branch context)
    ///
    /// Scope determines persistence:
    /// - Session: Persists for connection lifetime (USE BRANCH, SET app.branch)
    /// - Local: Single statement only (USE LOCAL BRANCH, SET LOCAL app.branch)
    UseBranch { name: String, scope: BranchScope },
    /// SHOW CURRENT BRANCH / SHOW app.branch statement
    ShowCurrentBranch,
    /// SHOW BRANCHES statement
    ShowBranches,
    /// DESCRIBE BRANCH statement
    DescribeBranch(String),
    /// SHOW DIVERGENCE statement
    ShowDivergence { branch: String, from: String },
    /// SHOW CONFLICTS FOR MERGE 'source' INTO 'target' statement
    ShowConflicts { source: String, target: String },
}

impl BranchStatement {
    /// Get the operation kind as a string
    pub fn operation(&self) -> &'static str {
        match self {
            BranchStatement::Create(_) => "CREATE BRANCH",
            BranchStatement::Drop(_) => "DROP BRANCH",
            BranchStatement::Alter(_) => "ALTER BRANCH",
            BranchStatement::Merge(_) => "MERGE BRANCH",
            BranchStatement::UseBranch { scope, .. } => match scope {
                BranchScope::Session => "USE BRANCH",
                BranchScope::Local => "USE LOCAL BRANCH",
            },
            BranchStatement::ShowCurrentBranch => "SHOW CURRENT BRANCH",
            BranchStatement::ShowBranches => "SHOW BRANCHES",
            BranchStatement::DescribeBranch(_) => "DESCRIBE BRANCH",
            BranchStatement::ShowDivergence { .. } => "SHOW DIVERGENCE",
            BranchStatement::ShowConflicts { .. } => "SHOW CONFLICTS",
        }
    }
}

impl std::fmt::Display for BranchStatement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BranchStatement::Create(stmt) => write!(f, "{}", stmt),
            BranchStatement::Drop(stmt) => write!(f, "{}", stmt),
            BranchStatement::Alter(stmt) => write!(f, "{}", stmt),
            BranchStatement::Merge(stmt) => write!(f, "{}", stmt),
            BranchStatement::UseBranch { name, scope } => match scope {
                BranchScope::Session => write!(f, "USE BRANCH '{}'", name),
                BranchScope::Local => write!(f, "USE LOCAL BRANCH '{}'", name),
            },
            BranchStatement::ShowCurrentBranch => write!(f, "SHOW CURRENT BRANCH"),
            BranchStatement::ShowBranches => write!(f, "SHOW BRANCHES"),
            BranchStatement::DescribeBranch(name) => write!(f, "DESCRIBE BRANCH '{}'", name),
            BranchStatement::ShowDivergence { branch, from } => {
                write!(f, "SHOW DIVERGENCE '{}' FROM '{}'", branch, from)
            }
            BranchStatement::ShowConflicts { source, target } => {
                write!(f, "SHOW CONFLICTS FOR MERGE '{}' INTO '{}'", source, target)
            }
        }
    }
}
