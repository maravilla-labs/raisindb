//\! Tests for BRANCH statement parsing

use super::super::branch::{
    BranchAlteration, BranchScope, BranchStatement, MergeStrategy, RevisionRef, SqlResolutionType,
};
use super::*;

#[test]
fn test_is_branch_statement() {
    assert!(is_branch_statement("CREATE BRANCH 'feature/x' FROM 'main'"));
    assert!(is_branch_statement("DROP BRANCH 'feature/x'"));
    assert!(is_branch_statement("DROP BRANCH IF EXISTS 'feature/x'"));
    assert!(is_branch_statement(
        "ALTER BRANCH 'feature/x' SET UPSTREAM 'main'"
    ));
    assert!(is_branch_statement("MERGE BRANCH 'feature/x' INTO 'main'"));
    assert!(is_branch_statement("USE BRANCH 'develop'"));
    assert!(is_branch_statement("USE LOCAL BRANCH 'feature/x'"));
    assert!(is_branch_statement("CHECKOUT BRANCH develop"));
    assert!(is_branch_statement("SHOW BRANCHES"));
    assert!(is_branch_statement("SHOW CURRENT BRANCH"));
    assert!(is_branch_statement("DESCRIBE BRANCH 'main'"));
    assert!(is_branch_statement(
        "SHOW DIVERGENCE 'feature/x' FROM 'main'"
    ));
    // PostgreSQL-compatible SET app.branch syntax
    assert!(is_branch_statement("SET app.branch = 'develop'"));
    assert!(is_branch_statement("SET app.branch TO 'develop'"));
    assert!(is_branch_statement("SET LOCAL app.branch = 'feature/x'"));
    assert!(is_branch_statement("SET LOCAL app.branch TO 'feature/x'"));
    assert!(is_branch_statement("SHOW app.branch"));
}

#[test]
fn test_is_not_branch_statement() {
    assert!(!is_branch_statement("SELECT * FROM nodes"));
    assert!(!is_branch_statement("CREATE NODETYPE 'myapp:Article'"));
    assert!(!is_branch_statement(
        "ORDER Page SET path='/a' ABOVE path='/b'"
    ));
}

// ========================================================================
// CREATE BRANCH tests
// ========================================================================

#[test]
fn test_parse_create_branch_simple() {
    let sql = "CREATE BRANCH 'feature/x'";
    let result = parse_branch(sql).unwrap().unwrap();
    if let BranchStatement::Create(stmt) = result {
        assert_eq!(stmt.name, "feature/x");
        assert!(stmt.from_branch.is_none());
    } else {
        panic!("Expected Create statement");
    }
}

#[test]
fn test_parse_create_branch_unquoted() {
    let sql = "CREATE BRANCH feature_x FROM main";
    let result = parse_branch(sql).unwrap().unwrap();
    if let BranchStatement::Create(stmt) = result {
        assert_eq!(stmt.name, "feature_x");
        assert_eq!(stmt.from_branch, Some("main".to_string()));
    } else {
        panic!("Expected Create statement");
    }
}

#[test]
fn test_parse_create_branch_from() {
    let sql = "CREATE BRANCH 'feature/x' FROM 'main'";
    let result = parse_branch(sql).unwrap().unwrap();
    if let BranchStatement::Create(stmt) = result {
        assert_eq!(stmt.name, "feature/x");
        assert_eq!(stmt.from_branch, Some("main".to_string()));
    } else {
        panic!("Expected Create statement");
    }
}

#[test]
fn test_parse_create_branch_at_revision_hlc() {
    let sql = "CREATE BRANCH 'hotfix' FROM 'main' AT REVISION 1734567890123_42";
    let result = parse_branch(sql).unwrap().unwrap();
    if let BranchStatement::Create(stmt) = result {
        assert_eq!(stmt.name, "hotfix");
        assert_eq!(
            stmt.at_revision,
            Some(RevisionRef::Hlc("1734567890123_42".to_string()))
        );
    } else {
        panic!("Expected Create statement");
    }
}

