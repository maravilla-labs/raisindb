//! Plan Phase 13e: WORKSPACE-owned compound indexes. Declared on the workspace
//! record, they hold every node of the workspace whatever its type, through
//! the one writer (local, replicated, merge), the one build and the one
//! state machinery — and never share a keyspace with a NodeType index of the
//! same authored name.

use super::compound_env::{item, ITEM};
use super::env::{node, Env, REPO, TENANT, WS};
use super::node_types::register_type_with;
use super::replica_tests::{applicator, upsert};
use raisin_context::ResolutionType;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::properties::schema::{
    CompoundColumnType, CompoundIndexColumn, CompoundIndexDefinition,
};
use raisin_models::nodes::Node;
use raisin_models::workspace::Workspace;
use raisin_rocksdb::indexing::compound::{cold, workspace_defs};
use raisin_storage::compound::{CompoundBuildPhase, CompoundIndexState};
use raisin_storage::{
    CompoundColumnValue, CompoundIndexRepository, NodeRepository, RepoScope, Storage,
    WorkspaceRepository,
};

/// The stored (keyspace) name of the workspace's `by_cat`.
pub(super) const WS_BY_CAT: &str = "@by_cat";

/// The workspace's `by_cat (cat)`: the SAME authored name as `test:Item`'s
/// type-owned index, on purpose.
pub(super) fn ws_by_cat(extra: Option<&str>) -> CompoundIndexDefinition {
    let mut columns = vec![CompoundIndexColumn {
        property: "cat".to_string(),
        ascending: None,
        column_type: CompoundColumnType::String,
    }];
    if let Some(prop) = extra {
        columns.push(CompoundIndexColumn {
            property: prop.to_string(),
            ascending: None,
            column_type: CompoundColumnType::String,
        });
    }
    CompoundIndexDefinition {
        name: "by_cat".to_string(),
        columns,
        has_order_column: false,
        owner: None,
    }
}

/// A `test:Page` (no NodeType record) at `/{id}` with `cat`.
pub(super) fn page(id: &str, cat: &str) -> Node {
    node(id, &format!("/{id}"), &[("cat", cat)])
}

fn later(offset_ms: u64) -> HLC {
    HLC::new(chrono::Utc::now().timestamp_millis() as u64 + offset_ms, 0)
}

impl Env {
    async fn workspace_record(&self) -> Result<Workspace> {
        Ok(self
            .storage
            .workspaces()
            .get(RepoScope::new(TENANT, REPO), WS)
            .await?
            .expect("workspace"))
    }

    pub(super) async fn declare(&self, defs: Option<Vec<CompoundIndexDefinition>>) -> Result<()> {
        let mut ws = self.workspace_record().await?;
        ws.compound_indexes = defs;
        self.storage
            .workspaces()
            .put(RepoScope::new(TENANT, REPO), ws)
            .await
    }

    /// The node ids the WORKSPACE index lists under `cat`.
    pub(super) async fn ws_listed(
        &self,
        branch: &str,
        cat: &str,
        at: Option<&HLC>,
    ) -> Result<Vec<String>> {
        let mut ids: Vec<String> = self
            .storage
            .compound_index()
            .scan_compound_index(
                self.scope(branch),
                WS_BY_CAT,
                &[CompoundColumnValue::String(cat.to_string())],
                false,
                true,
                None,
                at,
            )
            .await?
            .into_iter()
            .map(|entry| entry.node_id)
            .collect();
        ids.sort();
        Ok(ids)
    }

    /// Whether the planner may serve the workspace index as declared NOW.
    pub(super) fn ws_ready(&self, branch: &str) -> bool {
        let declared =
            workspace_defs::current(self.storage.db(), TENANT, REPO, WS).expect("declarations");
        let Some(def) = declared.iter().find(|d| d.name == WS_BY_CAT) else {
            return false;
        };
        self.storage
            .compound_state()
            .expect("compound state source")
            .compound_availability(TENANT, REPO, branch, WS, def)
            .is_ready()
    }

    pub(super) fn ws_record(&self, branch: &str) -> Option<CompoundIndexState> {
        raisin_rocksdb::compound_state::read_state(
            self.storage.db(),
            TENANT,
            REPO,
            branch,
            WS,
            WS_BY_CAT,
        )
        .expect("state read")
    }
}

