//! Plan Phase 13f: the BUILT-IN `(__parent_path, __created_at)` workspace
//! index, on every workspace unless its config opts out, built and dropped by
//! the automatic `compound_builds` repair (its format rebuild:
//! `compound_format_tests`).

use super::compound_env::{by_cat, item, ITEM};
use super::env::{node, repair_options, Env, REPO, TENANT, WS};
use super::node_types::register_type_with;
use super::replica_tests::{applicator, upsert};
use raisin_context::ResolutionType;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use raisin_models::workspace::builtin_indexes::{
    children_by_created_at_stored_name, CHILDREN_BY_CREATED_AT,
};
use raisin_models::workspace::{BuiltinIndexes, Workspace};
use raisin_rocksdb::compound_state::{read_state, CompoundStateStore};
use raisin_rocksdb::indexing::compound::{keyspace, workspace_defs};
use raisin_rocksdb::management::async_indexing::repair::{run_repair, RepairKind, RepairReport};
use raisin_storage::compound::CompoundStateSource;
use raisin_storage::{
    CompoundColumnValue, CompoundIndexRepository, NodeRepository, RepoScope, Storage,
    WorkspaceRepository,
};

pub(super) fn at(secs: i64) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::from_timestamp(1_700_000_000 + secs, 0).unwrap()
}

/// `node(..)` with a fixed `created_at`.
pub(super) fn born(id: &str, path: &str, secs: i64) -> Node {
    let mut n = node(id, path, &[]);
    n.created_at = Some(at(secs));
    n
}

impl Env {
    /// Turn the built-in index on or off through the workspace API.
    pub(super) async fn builtin(&self, on: bool) -> Result<()> {
        let mut ws: Workspace = self
            .storage
            .workspaces()
            .get(RepoScope::new(TENANT, REPO), WS)
            .await?
            .expect("workspace");
        ws.config.builtin_indexes = Some(BuiltinIndexes {
            children_by_created_at: on,
        });
        self.storage
            .workspaces()
            .put(RepoScope::new(TENANT, REPO), ws)
            .await
    }

    /// The children of `parent` the built-in index lists, newest first.
    pub(super) async fn children(&self, branch: &str, parent: &str) -> Result<Vec<String>> {
        Ok(self
            .storage
            .compound_index()
            .scan_compound_index(
                self.scope(branch),
                &children_by_created_at_stored_name(),
                &[CompoundColumnValue::String(parent.to_string())],
                false,
                true,
                None,
                None,
            )
            .await?
            .into_iter()
            .map(|entry| entry.node_id)
            .collect())
    }

    pub(super) fn builtin_ready(&self, branch: &str) -> bool {
        let declared = workspace_defs::current(self.storage.db(), TENANT, REPO, WS).unwrap();
        let Some(def) = declared
            .iter()
            .find(|d| d.name == children_by_created_at_stored_name())
        else {
            return false;
        };
        CompoundStateStore::new(self.storage.db().clone())
            .compound_availability(TENANT, REPO, branch, WS, def)
            .is_ready()
    }

    pub(super) fn builtin_entries(&self, branch: &str) -> u64 {
        keyspace::count(
            self.storage.db(),
            TENANT,
            REPO,
            branch,
            WS,
            &children_by_created_at_stored_name(),
        )
        .unwrap()
    }

    /// The `compound_builds` link on `branch` — what its job runs.
    pub(super) async fn compound_builds(&self, branch: &str) -> Result<RepairReport> {
        let mut reports = run_repair(
            &self.storage,
            TENANT,
            REPO,
            Some(branch),
            RepairKind::CompoundBuilds,
            repair_options(),
        )
        .await?;
        Ok(reports.remove(0))
    }

    pub(super) async fn pending_compound_builds(&self) -> Result<Vec<String>> {
        Ok(
            raisin_rocksdb::management::async_indexing::repair::pending_compound_build_branches(
                &self.storage,
            )?
            .into_iter()
            .filter(|(t, r, _)| t == TENANT && r == REPO)
            .map(|(_, _, b)| b)
            .collect(),
        )
    }
}

/// On by default: every write maintains it, the automatic link builds it
/// (`Ready`), and it lists a folder's children of every type newest first,
/// through moves into and out of the folder and deletes.
#[tokio::test]
async fn builtin_index_is_on_by_default_and_built_automatically() -> Result<()> {
    let env = Env::new(false).await?;
    register_type_with(&env.storage, "main", ITEM, None, None, Some(vec![by_cat()])).await?;
    env.add("main", born("f", "/f", 0)).await?;
    env.add("main", born("g", "/g", 1)).await?;
    env.add("main", born("c1", "/f/c1", 10)).await?;
    let mut typed = item("c2", "a", &[]);
    typed.path = "/f/c2".to_string();
    typed.parent = Some("f".to_string());
    typed.created_at = Some(at(20));
    env.add("main", typed).await?;
    env.add("main", born("c3", "/f/c3", 30)).await?;

    assert!(!env.builtin_ready("main"), "declared is not built");
    assert_eq!(env.pending_compound_builds().await?, ["main"]);
    let report = env.compound_builds("main").await?;
    assert!(report.completed);
    assert_eq!(report.compound.built, 1, "the built-in");
    assert!(env.builtin_ready("main"));
    assert!(env.pending_compound_builds().await?.is_empty());
    assert_eq!(env.children("main", "/f").await?, ["c3", "c2", "c1"]);
    assert_eq!(env.children("main", "/").await?, ["g", "f"]);

    // A later write keeps it Ready and listed; moves in and out; deletes.
    env.add("main", born("c4", "/f/c4", 40)).await?;
    env.storage
        .nodes()
        .move_node(env.scope("main"), "c2", "/g/c2", None)
        .await?;
    env.add("main", born("h1", "/g/h1", 5)).await?;
    env.storage
        .nodes()
        .move_node(env.scope("main"), "h1", "/f/h1", None)
        .await?;
    env.storage
        .nodes()
        .delete(env.scope("main"), "c1", Default::default())
        .await?;
    assert!(env.builtin_ready("main"), "every write maintained it");
    assert_eq!(env.children("main", "/f").await?, ["c4", "c3", "h1"]);
    assert_eq!(env.children("main", "/g").await?, ["c2"]);
    Ok(())
}