#[test]
fn test_parse_create_branch_at_revision_head_relative() {
    let sql = "CREATE BRANCH 'hotfix' FROM 'main' AT REVISION HEAD~5";
    let result = parse_branch(sql).unwrap().unwrap();
    if let BranchStatement::Create(stmt) = result {
        assert_eq!(stmt.at_revision, Some(RevisionRef::HeadRelative(5)));
    } else {
        panic!("Expected Create statement");
    }
}

#[test]
fn test_parse_create_branch_at_revision_branch_relative() {
    let sql = "CREATE BRANCH 'hotfix' FROM 'production' AT REVISION main~3";
    let result = parse_branch(sql).unwrap().unwrap();
    if let BranchStatement::Create(stmt) = result {
        assert_eq!(
            stmt.at_revision,
            Some(RevisionRef::BranchRelative {
                branch: "main".to_string(),
                offset: 3
            })
        );
    } else {
        panic!("Expected Create statement");
    }
}

#[test]
fn test_parse_create_branch_full() {
    let sql = "CREATE BRANCH 'develop' FROM 'main' AT REVISION HEAD~2 DESCRIPTION 'Development branch' PROTECTED UPSTREAM 'main' WITH HISTORY";
    let result = parse_branch(sql).unwrap().unwrap();
    if let BranchStatement::Create(stmt) = result {
        assert_eq!(stmt.name, "develop");
        assert_eq!(stmt.from_branch, Some("main".to_string()));
        assert_eq!(stmt.at_revision, Some(RevisionRef::HeadRelative(2)));
        assert_eq!(stmt.description, Some("Development branch".to_string()));
        assert!(stmt.protected);
        assert_eq!(stmt.upstream, Some("main".to_string()));
        assert!(stmt.with_history);
    } else {
        panic!("Expected Create statement");
    }
}

#[test]
fn test_parse_create_branch_clauses_any_order() {
    // Test that clauses can appear in any order
    let sql = "CREATE BRANCH 'develop' PROTECTED FROM 'main' WITH HISTORY DESCRIPTION 'dev'";
    let result = parse_branch(sql).unwrap().unwrap();
    if let BranchStatement::Create(stmt) = result {
        assert_eq!(stmt.from_branch, Some("main".to_string()));
        assert!(stmt.protected);
        assert!(stmt.with_history);
        assert_eq!(stmt.description, Some("dev".to_string()));
    } else {
        panic!("Expected Create statement");
    }
}

// ========================================================================
// DROP BRANCH tests
// ========================================================================

#[test]
fn test_parse_drop_branch() {
    let sql = "DROP BRANCH 'feature/old'";
    let result = parse_branch(sql).unwrap().unwrap();
    if let BranchStatement::Drop(stmt) = result {
        assert_eq!(stmt.name, "feature/old");
        assert!(!stmt.if_exists);
    } else {
        panic!("Expected Drop statement");
    }
}

#[test]
fn test_parse_drop_branch_if_exists() {
    let sql = "DROP BRANCH IF EXISTS 'feature/maybe'";
    let result = parse_branch(sql).unwrap().unwrap();
    if let BranchStatement::Drop(stmt) = result {
        assert_eq!(stmt.name, "feature/maybe");
        assert!(stmt.if_exists);
    } else {
        panic!("Expected Drop statement");
    }
}

// ========================================================================
// ALTER BRANCH tests
// ========================================================================

#[test]
fn test_parse_alter_branch_set_upstream() {
    let sql = "ALTER BRANCH 'feature/x' SET UPSTREAM 'main'";
    let result = parse_branch(sql).unwrap().unwrap();
    if let BranchStatement::Alter(stmt) = result {
        assert_eq!(stmt.name, "feature/x");
        assert_eq!(
            stmt.alteration,
            BranchAlteration::SetUpstream("main".to_string())
        );
    } else {
        panic!("Expected Alter statement");
    }
}

#[test]
fn test_parse_alter_branch_unset_upstream() {
    let sql = "ALTER BRANCH 'feature/x' UNSET UPSTREAM";
    let result = parse_branch(sql).unwrap().unwrap();
    if let BranchStatement::Alter(stmt) = result {
        assert_eq!(stmt.alteration, BranchAlteration::UnsetUpstream);
    } else {
        panic!("Expected Alter statement");
    }
}