/// One keyspace for every type, and none shared with the NodeType's
/// same-named index; updates, deletes and history through the one writer.
#[tokio::test]
async fn workspace_index_covers_every_node_type() -> Result<()> {
    let env = Env::new(false).await?;
    env.with_items("main").await?; // test:Item with its own `by_cat`
    env.declare(Some(vec![ws_by_cat(None)])).await?;
    assert!(!env.ws_ready("main"), "declared is not built");
    env.build_compound("main").await?;
    assert!(env.ws_ready("main") && env.compound_ready("main"));

    env.add("main", item("i", "a", &[])).await?;
    env.add("main", page("p", "a")).await?;
    let r1 = env.newest_revision("main", "p");
    assert_eq!(env.ws_listed("main", "a", None).await?, ["i", "p"]);
    assert_eq!(
        env.compound("main", "a", None).await?,
        ["i"],
        "the type-owned `by_cat` holds only its type"
    );

    env.put("main", page("p", "b")).await?;
    assert_eq!(env.ws_listed("main", "a", None).await?, ["i"]);
    assert_eq!(env.ws_listed("main", "b", None).await?, ["p"]);
    assert_eq!(env.ws_listed("main", "a", Some(&r1)).await?, ["i", "p"]);

    env.storage
        .nodes()
        .delete(env.scope("main"), "i", Default::default())
        .await?;
    assert!(env.ws_listed("main", "a", None).await?.is_empty());
    assert!(env.ws_ready("main"), "every write maintained it");
    Ok(())
}

/// `@` names a workspace keyspace; a NodeType may not claim one.
#[tokio::test]
async fn a_node_type_may_not_claim_a_workspace_keyspace_name() -> Result<()> {
    let env = Env::new(false).await?;
    let mut def = ws_by_cat(None);
    def.name = WS_BY_CAT.to_string();
    let refused = register_type_with(&env.storage, "main", ITEM, None, None, Some(vec![def])).await;
    assert!(refused.is_err(), "a NodeType declared `{WS_BY_CAT}`");
    Ok(())
}

/// The production receive path (`ApplyRevision` through the applicator)
/// maintains the workspace index inline and leaves it `Ready`.
#[tokio::test]
async fn replica_maintains_the_workspace_index() -> Result<()> {
    let env = Env::new(false).await?;
    env.declare(Some(vec![ws_by_cat(None)])).await?;
    env.build_compound("main").await?;
    env.add("main", page("w", "warm")).await?; // caches `test:Page` (no record)
    let replica = applicator(&env);

    let (r1, r2) = (later(1_000), later(2_000));
    upsert(&replica, &page("x", "b"), "a1", r1).await;
    assert!(
        env.ws_ready("main"),
        "a warm replicated write marked NotBuilt"
    );
    assert_eq!(env.ws_listed("main", "b", None).await?, ["x"]);
    upsert(&replica, &page("x", "c"), "a1", r2).await;
    assert!(env.ws_ready("main"));
    assert!(env.ws_listed("main", "b", None).await?.is_empty());
    assert_eq!(env.ws_listed("main", "c", None).await?, ["x"]);
    assert_eq!(env.ws_listed("main", "b", Some(&r1)).await?, ["x"]);
    Ok(())
}

/// A merge resolution writes the workspace index's entries at M.
#[tokio::test]
async fn merge_resolution_writes_workspace_index_entries() -> Result<()> {
    let env = Env::new(false).await?;
    env.declare(Some(vec![ws_by_cat(None)])).await?;
    env.build_compound("main").await?;
    env.add("main", page("x", "a")).await?;
    env.fork("feature").await?;
    assert!(
        env.ws_ready("feature"),
        "a fork inherits Ready with the entries"
    );
    env.put("main", page("x", "b")).await?;
    env.put("feature", page("x", "c")).await?;
    env.conflict_and_resolve("feature", "x", ResolutionType::KeepTheirs)
        .await?;
    assert!(env.ws_ready("main"), "the merge kept the index Ready");
    assert_eq!(env.ws_listed("main", "c", None).await?, ["x"]);
    assert!(env.ws_listed("main", "a", None).await?.is_empty());
    assert!(env.ws_listed("main", "b", None).await?.is_empty());
    Ok(())
}

/// A fork of a branch whose workspace index was never built is not `Ready`
/// either, until it is built there.
#[tokio::test]
async fn fork_of_an_unbuilt_workspace_index_is_unusable_until_built() -> Result<()> {
    let env = Env::new(false).await?;
    env.declare(Some(vec![ws_by_cat(None)])).await?;
    env.add("main", page("x", "a")).await?;
    env.fork("feature").await?;
    assert!(!env.ws_ready("main") && !env.ws_ready("feature"));
    env.put("feature", page("y", "a")).await?;
    assert!(!env.ws_ready("feature"));
    env.build_compound("feature").await?;
    assert!(env.ws_ready("feature"));
    assert!(!env.ws_ready("main"), "built on the fork only");
    assert_eq!(env.ws_listed("feature", "a", None).await?, ["x", "y"]);
    Ok(())
}

