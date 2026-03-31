//! Branch management keywords

use super::types::{KeywordCategory, KeywordInfo};

/// Branch management keywords
pub(super) fn branch_keywords() -> Vec<KeywordInfo> {
    vec![
        KeywordInfo {
            keyword: "BRANCH".into(),
            category: KeywordCategory::SchemaObject,
            description: "A named reference to a revision in the repository for version control".into(),
            syntax: Some("CREATE BRANCH 'name' [FROM 'source'] [AT REVISION <rev>] [DESCRIPTION 'desc'] [PROTECTED] [UPSTREAM 'branch'] [WITH HISTORY]".into()),
            example: Some("CREATE BRANCH 'feature/new-article' FROM 'main'".into()),
        },
        KeywordInfo {
            keyword: "MERGE".into(),
            category: KeywordCategory::Statement,
            description: "Merges one branch into another with optional strategy".into(),
            syntax: Some("MERGE BRANCH 'source' INTO 'target' [USING FAST_FORWARD|THREE_WAY] [MESSAGE 'msg']".into()),
            example: Some("MERGE BRANCH 'feature/x' INTO 'main' USING THREE_WAY".into()),
        },
        KeywordInfo {
            keyword: "USE".into(),
            category: KeywordCategory::Statement,
            description: "Sets the session branch context for subsequent queries".into(),
            syntax: Some("USE BRANCH 'branch-name'".into()),
            example: Some("USE BRANCH 'develop'".into()),
        },
        KeywordInfo {
            keyword: "CHECKOUT".into(),
            category: KeywordCategory::Statement,
            description: "Alias for USE BRANCH - sets the session branch context".into(),
            syntax: Some("CHECKOUT BRANCH 'branch-name'".into()),
            example: Some("CHECKOUT BRANCH 'feature/x'".into()),
        },
        KeywordInfo {
            keyword: "SHOW BRANCHES".into(),
            category: KeywordCategory::Statement,
            description: "Lists all branches in the repository".into(),
            syntax: Some("SHOW BRANCHES".into()),
            example: Some("SHOW BRANCHES".into()),
        },
        KeywordInfo {
            keyword: "SHOW CURRENT BRANCH".into(),
            category: KeywordCategory::Statement,
            description: "Shows the current session branch".into(),
            syntax: Some("SHOW CURRENT BRANCH".into()),
            example: Some("SHOW CURRENT BRANCH".into()),
        },
        KeywordInfo {
            keyword: "DESCRIBE BRANCH".into(),
            category: KeywordCategory::Statement,
            description: "Shows detailed information about a branch".into(),
            syntax: Some("DESCRIBE BRANCH 'branch-name'".into()),
            example: Some("DESCRIBE BRANCH 'main'".into()),
        },
        KeywordInfo {
            keyword: "SHOW DIVERGENCE".into(),
            category: KeywordCategory::Statement,
            description: "Shows how many commits a branch is ahead/behind another".into(),
            syntax: Some("SHOW DIVERGENCE 'branch' FROM 'base'".into()),
            example: Some("SHOW DIVERGENCE 'feature/x' FROM 'main'".into()),
        },
        KeywordInfo {
            keyword: "AT REVISION".into(),
            category: KeywordCategory::Clause,
            description: "Specifies a revision reference for branching (HLC timestamp or HEAD~N)".into(),
            syntax: Some("AT REVISION <hlc> | HEAD~N | branch~N".into()),
            example: Some("CREATE BRANCH 'hotfix' FROM 'main' AT REVISION HEAD~5".into()),
        },
        KeywordInfo {
            keyword: "INTO".into(),
            category: KeywordCategory::Clause,
            description: "Specifies the target branch for merge operations".into(),
            syntax: Some("INTO 'target-branch'".into()),
            example: Some("MERGE BRANCH 'feature/x' INTO 'main'".into()),
        },
        KeywordInfo {
            keyword: "USING".into(),
            category: KeywordCategory::Clause,
            description: "Specifies the merge strategy".into(),
            syntax: Some("USING FAST_FORWARD | THREE_WAY".into()),
            example: Some("MERGE BRANCH 'hotfix' INTO 'production' USING FAST_FORWARD".into()),
        },
        KeywordInfo {
            keyword: "MESSAGE".into(),
            category: KeywordCategory::Clause,
            description: "Specifies the commit message for a merge".into(),
            syntax: Some("MESSAGE 'commit message'".into()),
            example: Some("MERGE BRANCH 'feature/x' INTO 'main' MESSAGE 'Merge feature X'".into()),
        },
        KeywordInfo {
            keyword: "UPSTREAM".into(),
            category: KeywordCategory::Clause,
            description: "Sets the upstream branch for divergence tracking".into(),
            syntax: Some("UPSTREAM 'branch-name'".into()),
            example: Some("CREATE BRANCH 'feature/x' FROM 'main' UPSTREAM 'main'".into()),
        },
        KeywordInfo {
            keyword: "WITH HISTORY".into(),
            category: KeywordCategory::Modifier,
            description: "Copies revision history when creating a branch".into(),
            syntax: Some("WITH HISTORY".into()),
            example: Some("CREATE BRANCH 'archive' FROM 'main' WITH HISTORY".into()),
        },
        KeywordInfo {
            keyword: "FAST_FORWARD".into(),
            category: KeywordCategory::Modifier,
            description: "Fast-forward merge strategy (only if target is ancestor of source)".into(),
            syntax: Some("USING FAST_FORWARD".into()),
            example: Some("MERGE BRANCH 'hotfix' INTO 'production' USING FAST_FORWARD".into()),
        },
        KeywordInfo {
            keyword: "THREE_WAY".into(),
            category: KeywordCategory::Modifier,
            description: "Three-way merge with conflict detection (default)".into(),
            syntax: Some("USING THREE_WAY".into()),
            example: Some("MERGE BRANCH 'feature/x' INTO 'main' USING THREE_WAY".into()),
        },
        KeywordInfo {
            keyword: "HEAD".into(),
            category: KeywordCategory::Operator,
            description: "Reference to the current HEAD revision of a branch".into(),
            syntax: Some("HEAD~N".into()),
            example: Some("AT REVISION HEAD~5".into()),
        },
        KeywordInfo {
            keyword: "IF EXISTS".into(),
            category: KeywordCategory::Modifier,
            description: "Suppresses error if the object doesn't exist".into(),
            syntax: Some("DROP BRANCH IF EXISTS 'name'".into()),
            example: Some("DROP BRANCH IF EXISTS 'feature/old'".into()),
        },
        KeywordInfo {
            keyword: "RENAME TO".into(),
            category: KeywordCategory::Clause,
            description: "Renames a branch to a new name".into(),
            syntax: Some("ALTER BRANCH 'old' RENAME TO 'new'".into()),
            example: Some("ALTER BRANCH 'old-name' RENAME TO 'new-name'".into()),
        },
        KeywordInfo {
            keyword: "UNSET".into(),
            category: KeywordCategory::Operator,
            description: "Removes a property or setting".into(),
            syntax: Some("UNSET UPSTREAM".into()),
            example: Some("ALTER BRANCH 'feature/x' UNSET UPSTREAM".into()),
        },
    ]
}