#[test]
fn test_parse_alter_branch_set_protected() {
    let sql = "ALTER BRANCH 'production' SET PROTECTED TRUE";
    let result = parse_branch(sql).unwrap().unwrap();
    if let BranchStatement::Alter(stmt) = result {
        assert_eq!(stmt.alteration, BranchAlteration::SetProtected(true));
    } else {
        panic!("Expected Alter statement");
    }
}

#[test]
fn test_parse_alter_branch_set_description() {
    let sql = "ALTER BRANCH 'develop' SET DESCRIPTION 'Main dev branch'";
    let result = parse_branch(sql).unwrap().unwrap();
    if let BranchStatement::Alter(stmt) = result {
        assert_eq!(
            stmt.alteration,
            BranchAlteration::SetDescription("Main dev branch".to_string())
        );
    } else {
        panic!("Expected Alter statement");
    }
}

#[test]
fn test_parse_alter_branch_rename() {
    let sql = "ALTER BRANCH 'old-name' RENAME TO 'new-name'";
    let result = parse_branch(sql).unwrap().unwrap();
    if let BranchStatement::Alter(stmt) = result {
        assert_eq!(
            stmt.alteration,
            BranchAlteration::RenameTo("new-name".to_string())
        );
    } else {
        panic!("Expected Alter statement");
    }
}

// ========================================================================
// MERGE BRANCH tests
// ========================================================================

#[test]
fn test_parse_merge_branch_simple() {
    let sql = "MERGE BRANCH 'feature/x' INTO 'main'";
    let result = parse_branch(sql).unwrap().unwrap();
    if let BranchStatement::Merge(stmt) = result {
        assert_eq!(stmt.source_branch, "feature/x");
        assert_eq!(stmt.target_branch, "main");
        assert!(stmt.strategy.is_none());
        assert!(stmt.message.is_none());
    } else {
        panic!("Expected Merge statement");
    }
}

#[test]
fn test_parse_merge_branch_with_strategy() {
    let sql = "MERGE BRANCH 'hotfix' INTO 'production' USING FAST_FORWARD";
    let result = parse_branch(sql).unwrap().unwrap();
    if let BranchStatement::Merge(stmt) = result {
        assert_eq!(stmt.strategy, Some(MergeStrategy::FastForward));
    } else {
        panic!("Expected Merge statement");
    }
}

#[test]
fn test_parse_merge_branch_full() {
    let sql = "MERGE BRANCH 'feature/x' INTO 'develop' USING THREE_WAY MESSAGE 'Merge feature X'";
    let result = parse_branch(sql).unwrap().unwrap();
    if let BranchStatement::Merge(stmt) = result {
        assert_eq!(stmt.strategy, Some(MergeStrategy::ThreeWay));
        assert_eq!(stmt.message, Some("Merge feature X".to_string()));
    } else {
        panic!("Expected Merge statement");
    }
}

// ========================================================================
// USE/CHECKOUT BRANCH tests
// ========================================================================

#[test]
fn test_parse_use_branch() {
    let sql = "USE BRANCH 'develop'";
    let result = parse_branch(sql).unwrap().unwrap();
    assert_eq!(
        result,
        BranchStatement::UseBranch {
            name: "develop".to_string(),
            scope: BranchScope::Session
        }
    );
}

#[test]
fn test_parse_use_local_branch() {
    let sql = "USE LOCAL BRANCH 'feature/x'";
    let result = parse_branch(sql).unwrap().unwrap();
    assert_eq!(
        result,
        BranchStatement::UseBranch {
            name: "feature/x".to_string(),
            scope: BranchScope::Local
        }
    );
}

#[test]
fn test_parse_checkout_branch() {
    let sql = "CHECKOUT BRANCH develop";
    let result = parse_branch(sql).unwrap().unwrap();
    assert_eq!(
        result,
        BranchStatement::UseBranch {
            name: "develop".to_string(),
            scope: BranchScope::Session
        }
    );
}