/// A workspace that opts out never gets the index: no entries are written
/// and nothing is owed.
#[tokio::test]
async fn an_opted_out_workspace_writes_no_entries() -> Result<()> {
    let env = Env::new(false).await?;
    env.builtin(false).await?;
    env.add("main", born("f", "/f", 0)).await?;
    env.add("main", born("c1", "/f/c1", 10)).await?;
    env.put("main", born("c1", "/f/c1", 10)).await?;
    let report = env.compound_builds("main").await?;
    assert_eq!(report.compound.built, 0);
    assert_eq!(env.builtin_entries("main"), 0);
    assert!(!env.builtin_ready("main"));
    assert!(env.pending_compound_builds().await?.is_empty());
    Ok(())
}

/// Off: the index fails closed at once and the link drops its record and
/// entries. On again: owed, built, Ready, and complete (writes made while it
/// was off are in it).
#[tokio::test]
async fn switching_off_drops_it_and_on_rebuilds_it() -> Result<()> {
    let env = Env::new(false).await?;
    env.add("main", born("f", "/f", 0)).await?;
    env.add("main", born("c1", "/f/c1", 10)).await?;
    env.compound_builds("main").await?;
    assert!(env.builtin_ready("main") && env.builtin_entries("main") > 0);

    env.builtin(false).await?;
    assert!(!env.builtin_ready("main"));
    env.add("main", born("c2", "/f/c2", 20)).await?;
    assert_eq!(
        env.pending_compound_builds().await?,
        ["main"],
        "a drop is owed"
    );
    let report = env.compound_builds("main").await?;
    assert_eq!(report.compound.dropped, 1);
    assert!(report.compound.dropped_entries > 0);
    assert_eq!(env.builtin_entries("main"), 0);
    let name = children_by_created_at_stored_name();
    assert!(read_state(env.storage.db(), TENANT, REPO, "main", WS, &name)?.is_none());
    assert!(env.pending_compound_builds().await?.is_empty());

    env.builtin(true).await?;
    assert!(!env.builtin_ready("main"));
    assert_eq!(env.pending_compound_builds().await?, ["main"]);
    assert_eq!(env.compound_builds("main").await?.compound.built, 1);
    assert!(env.builtin_ready("main"));
    assert_eq!(env.children("main", "/f").await?, ["c2", "c1"]);
    Ok(())
}

/// A fork inherits it `Ready` with the entries; a merge resolution and a
/// replicated upsert maintain it like any workspace index.
#[tokio::test]
async fn fork_replica_and_merge_maintain_the_builtin_index() -> Result<()> {
    let titled = |title: &str| {
        let mut n = node("x", "/x", &[("title", title)]);
        n.created_at = Some(at(10));
        n
    };
    let env = Env::new(false).await?;
    env.add("main", titled("first")).await?;
    env.compound_builds("main").await?;
    env.fork("feature").await?;
    assert!(env.builtin_ready("feature"), "a fork inherits Ready");
    assert_eq!(env.children("feature", "/").await?, ["x"]);

    // Merge resolution.
    env.put("main", titled("main")).await?;
    env.put("feature", titled("feature")).await?;
    env.conflict_and_resolve("feature", "x", ResolutionType::KeepTheirs)
        .await?;
    assert!(env.builtin_ready("main"), "the merge kept it Ready");
    assert_eq!(env.children("main", "/").await?, ["x"]);

    // Replica: the production receive path.
    env.add("main", born("w", "/w", 5)).await?; // warms test:Page
    let mut r = born("r", "/r", 50);
    r.workspace = Some(WS.to_string());
    let now = chrono::Utc::now().timestamp_millis() as u64;
    upsert(&applicator(&env), &r, "a1", HLC::new(now + 1_000, 0)).await;
    assert!(
        env.builtin_ready("main"),
        "a replicated write kept it Ready"
    );
    assert_eq!(env.children("main", "/").await?, ["r", "x", "w"]);
    Ok(())
}

/// A user declaration may not take the built-in's reserved name.
#[tokio::test]
async fn a_reserved_name_is_refused_at_workspace_write() -> Result<()> {
    let env = Env::new(false).await?;
    let mut ws: Workspace = env
        .storage
        .workspaces()
        .get(RepoScope::new(TENANT, REPO), WS)
        .await?
        .expect("workspace");
    let mut fake = by_cat();
    fake.name = CHILDREN_BY_CREATED_AT.to_string();
    ws.compound_indexes = Some(vec![fake]);
    let refused = env
        .storage
        .workspaces()
        .put(RepoScope::new(TENANT, REPO), ws)
        .await;
    assert!(refused.is_err(), "a user declared a reserved name");
    Ok(())
}
