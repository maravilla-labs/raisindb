//! The node-development surface against real RocksDB: a tree move keeps ids
//! and references, a stale revision is a conflict, a changeset is one
//! reviewable unit, and a replayed op does not apply twice.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use raisin_context::RepositoryConfig;
use raisin_core::services::node_dev::read::ReadResult;
use raisin_core::services::node_dev::root::Roots;
use raisin_core::services::node_dev::{
    ChangeOp, ChangesetRequest, ChangesetStatus, CommitOutcome, DevScope, ExpectedRevision,
    NodeDevService, OpAction, OpKind, Receipt, Target, WorkRoot,
};
use raisin_core::services::workspace_service::WorkspaceService;
use raisin_error::Result;
use raisin_events::{Event, EventHandler, NodeEvent, NodeEventKind};
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::{PropertyValue, RaisinReference};
use raisin_models::nodes::types::node_type::NodeType;
use raisin_models::workspace::Workspace;
use raisin_rocksdb::{RocksDBConfig, RocksDBStorage};
use raisin_storage::scope::{BranchScope, StorageScope};
use raisin_storage::{
    BranchRepository, CommitMetadata, NodeRepository, NodeTypeRepository, ReferenceIndexRepository,
    RegistryRepository, RepositoryManagementRepository, Storage,
};
use serde_json::{json, Map, Value};
use tempfile::TempDir;

const TENANT: &str = "nd";
const REPO: &str = "repo";
const WS: &str = "content";

struct Recorder(Arc<Mutex<Vec<NodeEvent>>>);

impl EventHandler for Recorder {
    fn name(&self) -> &str {
        "node_dev_recorder"
    }
    fn handle<'a>(
        &'a self,
        e: &'a Event,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            if let Event::Node(n) = e {
                self.0.lock().unwrap().push(n.clone());
            }
            Ok(())
        })
    }
}

fn node_type(name: &str) -> NodeType {
    NodeType {
        id: Some(name.to_string()),
        strict: Some(false),
        name: name.to_string(),
        extends: None,
        mixins: Vec::new(),
        overrides: None,
        description: None,
        icon: None,
        version: Some(1),
        properties: None,
        allowed_children: vec!["*".to_string()],
        required_nodes: Vec::new(),
        initial_structure: None,
        versionable: Some(true),
        immutable: None,
        publishable: Some(true),
        auditable: Some(false),
        indexable: Some(true),
        index_types: None,
        created_at: Some(chrono::Utc::now()),
        updated_at: None,
        published_at: None,
        published_by: None,
        previous_version: None,
        compound_indexes: None,
        is_mixin: None,
    }
}

async fn setup(dir: &TempDir) -> Result<Arc<RocksDBStorage>> {
    let mut config = RocksDBConfig::default();
    config.path = dir.path().to_path_buf();
    let storage = Arc::new(RocksDBStorage::with_config(config)?);
    storage
        .registry()
        .register_tenant(TENANT, HashMap::new())
        .await?;
    let repo_config = RepositoryConfig {
        default_language: "en".to_string(),
        supported_languages: vec!["en".to_string()],
        locale_fallback_chains: HashMap::new(),
        default_branch: "main".to_string(),
        description: None,
        tags: HashMap::new(),
    };
    storage
        .repository_management()
        .create_repository(TENANT, REPO, repo_config)
        .await?;
    storage
        .branches()
        .create_branch(TENANT, REPO, "main", "system", None, None, false, false)
        .await?;
    for t in ["raisin:Folder", "raisin:Node"] {
        storage
            .node_types()
            .upsert(
                BranchScope::new(TENANT, REPO, "main"),
                node_type(t),
                CommitMetadata::system("seed"),
            )
            .await?;
    }
    for ws in [WS, "raisin:system"] {
        let mut w = Workspace::new(ws.to_string());
        w.config.default_branch = "main".to_string();
        WorkspaceService::new(storage.clone())
            .put(TENANT, REPO, w)
            .await?;
    }
    Ok(storage)
}

fn scope() -> DevScope {
    DevScope::new(TENANT, REPO, "main")
}

fn roots() -> Vec<WorkRoot> {
    vec![WorkRoot::workspace(WS)]
}