// ========================================================================
// SET app.branch tests
// ========================================================================

#[test]
fn test_parse_set_app_branch_equals() {
    let sql = "SET app.branch = 'develop'";
    let result = parse_branch(sql).unwrap().unwrap();
    assert_eq!(
        result,
        BranchStatement::UseBranch {
            name: "develop".to_string(),
            scope: BranchScope::Session
        }
    );
}

#[test]
fn test_parse_set_app_branch_to() {
    let sql = "SET app.branch TO 'develop'";
    let result = parse_branch(sql).unwrap().unwrap();
    assert_eq!(
        result,
        BranchStatement::UseBranch {
            name: "develop".to_string(),
            scope: BranchScope::Session
        }
    );
}

#[test]
fn test_parse_set_local_app_branch_equals() {
    let sql = "SET LOCAL app.branch = 'feature/x'";
    let result = parse_branch(sql).unwrap().unwrap();
    assert_eq!(
        result,
        BranchStatement::UseBranch {
            name: "feature/x".to_string(),
            scope: BranchScope::Local
        }
    );
}

#[test]
fn test_parse_set_local_app_branch_to() {
    let sql = "SET LOCAL app.branch TO feature_branch";
    let result = parse_branch(sql).unwrap().unwrap();
    assert_eq!(
        result,
        BranchStatement::UseBranch {
            name: "feature_branch".to_string(),
            scope: BranchScope::Local
        }
    );
}

#[test]
fn test_parse_show_app_branch() {
    let sql = "SHOW app.branch";
    let result = parse_branch(sql).unwrap().unwrap();
    assert_eq!(result, BranchStatement::ShowCurrentBranch);
}

// ========================================================================
// SHOW statement tests
// ========================================================================

#[test]
fn test_parse_show_branches() {
    let sql = "SHOW BRANCHES";
    let result = parse_branch(sql).unwrap().unwrap();
    assert_eq!(result, BranchStatement::ShowBranches);
}

#[test]
fn test_parse_show_current_branch() {
    let sql = "SHOW CURRENT BRANCH";
    let result = parse_branch(sql).unwrap().unwrap();
    assert_eq!(result, BranchStatement::ShowCurrentBranch);
}

#[test]
fn test_parse_describe_branch() {
    let sql = "DESCRIBE BRANCH 'main'";
    let result = parse_branch(sql).unwrap().unwrap();
    assert_eq!(result, BranchStatement::DescribeBranch("main".to_string()));
}

#[test]
fn test_parse_show_divergence() {
    let sql = "SHOW DIVERGENCE 'feature/x' FROM 'main'";
    let result = parse_branch(sql).unwrap().unwrap();
    assert_eq!(
        result,
        BranchStatement::ShowDivergence {
            branch: "feature/x".to_string(),
            from: "main".to_string()
        }
    );
}

// ========================================================================
// SHOW CONFLICTS tests
// ========================================================================

#[test]
fn test_parse_show_conflicts() {
    let sql = "SHOW CONFLICTS FOR MERGE 'feature/x' INTO 'main'";
    let result = parse_branch(sql).unwrap().unwrap();
    assert_eq!(
        result,
        BranchStatement::ShowConflicts {
            source: "feature/x".to_string(),
            target: "main".to_string()
        }
    );
}

#[test]
fn test_parse_show_conflicts_unquoted() {
    let sql = "SHOW CONFLICTS FOR MERGE feature_x INTO main";
    let result = parse_branch(sql).unwrap().unwrap();
    assert_eq!(
        result,
        BranchStatement::ShowConflicts {
            source: "feature_x".to_string(),
            target: "main".to_string()
        }
    );
}

// ========================================================================
// MERGE with RESOLVE CONFLICTS tests
// ========================================================================