/// A changed declaration fails the index closed (record `NotBuilt`, build
/// requested); the rebuild re-earns `Ready` under the new layout.
#[tokio::test]
async fn declaration_change_fails_closed_until_rebuilt() -> Result<()> {
    let env = Env::new(false).await?;
    env.declare(Some(vec![ws_by_cat(None)])).await?;
    env.build_compound("main").await?;
    env.add("main", node("x", "/x", &[("cat", "a"), ("tone", "warm")]))
        .await?;
    assert!(env.ws_ready("main"));

    env.declare(Some(vec![ws_by_cat(Some("tone"))])).await?;
    let record = env.ws_record("main").expect("record");
    assert_eq!(record.phase, CompoundBuildPhase::NotBuilt);
    assert!(!env.ws_ready("main"));
    assert!(cold::is_requested(
        env.storage.db(),
        TENANT,
        REPO,
        "main",
        WS
    ));

    env.build_compound("main").await?;
    assert!(env.ws_ready("main"));
    let entries = env
        .storage
        .compound_index()
        .scan_compound_index(
            env.scope("main"),
            WS_BY_CAT,
            &[
                CompoundColumnValue::String("a".to_string()),
                CompoundColumnValue::String("warm".to_string()),
            ],
            false,
            true,
            None,
            None,
        )
        .await?;
    assert_eq!(entries.len(), 1);
    Ok(())
}

/// Removing a declaration and bringing the same one back must not trust the
/// old record: writes in between were not indexed. Also when the record is
/// written by a path with no hook (here a raw write, as a backup import does)
/// and the process restarted in between — the next writer reconciles.
#[tokio::test]
async fn removal_and_return_never_trust_a_stale_record() -> Result<()> {
    let env = Env::new(false).await?;
    env.declare(Some(vec![ws_by_cat(None)])).await?;
    env.build_compound("main").await?;
    assert!(env.ws_ready("main"));

    // Raw record write (no repository hook), then a "restart".
    let mut ws = env.workspace_record().await?;
    ws.compound_indexes = None;
    let db = env.storage.db();
    let cf = db.cf_handle(raisin_rocksdb::cf::WORKSPACES).unwrap();
    db.put_cf(
        cf,
        raisin_rocksdb::keys::workspace_key(TENANT, REPO, WS),
        rmp_serde::to_vec_named(&ws).unwrap(),
    )
    .unwrap();
    workspace_defs::invalidate_all();
    // The next writer reads the declarations first, and reconciles.
    env.add("main", page("x", "a")).await?;
    assert_eq!(
        env.ws_record("main").expect("record").phase,
        CompoundBuildPhase::NotBuilt,
        "a removed declaration's record must not stay Ready"
    );

    env.declare(Some(vec![ws_by_cat(None)])).await?;
    assert!(
        !env.ws_ready("main"),
        "x was written while the index was undeclared"
    );
    env.build_compound("main").await?;
    assert_eq!(env.ws_listed("main", "a", None).await?, ["x"]);
    Ok(())
}

/// A REPLICATED workspace record carrying a changed declaration fails the
/// index closed on the receiving node.
#[tokio::test]
async fn replicated_declaration_change_fails_closed() -> Result<()> {
    let env = Env::new(false).await?;
    env.declare(Some(vec![ws_by_cat(None)])).await?;
    env.build_compound("main").await?;
    assert!(env.ws_ready("main"));

    let mut ws = env.workspace_record().await?;
    ws.compound_indexes = Some(vec![ws_by_cat(Some("tone"))]);
    ws.updated_at = Some(raisin_models::timestamp::StorageTimestamp::from(
        chrono::Utc::now() + chrono::Duration::hours(1),
    ));
    let op = raisin_replication::Operation {
        op_id: uuid::Uuid::new_v4(),
        op_seq: 1,
        cluster_node_id: "peer".to_string(),
        timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
        vector_clock: raisin_replication::VectorClock::new(),
        tenant_id: TENANT.to_string(),
        repo_id: REPO.to_string(),
        branch: "main".to_string(),
        op_type: raisin_replication::OpType::UpdateWorkspace {
            workspace_id: WS.to_string(),
            workspace: ws,
        },
        revision: None,
        actor: "peer".to_string(),
        agent: None,
        message: None,
        is_system: false,
        acknowledged_by: Default::default(),
    };
    applicator(&env).apply_operation(&op).await?;
    assert_eq!(
        env.ws_record("main").expect("record").phase,
        CompoundBuildPhase::NotBuilt
    );
    assert!(!env.ws_ready("main"));
    Ok(())
}
