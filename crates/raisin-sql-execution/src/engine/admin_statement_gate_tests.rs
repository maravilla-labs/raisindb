// SPDX-License-Identifier: BSL-1.1

//! Administrative statements refuse callers who are not operators, and keep
//! working for the ones who are (the system: admin console, CLI, API keys,
//! functions; and `system_admin` users).

use super::QueryEngine;
use futures::StreamExt;
use raisin_error::Error;
use raisin_models::auth::AuthContext;
use raisin_models::permissions::ResolvedPermissions;
use raisin_storage_memory::InMemoryStorage;
use std::sync::Arc;

fn engine(auth: AuthContext) -> QueryEngine<InMemoryStorage> {
    QueryEngine::new(
        Arc::new(InMemoryStorage::default()),
        "t_gate",
        "repo",
        "main",
    )
    .with_auth(auth)
}

fn user() -> AuthContext {
    AuthContext::for_user("bob").with_permissions(ResolvedPermissions::empty("bob"))
}

/// What HTTP builds for a request without credentials.
fn anonymous() -> AuthContext {
    AuthContext::anonymous_user("anon-node")
        .with_permissions(ResolvedPermissions::anonymous(vec![]))
}

fn admin() -> AuthContext {
    AuthContext::for_user("root").with_permissions(ResolvedPermissions::system_admin())
}

async fn run(engine: &QueryEngine<InMemoryStorage>, sql: &str) -> Result<(), Error> {
    let mut stream = engine.execute(sql).await?;
    while let Some(row) = stream.next().await {
        row?;
    }
    Ok(())
}

fn forbidden(r: Result<(), Error>) -> bool {
    matches!(r, Err(Error::Forbidden(_)))
}

const BRANCH_MUTATIONS: &[&str] = &[
    "CREATE BRANCH 'gate-probe' FROM 'main'",
    "DROP BRANCH IF EXISTS 'gate-probe'",
    "DROP BRANCH 'main'",
    "ALTER BRANCH 'main' SET PROTECTED true",
    "MERGE BRANCH 'gate-probe' INTO 'main'",
];

const AI_MUTATIONS: &[&str] = &[
    "ALTER EMBEDDING CONFIG SET BASE_URL = 'https://attacker.example'",
    "TEST EMBEDDING CONNECTION",
    "REBUILD VECTOR INDEX",
    "REGENERATE EMBEDDINGS",
];

#[tokio::test]
async fn branch_and_ai_config_changes_need_an_operator() {
    for who in [user(), anonymous(), AuthContext::anonymous()] {
        let e = engine(who);
        for sql in BRANCH_MUTATIONS.iter().chain(AI_MUTATIONS) {
            assert!(forbidden(run(&e, sql).await), "{sql} must be refused");
        }
    }
}

#[tokio::test]
async fn operators_still_pass_the_gate() {
    for who in [AuthContext::system(), admin()] {
        let e = engine(who);
        for sql in BRANCH_MUTATIONS.iter().chain(AI_MUTATIONS) {
            // Past the gate the statement may still fail on its own terms
            // (no such branch, no HNSW engine here), but never as Forbidden.
            assert!(!forbidden(run(&e, sql).await), "{sql} must not be refused");
        }
        // And a plain one succeeds outright.
        run(&e, "DROP BRANCH IF EXISTS 'never-existed'")
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn reading_branches_stays_open() {
    for who in [user(), anonymous()] {
        assert!(!forbidden(run(&engine(who), "SHOW BRANCHES").await));
    }
}

#[tokio::test]
async fn anonymous_http_callers_cannot_list_roles() {
    // The HTTP anonymous context is a real user id with `is_anonymous` unset;
    // the ACL gate used to read only the flag.
    assert!(forbidden(run(&engine(anonymous()), "SHOW ROLES").await));
    assert!(!forbidden(run(&engine(user()), "SHOW ROLES").await));
}
