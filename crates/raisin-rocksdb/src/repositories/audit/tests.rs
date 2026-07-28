//! Tests for the persistent audit-log repository.

use super::*;
use crate::open_db;
use raisin_audit::{AuditRepository, AuditScope};
use raisin_models::nodes::audit_log::{AuditLog, AuditLogAction};
use tempfile::TempDir;

fn setup() -> (TempDir, RocksDBAuditRepo) {
    let temp_dir = TempDir::new().unwrap();
    let db = Arc::new(open_db(temp_dir.path()).unwrap());
    let repo = RocksDBAuditRepo::new(db);
    (temp_dir, repo)
}

fn scope<'a>() -> AuditScope<'a> {
    AuditScope::new("tenant-1", "repo-1", "main", "content")
}

fn entry(node_id: &str, path: &str, action: AuditLogAction, offset_ms: i64) -> AuditLog {
    AuditLog {
        id: format!("log:{}:{}", node_id, offset_ms),
        node_id: node_id.to_string(),
        path: path.to_string(),
        workspace: "content".to_string(),
        user_id: Some("alice".to_string()),
        action,
        timestamp: chrono::DateTime::from_timestamp_millis(1_700_000_000_000 + offset_ms).unwrap(),
        details: None,
        agent: None,
    }
}

#[tokio::test]
async fn write_then_read_back_by_node_id() {
    let (_tmp, repo) = setup();

    let log = entry("node-a", "/pages/home", AuditLogAction::Create, 0);
    repo.write_log_scoped(scope(), log.clone()).await.unwrap();

    let read = repo.get_logs_scoped(scope(), "node-a", None).await.unwrap();
    assert_eq!(read.len(), 1);
    assert_eq!(read[0].id, log.id);
    assert_eq!(read[0].node_id, "node-a");
    assert_eq!(read[0].path, "/pages/home");
    assert_eq!(read[0].workspace, "content");
    assert_eq!(read[0].user_id.as_deref(), Some("alice"));
    assert_eq!(read[0].action, AuditLogAction::Create);
    assert_eq!(read[0].agent, None);
}

/// The by-path HTTP route resolves a path to a node id and then reads by id, so
/// "read by path" must work through whatever id that resolution yields — even
/// after the node has been moved and later entries carry a different path.
#[tokio::test]
async fn history_follows_the_node_id_across_a_path_change() {
    let (_tmp, repo) = setup();

    repo.write_log_scoped(scope(), entry("node-a", "/old", AuditLogAction::Create, 0))
        .await
        .unwrap();
    repo.write_log_scoped(scope(), entry("node-a", "/new", AuditLogAction::Update, 10))
        .await
        .unwrap();

    let read = repo.get_logs_scoped(scope(), "node-a", None).await.unwrap();
    assert_eq!(read.len(), 2, "both entries stay attached to the node id");
    let paths: Vec<&str> = read.iter().map(|l| l.path.as_str()).collect();
    assert!(paths.contains(&"/old"));
    assert!(paths.contains(&"/new"));
}

#[tokio::test]
async fn entries_are_returned_newest_first() {
    let (_tmp, repo) = setup();

    repo.write_log_scoped(scope(), entry("n", "/p", AuditLogAction::Create, 0))
        .await
        .unwrap();
    repo.write_log_scoped(scope(), entry("n", "/p", AuditLogAction::Update, 1_000))
        .await
        .unwrap();
    repo.write_log_scoped(scope(), entry("n", "/p", AuditLogAction::Delete, 2_000))
        .await
        .unwrap();

    let read = repo.get_logs_scoped(scope(), "n", None).await.unwrap();
    assert_eq!(read.len(), 3);
    assert_eq!(read[0].action, AuditLogAction::Delete);
    assert_eq!(read[1].action, AuditLogAction::Update);
    assert_eq!(read[2].action, AuditLogAction::Create);
}