fn create(path: &str, props: Value) -> ChangeOp {
    let properties: Map<String, Value> = props.as_object().cloned().unwrap_or_default();
    ChangeOp::Create {
        workspace: None,
        path: path.to_string(),
        node_type: "raisin:Folder".to_string(),
        archetype: None,
        properties,
    }
}

fn req(ops: Vec<ChangeOp>, key: Option<&str>) -> ChangesetRequest {
    ChangesetRequest {
        roots: roots(),
        ops,
        idempotency_key: key.map(str::to_string),
        ..ChangesetRequest::default()
    }
}

fn committed(o: CommitOutcome) -> Receipt {
    match o {
        CommitOutcome::Committed { receipt } => receipt,
        CommitOutcome::Conflict { conflicts, .. } => panic!("unexpected conflict: {conflicts:?}"),
    }
}

async fn read(svc: &NodeDevService<RocksDBStorage>, path: &str) -> ReadResult {
    let roots = Roots::new(roots()).unwrap();
    svc.read(
        &scope(),
        &AuthContext::system(),
        &roots,
        &Target::path(path),
        None,
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn tree_move_preserves_ids_and_references_and_emits_move_events() -> Result<()> {
    let dir = TempDir::new().unwrap();
    let storage = setup(&dir).await?;
    let events = Arc::new(Mutex::new(Vec::new()));
    storage
        .event_bus()
        .subscribe(Arc::new(Recorder(events.clone())));
    let svc = NodeDevService::new(storage.clone());
    let auth = AuthContext::system();

    committed(
        svc.apply(
            &scope(),
            &auth,
            req(
                vec![
                    create("/src", json!({})),
                    create("/src/a", json!({})),
                    create("/src/a/b", json!({"title": "leaf"})),
                    create("/dst", json!({})),
                ],
                None,
            ),
        )
        .await
        .unwrap(),
    );
    let a = read(&svc, "/src/a").await;
    let b = read(&svc, "/src/a/b").await;
    let b_id = b.locator.node_id.clone().unwrap();
    let link = serde_json::to_value(PropertyValue::Reference(RaisinReference {
        id: b_id.clone(),
        workspace: WS.to_string(),
        path: "/src/a/b".to_string(),
    }))
    .unwrap();
    committed(
        svc.apply(
            &scope(),
            &auth,
            req(vec![create("/page", json!({ "link": link }))], None),
        )
        .await
        .unwrap(),
    );
    let page_id = read(&svc, "/page").await.locator.node_id.unwrap();
    events.lock().unwrap().clear();

    let receipt = committed(
        svc.apply(
            &scope(),
            &auth,
            req(
                vec![ChangeOp::Move {
                    target: Target::path("/src/a"),
                    to_parent: Target::path("/dst"),
                    new_name: None,
                    expected_revision: Some(ExpectedRevision::Value(
                        a.locator.revision.clone().unwrap().value,
                    )),
                }],
                Some("move-1"),
            ),
        )
        .await
        .unwrap(),
    );
    let op = &receipt.ops[0];
    assert_eq!(op.action, OpAction::Moved);
    assert_eq!(op.old.as_ref().unwrap().path, "/src/a");
    assert_eq!(op.new.as_ref().unwrap().path, "/dst/a");
    assert_eq!(
        op.new.as_ref().unwrap().node_id,
        a.locator.node_id,
        "the id survives the move"
    );
    assert_eq!(op.moved_descendants.len(), 1);
    assert_eq!(op.moved_descendants[0].from_path, "/src/a/b");
    assert_eq!(op.moved_descendants[0].to.path, "/dst/a/b");
    assert_eq!(
        op.moved_descendants[0].to.node_id.as_deref(),
        Some(b_id.as_str())
    );
    assert!(
        op.rewritten_references
            .iter()
            .any(|r| r.node_id == page_id && r.target_id == b_id),
        "the receipt names the referrer: {:?}",
        op.rewritten_references
    );

    // Storage agrees: same ids at the new paths, the reference still resolves by id.
    let s = StorageScope::new(TENANT, REPO, "main", WS);
    let moved_b = storage.nodes().get(s, &b_id, None).await?.unwrap();
    assert_eq!(moved_b.path, "/dst/a/b");
    assert!(storage
        .nodes()
        .get_by_path(s, "/src/a", None)
        .await?
        .is_none());
    let referrers = storage
        .reference_index()
        .find_referencing_nodes(s, WS, &b_id, false)
        .await?;
    assert_eq!(referrers, vec![(page_id.clone(), "link".to_string())]);

    // One commit, and every moved node's event says MOVE, not delete+create.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let evs = events.lock().unwrap().clone();
    assert!(
        evs.iter().all(|e| e.kind != NodeEventKind::Deleted),
        "no delete events for a move"
    );
    let moved_flag = |id: &str| {
        evs.iter().any(|e| {
            e.node_id == id
                && e.kind == NodeEventKind::Updated
                && e.metadata
                    .as_ref()
                    .and_then(|m| m.get("moved"))
                    .and_then(Value::as_bool)
                    == Some(true)
        })
    };
    assert!(
        moved_flag(a.locator.node_id.as_deref().unwrap()),
        "root move event: {evs:?}"
    );
    assert!(moved_flag(&b_id), "descendant move event");
    Ok(())
}

#[tokio::test]
async fn stale_revision_is_a_conflict_and_writes_nothing() -> Result<()> {
    let dir = TempDir::new().unwrap();
    let storage = setup(&dir).await?;
    let svc = NodeDevService::new(storage.clone());
    let auth = AuthContext::system();
    committed(
        svc.apply(
            &scope(),
            &auth,
            req(vec![create("/x", json!({"title": "v1"}))], None),
        )
        .await
        .unwrap(),
    );
    let seen = read(&svc, "/x").await.locator.revision.unwrap().value;

    let patch = |title: &str, rev: &str| ChangeOp::Patch {
        target: Target::path("/x"),
        expected_revision: Some(ExpectedRevision::Value(rev.to_string())),
        set: json!({ "title": title }).as_object().cloned().unwrap(),
        unset: vec![],
        archetype: None,
    };
    committed(
        svc.apply(&scope(), &auth, req(vec![patch("v2", &seen)], None))
            .await
            .unwrap(),
    );

    // A second writer still holding the old revision.
    match svc
        .apply(&scope(), &auth, req(vec![patch("v3", &seen)], None))
        .await
        .unwrap()
    {
        CommitOutcome::Conflict { conflicts, .. } => {
            assert_eq!(conflicts[0].code, "stale_revision");
            assert_eq!(conflicts[0].expected.as_deref(), Some(seen.as_str()));
            assert!(conflicts[0].actual.is_some());
        }
        other => panic!("expected a conflict, got {other:?}"),
    }
    assert_eq!(
        read(&svc, "/x").await.properties["title"],
        json!("v2"),
        "the stale write did not land"
    );
    Ok(())
}

#[tokio::test]
async fn changeset_is_one_reviewable_unit() -> Result<()> {
    let dir = TempDir::new().unwrap();
    let storage = setup(&dir).await?;
    let svc = NodeDevService::new(storage.clone());
    let auth = AuthContext::system();
    committed(
        svc.apply(
            &scope(),
            &auth,
            req(
                vec![
                    create("/src", json!({})),
                    create("/src/a", json!({})),
                    create("/dst", json!({})),
                ],
                None,
            ),
        )
        .await
        .unwrap(),
    );

    let (rec, created) = svc
        .propose(
            &scope(),
            &auth,
            req(
                vec![
                    create("/dst/new", json!({"title": "n"})),
                    create("/dst/new/child", json!({})),
                    ChangeOp::Move {
                        target: Target::path("/src/a"),
                        to_parent: Target::path("/dst/new"),
                        new_name: None,
                        expected_revision: None,
                    },
                    ChangeOp::Patch {
                        target: Target::path("/dst/new/a"),
                        expected_revision: None,
                        set: json!({"title": "moved"}).as_object().cloned().unwrap(),
                        unset: vec![],
                        archetype: None,
                    },
                ],
                Some("review-1"),
            ),
        )
        .await
        .unwrap();
    assert!(created);
    assert_eq!(rec.status, ChangesetStatus::Proposed);
    assert_eq!(
        rec.plan.ops.len(),
        4,
        "every op resolved in one plan: {:?}",
        rec.plan.conflicts
    );
    assert!(rec.plan.conflicts.is_empty());
    assert!(rec.plan.digest.starts_with("sha256:"));

    // Reviewable, and nothing written yet.
    let again = svc
        .get_changeset(&scope(), &auth, &rec.changeset_id)
        .await
        .unwrap();
    assert_eq!(again.plan.digest, rec.plan.digest);
    let s = StorageScope::new(TENANT, REPO, "main", WS);
    assert!(storage
        .nodes()
        .get_by_path(s, "/dst/new", None)
        .await?
        .is_none());

    // An approval for another digest cannot commit it.
    match svc
        .commit(&scope(), &auth, &rec.changeset_id, Some("sha256:other"))
        .await
        .unwrap()
    {
        CommitOutcome::Conflict { conflicts, .. } => {
            assert_eq!(conflicts[0].code, "digest_mismatch")
        }
        other => panic!("expected digest mismatch, got {other:?}"),
    }

    let receipt = committed(
        svc.commit(&scope(), &auth, &rec.changeset_id, Some(&rec.plan.digest))
            .await
            .unwrap(),
    );
    assert_eq!(receipt.ops.len(), 4);
    assert!(receipt.committed_revision.is_some());
    assert_eq!(
        read(&svc, "/dst/new/a").await.properties["title"],
        json!("moved")
    );
    let done = svc
        .get_changeset(&scope(), &auth, &rec.changeset_id)
        .await
        .unwrap();
    assert_eq!(done.status, ChangesetStatus::Committed);
    assert_eq!(done.receipt.unwrap().ops.len(), 4);
    Ok(())
}

#[tokio::test]
async fn replayed_op_does_not_duplicate() -> Result<()> {
    let dir = TempDir::new().unwrap();
    let storage = setup(&dir).await?;
    let svc = NodeDevService::new(storage.clone());
    let auth = AuthContext::system();
    committed(
        svc.apply(&scope(), &auth, req(vec![create("/p", json!({}))], None))
            .await
            .unwrap(),
    );

    let op = || {
        req(
            vec![create("/p/fn", json!({"title": "f"}))],
            Some("run-1:op-7"),
        )
    };
    let first = committed(svc.apply(&scope(), &auth, op()).await.unwrap());
    let second = committed(svc.apply(&scope(), &auth, op()).await.unwrap());
    assert!(!first.replayed);
    assert!(second.replayed, "the replay answers from the record");
    assert_eq!(
        first.ops[0].new.as_ref().unwrap().node_id,
        second.ops[0].new.as_ref().unwrap().node_id
    );
    let children = storage
        .nodes()
        .list_children(
            StorageScope::new(TENANT, REPO, "main", WS),
            "/p",
            raisin_storage::ListOptions::for_api(),
        )
        .await?;
    assert_eq!(
        children.len(),
        1,
        "exactly one node, however often the op is replayed"
    );

    // The same key with different ops is refused, not silently merged.
    let err = svc
        .apply(
            &scope(),
            &auth,
            req(vec![create("/p/other", json!({}))], Some("run-1:op-7")),
        )
        .await
        .unwrap_err();
    assert_eq!(err.code, "idempotency_key_reused");
    Ok(())
}

#[tokio::test]
async fn roots_bound_every_path_and_grant() -> Result<()> {
    let dir = TempDir::new().unwrap();
    let storage = setup(&dir).await?;
    let svc = NodeDevService::new(storage.clone());
    let auth = AuthContext::system();
    committed(
        svc.apply(
            &scope(),
            &auth,
            req(
                vec![create("/build", json!({})), create("/other", json!({}))],
                None,
            ),
        )
        .await
        .unwrap(),
    );

    let scoped = |ops: Vec<OpKind>, op: ChangeOp| ChangesetRequest {
        roots: vec![WorkRoot {
            workspace: WS.into(),
            path: "/build".into(),
            ops: Some(ops),
        }],
        ops: vec![op],
        ..ChangesetRequest::default()
    };
    let escape = svc
        .apply(
            &scope(),
            &auth,
            scoped(OpKind::ALL.to_vec(), create("../other/x", json!({}))),
        )
        .await
        .unwrap_err();
    assert_eq!(escape.code, "path_escape");
    let outside = svc
        .apply(
            &scope(),
            &auth,
            scoped(
                OpKind::ALL.to_vec(),
                ChangeOp::Delete {
                    target: Target::path("/other"),
                    expected_revision: None,
                    recursive: true,
                },
            ),
        )
        .await
        .unwrap_err();
    assert_eq!(outside.code, "path_escape");
    let not_granted = svc
        .apply(
            &scope(),
            &auth,
            scoped(vec![OpKind::Read], create("x", json!({}))),
        )
        .await
        .unwrap_err();
    assert_eq!(not_granted.code, "forbidden");
    // Inside the root with the grant: fine, and relative to the root.
    let ok = committed(
        svc.apply(
            &scope(),
            &auth,
            scoped(vec![OpKind::Create], create("x", json!({}))),
        )
        .await
        .unwrap(),
    );
    assert_eq!(ok.ops[0].new.as_ref().unwrap().path, "/build/x");
    Ok(())
}

#[tokio::test]
async fn tool_calls_get_envelopes_and_replays_are_idempotent() -> Result<()> {
    use raisin_core::services::node_dev::dispatch;
    let dir = TempDir::new().unwrap();
    let storage = setup(&dir).await?;
    let svc = NodeDevService::new(storage.clone());
    let auth = AuthContext::system();
    committed(
        svc.apply(&scope(), &auth, req(vec![create("/apps", json!({}))], None))
            .await
            .unwrap(),
    );

    // What a tool receives: the model's args plus the runtime's context.
    let args = json!({
        "roots": [{ "workspace": WS, "path": "/" }],
        "ops": [{ "op": "create", "path": "/apps/board", "node_type": "raisin:Folder", "properties": { "title": "Board" } }],
        "__raisin_context": { "run_id": "run-1", "operation_id": "op-9" }
    });
    // The run's grant: only /apps, only create + read.
    let grant = vec![WorkRoot {
        workspace: WS.into(),
        path: "/apps".into(),
        ops: Some(vec![OpKind::Read, OpKind::Create]),
    }];
    let first = dispatch::call(&svc, &scope(), &auth, "apply", args.clone(), Some(&grant))
        .await
        .unwrap();
    // The asked root "/" is wider than the grant: refused, as an envelope.
    assert_eq!(first["status"], json!("failed"), "{first}");
    assert_eq!(first["diagnostics"][0]["code"], json!("forbidden"));

    let mut args = args;
    args["roots"] = json!([{ "workspace": WS, "path": "/apps" }]);
    let a = dispatch::call(&svc, &scope(), &auth, "apply", args.clone(), Some(&grant))
        .await
        .unwrap();
    let b = dispatch::call(&svc, &scope(), &auth, "apply", args.clone(), Some(&grant))
        .await
        .unwrap();
    for env in [&a, &b] {
        let parsed: raisin_core::services::node_dev::contract::ToolResultEnvelope =
            serde_json::from_value(env.clone()).unwrap();
        raisin_core::services::node_dev::contract::validate_tool_result(&parsed)
            .expect("valid envelope");
        assert_eq!(
            parsed.status,
            raisin_core::services::node_dev::contract::ToolStatus::Succeeded
        );
        assert_eq!(parsed.operation_id, "op-9");
    }
    assert_eq!(
        b["payload"]["replayed"],
        json!(true),
        "the replay is answered from the record"
    );
    assert_eq!(
        a["writes"][0]["locator"]["node_id"],
        b["writes"][0]["locator"]["node_id"]
    );
    let children = storage
        .nodes()
        .list_children(
            StorageScope::new(TENANT, REPO, "main", WS),
            "/apps",
            raisin_storage::ListOptions::for_api(),
        )
        .await?;
    assert_eq!(children.len(), 1);

    // A delete is not granted.
    let del = json!({
        "roots": [{ "workspace": WS, "path": "/apps" }],
        "ops": [{ "op": "delete", "target": { "path": "/apps/board" } }],
        "__raisin_context": { "run_id": "run-1", "operation_id": "op-10" }
    });
    let d = dispatch::call(&svc, &scope(), &auth, "apply", del, Some(&grant))
        .await
        .unwrap();
    assert_eq!(d["diagnostics"][0]["code"], json!("forbidden"));

    // A read in tool mode records what it saw.
    let r = dispatch::call(&svc, &scope(), &auth, "read", json!({ "target": "board", "roots": [{ "workspace": WS, "path": "/apps" }], "envelope": true }), Some(&grant)).await.unwrap();
    assert_eq!(r["reads"][0]["locator"]["path"], json!("/apps/board"));
    assert!(r["reads"][0]["revision"]["value"].is_string());
    Ok(())
}