#[test]
fn test_parse_merge_with_resolve_conflicts() {
    let sql = r#"MERGE BRANCH 'feature' INTO 'main' MESSAGE 'Merge feature' RESOLVE CONFLICTS (
        ('uuid1', KEEP_OURS),
        ('uuid2', KEEP_THEIRS)
    )"#;
    let result = parse_branch(sql).unwrap().unwrap();
    if let BranchStatement::Merge(merge) = result {
        assert_eq!(merge.source_branch, "feature");
        assert_eq!(merge.target_branch, "main");
        assert_eq!(merge.message, Some("Merge feature".to_string()));
        assert_eq!(merge.resolutions.len(), 2);
        assert_eq!(merge.resolutions[0].node_id, "uuid1");
        assert_eq!(merge.resolutions[0].resolution, SqlResolutionType::KeepOurs);
        assert_eq!(merge.resolutions[1].node_id, "uuid2");
        assert_eq!(
            merge.resolutions[1].resolution,
            SqlResolutionType::KeepTheirs
        );
    } else {
        panic!("Expected Merge statement");
    }
}

#[test]
fn test_parse_merge_with_delete_resolution() {
    let sql = "MERGE BRANCH 'feature' INTO 'main' RESOLVE CONFLICTS (('uuid1', DELETE))";
    let result = parse_branch(sql).unwrap().unwrap();
    if let BranchStatement::Merge(merge) = result {
        assert_eq!(merge.resolutions.len(), 1);
        assert_eq!(merge.resolutions[0].node_id, "uuid1");
        assert_eq!(merge.resolutions[0].resolution, SqlResolutionType::Delete);
    } else {
        panic!("Expected Merge statement");
    }
}

#[test]
fn test_parse_merge_with_use_value_resolution() {
    let sql = r#"MERGE BRANCH 'feature' INTO 'main' RESOLVE CONFLICTS (
        ('uuid1', USE_VALUE '{"name": "merged"}')
    )"#;
    let result = parse_branch(sql).unwrap().unwrap();
    if let BranchStatement::Merge(merge) = result {
        assert_eq!(merge.resolutions.len(), 1);
        assert_eq!(merge.resolutions[0].node_id, "uuid1");
        if let SqlResolutionType::UseValue(v) = &merge.resolutions[0].resolution {
            assert_eq!(v["name"], "merged");
        } else {
            panic!("Expected UseValue resolution");
        }
    } else {
        panic!("Expected Merge statement");
    }
}

#[test]
fn test_parse_merge_with_locale_resolution() {
    let sql = r#"MERGE BRANCH 'feature' INTO 'main' RESOLVE CONFLICTS (
        ('uuid1', 'en', KEEP_OURS),
        ('uuid1', 'de', KEEP_THEIRS)
    )"#;
    let result = parse_branch(sql).unwrap().unwrap();
    if let BranchStatement::Merge(merge) = result {
        assert_eq!(merge.resolutions.len(), 2);
        assert_eq!(merge.resolutions[0].node_id, "uuid1");
        assert_eq!(
            merge.resolutions[0].translation_locale,
            Some("en".to_string())
        );
        assert_eq!(merge.resolutions[0].resolution, SqlResolutionType::KeepOurs);
        assert_eq!(merge.resolutions[1].node_id, "uuid1");
        assert_eq!(
            merge.resolutions[1].translation_locale,
            Some("de".to_string())
        );
        assert_eq!(
            merge.resolutions[1].resolution,
            SqlResolutionType::KeepTheirs
        );
    } else {
        panic!("Expected Merge statement");
    }
}

// ========================================================================
// Edge cases
// ========================================================================

#[test]
fn test_parse_branch_case_insensitive() {
    let sql = "create BRANCH 'test' from 'MAIN'";
    let result = parse_branch(sql).unwrap().unwrap();
    if let BranchStatement::Create(stmt) = result {
        assert_eq!(stmt.name, "test");
        assert_eq!(stmt.from_branch, Some("MAIN".to_string()));
    } else {
        panic!("Expected Create statement");
    }
}

#[test]
fn test_parse_branch_with_semicolon() {
    let sql = "CREATE BRANCH 'feature/x' FROM 'main';";
    let result = parse_branch(sql);
    assert!(result.is_ok());
    assert!(result.unwrap().is_some());
}

#[test]
fn test_parse_non_branch_statement() {
    let sql = "SELECT * FROM nodes";
    let result = parse_branch(sql).unwrap();
    assert!(result.is_none());
}