/// `make_log` stamps ids at millisecond resolution, so same-millisecond writes
/// are expected. The seq tiebreaker must keep them as distinct keys rather than
/// letting the second overwrite the first.
#[tokio::test]
async fn same_millisecond_writes_do_not_overwrite_each_other() {
    let (_tmp, repo) = setup();

    for i in 0..5 {
        let mut log = entry("n", "/p", AuditLogAction::Update, 0);
        log.id = format!("log-{}", i);
        repo.write_log_scoped(scope(), log).await.unwrap();
    }

    let read = repo.get_logs_scoped(scope(), "n", None).await.unwrap();
    assert_eq!(read.len(), 5, "no entry may be lost to a key collision");
    // Same millisecond ⇒ ordered by descending seq, i.e. last written first.
    assert_eq!(read[0].id, "log-4");
    assert_eq!(read[4].id, "log-0");
}

#[tokio::test]
async fn reads_are_isolated_by_tenant_repo_branch_and_workspace() {
    let (_tmp, repo) = setup();

    repo.write_log_scoped(
        AuditScope::new("tenant-1", "repo-1", "main", "content"),
        entry("shared-id", "/p", AuditLogAction::Create, 0),
    )
    .await
    .unwrap();

    // Same node id in every other scope dimension must be invisible.
    for other in [
        AuditScope::new("tenant-2", "repo-1", "main", "content"),
        AuditScope::new("tenant-1", "repo-2", "main", "content"),
        AuditScope::new("tenant-1", "repo-1", "feature-x", "content"),
        AuditScope::new("tenant-1", "repo-1", "main", "assets"),
    ] {
        let read = repo.get_logs_scoped(other, "shared-id", None).await.unwrap();
        assert!(
            read.is_empty(),
            "scope {:?} must not see another scope's entries",
            other
        );
    }

    let read = repo
        .get_logs_scoped(
            AuditScope::new("tenant-1", "repo-1", "main", "content"),
            "shared-id",
            None,
        )
        .await
        .unwrap();
    assert_eq!(read.len(), 1);
}

#[tokio::test]
async fn other_nodes_entries_are_not_returned() {
    let (_tmp, repo) = setup();

    repo.write_log_scoped(scope(), entry("n", "/p", AuditLogAction::Create, 0))
        .await
        .unwrap();
    // "n-2" shares a textual prefix with "n"; the trailing separator in the scan
    // prefix is what keeps it out of the result.
    repo.write_log_scoped(scope(), entry("n-2", "/p2", AuditLogAction::Create, 0))
        .await
        .unwrap();

    let read = repo.get_logs_scoped(scope(), "n", None).await.unwrap();
    assert_eq!(read.len(), 1);
    assert_eq!(read[0].node_id, "n");
}

