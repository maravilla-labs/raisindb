//! BRANCH statement AST definitions
//!
//! Defines the Abstract Syntax Tree for branch management statements:
//! - CREATE BRANCH 'feature/x' FROM 'main' [AT REVISION ...]
//! - DROP BRANCH [IF EXISTS] 'feature/x'
//! - ALTER BRANCH 'feature/x' SET UPSTREAM 'main'
//! - MERGE BRANCH 'feature/x' INTO 'main' [USING FAST_FORWARD]
//! - USE BRANCH 'feature/x' / CHECKOUT BRANCH 'feature/x'
//! - USE LOCAL BRANCH 'feature/x' / SET LOCAL app.branch = 'feature/x'
//! - SET app.branch = 'feature/x' / SET app.branch TO 'feature/x'
//! - SHOW BRANCHES / DESCRIBE BRANCH 'main' / SHOW DIVERGENCE

mod branch_statement;
mod merge;
mod statements;
mod types;

// Re-export all public types to preserve the original module interface
pub use branch_statement::BranchStatement;
pub use merge::{MergeBranch, MergeStrategy, SqlConflictResolution, SqlResolutionType};
pub use statements::{AlterBranch, BranchAlteration, CreateBranch, DropBranch};
pub use types::{BranchScope, RevisionRef};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_revision_ref_hlc() {
        let rev = RevisionRef::hlc("1734567890123_42");
        assert_eq!(rev.to_string(), "1734567890123_42");
    }

    #[test]
    fn test_revision_ref_head_relative() {
        let rev = RevisionRef::head_relative(5);
        assert_eq!(rev.to_string(), "HEAD~5");
    }

    #[test]
    fn test_revision_ref_branch_relative() {
        let rev = RevisionRef::branch_relative("main", 3);
        assert_eq!(rev.to_string(), "main~3");
    }

    #[test]
    fn test_create_branch_simple() {
        let stmt = CreateBranch::new("feature/x");
        assert_eq!(stmt.to_string(), "CREATE BRANCH 'feature/x'");
    }

    #[test]
    fn test_create_branch_from() {
        let stmt = CreateBranch::from("feature/x", "main");
        assert_eq!(stmt.to_string(), "CREATE BRANCH 'feature/x' FROM 'main'");
    }

    #[test]
    fn test_create_branch_full() {
        let stmt = CreateBranch::from("feature/x", "main")
            .at_revision(RevisionRef::head_relative(2))
            .description("New feature branch")
            .protected()
            .upstream("main")
            .with_history();

        assert_eq!(
            stmt.to_string(),
            "CREATE BRANCH 'feature/x' FROM 'main' AT REVISION HEAD~2 DESCRIPTION 'New feature branch' PROTECTED UPSTREAM 'main' WITH HISTORY"
        );
    }

    #[test]
    fn test_drop_branch() {
        let stmt = DropBranch::new("feature/x");
        assert_eq!(stmt.to_string(), "DROP BRANCH 'feature/x'");
    }

    #[test]
    fn test_drop_branch_if_exists() {
        let stmt = DropBranch::if_exists("feature/x");
        assert_eq!(stmt.to_string(), "DROP BRANCH IF EXISTS 'feature/x'");
    }

    #[test]
    fn test_alter_branch_set_upstream() {
        let stmt = AlterBranch::set_upstream("feature/x", "main");
        assert_eq!(
            stmt.to_string(),
            "ALTER BRANCH 'feature/x' SET UPSTREAM 'main'"
        );
    }

    #[test]
    fn test_alter_branch_unset_upstream() {
        let stmt = AlterBranch::unset_upstream("feature/x");
        assert_eq!(stmt.to_string(), "ALTER BRANCH 'feature/x' UNSET UPSTREAM");
    }

    #[test]
    fn test_alter_branch_set_protected() {
        let stmt = AlterBranch::set_protected("production", true);
        assert_eq!(
            stmt.to_string(),
            "ALTER BRANCH 'production' SET PROTECTED true"
        );
    }

    #[test]
    fn test_alter_branch_rename() {
        let stmt = AlterBranch::rename_to("old-name", "new-name");
        assert_eq!(
            stmt.to_string(),
            "ALTER BRANCH 'old-name' RENAME TO 'new-name'"
        );
    }

    #[test]
    fn test_merge_branch_simple() {
        let stmt = MergeBranch::new("feature/x", "main");
        assert_eq!(stmt.to_string(), "MERGE BRANCH 'feature/x' INTO 'main'");
    }

    #[test]
    fn test_merge_branch_with_strategy() {
        let stmt = MergeBranch::new("feature/x", "main").using(MergeStrategy::FastForward);
        assert_eq!(
            stmt.to_string(),
            "MERGE BRANCH 'feature/x' INTO 'main' USING FAST_FORWARD"
        );
    }

    #[test]
    fn test_merge_branch_full() {
        let stmt = MergeBranch::new("feature/x", "main")
            .using(MergeStrategy::ThreeWay)
            .message("Merge feature X");
        assert_eq!(
            stmt.to_string(),
            "MERGE BRANCH 'feature/x' INTO 'main' USING THREE_WAY MESSAGE 'Merge feature X'"
        );
    }

    #[test]
    fn test_branch_statement_use_session() {
        let stmt = BranchStatement::UseBranch {
            name: "develop".to_string(),
            scope: BranchScope::Session,
        };
        assert_eq!(stmt.to_string(), "USE BRANCH 'develop'");
        assert_eq!(stmt.operation(), "USE BRANCH");
    }

    #[test]
    fn test_branch_statement_use_local() {
        let stmt = BranchStatement::UseBranch {
            name: "feature/x".to_string(),
            scope: BranchScope::Local,
        };
        assert_eq!(stmt.to_string(), "USE LOCAL BRANCH 'feature/x'");
        assert_eq!(stmt.operation(), "USE LOCAL BRANCH");
    }

    #[test]
    fn test_branch_statement_show_branches() {
        let stmt = BranchStatement::ShowBranches;
        assert_eq!(stmt.to_string(), "SHOW BRANCHES");
    }

    #[test]
    fn test_branch_statement_describe() {
        let stmt = BranchStatement::DescribeBranch("main".to_string());
        assert_eq!(stmt.to_string(), "DESCRIBE BRANCH 'main'");
    }

    #[test]
    fn test_branch_statement_show_divergence() {
        let stmt = BranchStatement::ShowDivergence {
            branch: "feature/x".to_string(),
            from: "main".to_string(),
        };
        assert_eq!(stmt.to_string(), "SHOW DIVERGENCE 'feature/x' FROM 'main'");
    }
}