#[tokio::test]
async fn limit_caps_the_result_and_keeps_the_newest() {
    let (_tmp, repo) = setup();

    for i in 0..10 {
        repo.write_log_scoped(scope(), entry("n", "/p", AuditLogAction::Update, i * 100))
            .await
            .unwrap();
    }

    let read = repo.get_logs_scoped(scope(), "n", Some(3)).await.unwrap();
    assert_eq!(read.len(), 3);
    assert_eq!(read[0].id, "log:n:900", "newest entry first");
    assert_eq!(read[2].id, "log:n:700");

    assert!(repo
        .get_logs_scoped(scope(), "n", Some(0))
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn agent_attribution_round_trips() {
    let (_tmp, repo) = setup();

    let mut log = entry("n", "/p", AuditLogAction::Update, 0);
    log.agent = Some("mcp:studio-admin".to_string());
    repo.write_log_scoped(scope(), log).await.unwrap();

    let read = repo.get_logs_scoped(scope(), "n", None).await.unwrap();
    assert_eq!(read.len(), 1);
    assert_eq!(
        read[0].agent.as_deref(),
        Some("mcp:studio-admin"),
        "the agent marker must survive the msgpack round trip"
    );
    assert_eq!(
        read[0].user_id.as_deref(),
        Some("alice"),
        "the human actor is recorded alongside the agent, not replaced by it"
    );
}

/// Entries written before `AuditLog::agent` existed carry no `agent` key in
/// their msgpack map. Decoding one must succeed and yield `None`, not fail.
#[tokio::test]
async fn records_persisted_without_the_agent_field_still_decode() {
    #[derive(serde::Serialize)]
    struct LegacyAuditLog {
        id: String,
        node_id: String,
        path: String,
        workspace: String,
        user_id: Option<String>,
        action: AuditLogAction,
        timestamp: chrono::DateTime<chrono::Utc>,
        details: Option<String>,
    }

    let legacy = LegacyAuditLog {
        id: "log:legacy:1".to_string(),
        node_id: "n".to_string(),
        path: "/p".to_string(),
        workspace: "content".to_string(),
        user_id: Some("alice".to_string()),
        action: AuditLogAction::Create,
        timestamp: chrono::DateTime::from_timestamp_millis(1_700_000_000_000).unwrap(),
        details: None,
    };

    let bytes = rmp_serde::to_vec_named(&legacy).unwrap();
    let decoded: AuditLog = rmp_serde::from_slice(&bytes).unwrap();
    assert_eq!(decoded.id, "log:legacy:1");
    assert_eq!(decoded.agent, None);
}

/// The unscoped trait methods cannot be honoured by a scope-keyed store. They
/// must degrade safely (empty read) rather than panic or leak across tenants.
#[tokio::test]
async fn unscoped_read_returns_empty_rather_than_leaking() {
    let (_tmp, repo) = setup();

    repo.write_log_scoped(scope(), entry("n", "/p", AuditLogAction::Create, 0))
        .await
        .unwrap();

    assert!(repo.get_logs_by_node_id("n").await.unwrap().is_empty());
}

/// The one-way door check: a database written by a build that predates the
/// AUDIT_LOG column family must still open, and must gain a working audit CF.
///
/// This is not hypothetical — every existing deployment's data dir is in exactly
/// that state. It works because `to_rocksdb_options` sets
/// `create_missing_column_families(true)` unconditionally, so opening with the
/// full descriptor list creates the absent CF in place. Simulated here by
/// opening with the descriptor list minus AUDIT_LOG, closing, then opening
/// through the real production path.
#[tokio::test]
async fn a_database_without_the_audit_cf_still_opens_and_gains_it() {
    let temp_dir = TempDir::new().unwrap();

    // 1. An "old" database: every CF except the audit one.
    {
        let mut opts = rocksdb::Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);

        let old_cfs: Vec<rocksdb::ColumnFamilyDescriptor> = crate::all_column_families()
            .into_iter()
            .filter(|name| *name != crate::cf::AUDIT_LOG)
            .map(|name| rocksdb::ColumnFamilyDescriptor::new(name, rocksdb::Options::default()))
            .collect();

        let db = rocksdb::DB::open_cf_descriptors(&opts, temp_dir.path(), old_cfs)
            .expect("old-format database should be creatable");
        assert!(
            db.cf_handle(crate::cf::AUDIT_LOG).is_none(),
            "precondition: the old database must not have the audit CF"
        );
    }

    // 2. The new binary opens it through the normal path.
    let db = Arc::new(
        open_db(temp_dir.path()).expect("an existing database must still open after the CF is added"),
    );
    let repo = RocksDBAuditRepo::new(db);

    // 3. And the audit CF is live.
    repo.write_log_scoped(scope(), entry("n", "/p", AuditLogAction::Create, 0))
        .await
        .unwrap();
    assert_eq!(repo.get_logs_scoped(scope(), "n", None).await.unwrap().len(), 1);
}

#[tokio::test]
async fn entries_survive_reopening_the_database() {
    let temp_dir = TempDir::new().unwrap();

    {
        let db = Arc::new(open_db(temp_dir.path()).unwrap());
        let repo = RocksDBAuditRepo::new(db);
        repo.write_log_scoped(scope(), entry("n", "/p", AuditLogAction::Create, 0))
            .await
            .unwrap();
    }

    let db = Arc::new(open_db(temp_dir.path()).unwrap());
    let repo = RocksDBAuditRepo::new(db);
    let read = repo.get_logs_scoped(scope(), "n", None).await.unwrap();
    assert_eq!(read.len(), 1, "audit history must outlive the process");
}
