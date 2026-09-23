//! Child runs: waiting tools, messages, objective narrowing, idempotent spawn.

use raisin_agent_runtime::control::ActorRef;
use raisin_agent_runtime::events::{OpOutcome, RunEventKind};
use raisin_agent_runtime::lifecycle::{OperationResult, OperationSpec};
use raisin_agent_runtime::record::OperationKind;
use raisin_agent_runtime::service::ServiceError;
use raisin_agent_runtime::service_child_ctl::ChildAction;
use raisin_agent_runtime::state::{RunStatus, TerminalStatus};
use raisin_agent_runtime::store::AgentRunStore;
use serde_json::{json, Value};

use crate::children::{alice, finish_client, host, pump, spawn, spawn_req, succeeded};
use crate::common::*;

#[tokio::test]
async fn a_waiting_tool_is_answered_by_the_handback_whichever_finishes_first() {
    let h = Harness::manual();
    let host = host(&h);
    for child_first in [false, true] {
        let parent = h.create(&format!("/p-{child_first}")).await;
        let out = spawn(&h, &parent, spawn_req("c", json!({}))).await;
        if child_first {
            finish_client(
                &h,
                &out.child_run_id,
                TerminalStatus::Completed,
                succeeded(json!({})),
            )
            .await;
            pump(&h, &host).await;
        }
        let g = h
            .svc
            .acquire_lease(&h.scope, &parent, "client")
            .await
            .unwrap();
        let spec = OperationSpec {
            input: Some(json!({ "tool": "/lib/raisin/ai/delegate" })),
            ..Default::default()
        };
        let op = h
            .svc
            .begin_operation(&h.scope, &parent, &g.fence, spec)
            .await
            .unwrap()
            .op;
        let waiting = OperationResult {
            outcome: Some(OpOutcome::Waiting),
            resume_key: Some(out.resume_key.clone()),
            ..Default::default()
        };
        h.svc
            .finish_operation(&h.scope, &parent, &g.fence, &op.op_id, waiting)
            .await
            .unwrap();
        if !child_first {
            assert_eq!(h.status(&parent).await, RunStatus::Waiting);
            finish_client(
                &h,
                &out.child_run_id,
                TerminalStatus::Completed,
                succeeded(json!({})),
            )
            .await;
            pump(&h, &host).await;
        }
        assert_eq!(
            h.status(&parent).await,
            RunStatus::Queued,
            "child_first={child_first}"
        );
        let resolved = h
            .events(&parent)
            .await
            .into_iter()
            .find_map(|e| match e.kind {
                RunEventKind::RequestResolved { resolution, .. } => Some(resolution),
                _ => None,
            });
        let resolution = resolved.expect("resolved");
        assert_eq!(resolution["kind"], "external");
        let key = resolution["result_key"].as_str().unwrap();
        let env: Value = serde_json::from_slice(
            &h.store
                .read_result(&h.scope, &parent, key)
                .await
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(env["envelope"], "raisin.tool-result/1");
    }
}

#[tokio::test]
async fn parent_messages_and_steers_a_child_and_the_child_posts_back() {
    let h = Harness::manual();
    let parent = h.create("/p").await;
    let child = spawn(&h, &parent, spawn_req("c", json!({})))
        .await
        .child_run_id;
    let msg = ChildAction::Message {
        message: json!({ "text": "use the /local layer" }),
    };
    h.svc
        .control_child(&h.scope, &parent, &child, msg, "m1", &alice(), None)
        .await
        .unwrap();
    let steer = ChildAction::Steer {
        input: json!({ "text": "skip the board view" }),
    };
    h.svc
        .control_child(&h.scope, &parent, &child, steer, "s1", &alice(), None)
        .await
        .unwrap();
    let crec = h.rec(&child).await;
    assert_eq!(crec.steer_queue[0].input["type"], "parent_message");
    assert_eq!(crec.steer_queue[1].input["type"], "parent_steer");
    let logged = count(&h.events(&parent).await, |k| {
        matches!(k, RunEventKind::ChildControlled { .. })
    });
    assert_eq!(logged, 2);
    // Somebody else may not control the child through the parent.
    let bob = ActorRef::user("bob");
    let err = h
        .svc
        .control_child(
            &h.scope,
            &parent,
            &child,
            ChildAction::Resume,
            "r",
            &bob,
            None,
        )
        .await
        .unwrap_err();
    assert_eq!(err, ServiceError::Unauthorized);

    let n = h
        .svc
        .post_to_parent(
            &h.scope,
            &child,
            json!({ "question": "which workspace?" }),
            "q1",
            &alice(),
            None,
        )
        .await
        .unwrap();
    let again = h
        .svc
        .post_to_parent(
            &h.scope,
            &child,
            json!({ "question": "which workspace?" }),
            "q1",
            &alice(),
            None,
        )
        .await
        .unwrap();
    assert_eq!((n, again), (1, 1));
    let mb = h.svc.mailbox(&h.scope, &parent).await.unwrap();
    assert_eq!(mb.len(), 1);
    assert_eq!(mb[0]["item"]["kind"], "message");
    assert_eq!(mb[0]["payload"]["message"]["question"], "which workspace?");
    assert_eq!(
        h.svc
            .ack_mailbox(&h.scope, &parent, n, &alice(), None)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn objective_narrows_tools_and_writes() {
    let h = Harness::manual();
    let mut req = h.request("/p");
    req.executor_config = Some(
        json!({ "node_dev": { "roots": [{ "workspace": "functions", "path": "/lib/app", "ops": ["read", "create"] }] } }),
    );
    let parent = h.create_with(req).await;
    let inside = spawn_req(
        "w",
        json!({ "objective": {
        "allowed_tools": ["/lib/raisin/node-dev/*"],
        "allowed_writes": [{ "workspace": "functions", "path": "/lib/app/x" }],
    } }),
    );
    let child = spawn(&h, &parent, inside).await.child_run_id;
    let cfg = h.rec(&child).await.executor_config.unwrap();
    assert_eq!(
        cfg["node_dev"]["roots"][0]["ops"],
        json!(["read", "create"])
    );
    assert_eq!(cfg["allowed_tools"][0], "/lib/raisin/node-dev/*");
    let outside = spawn_req(
        "w2",
        json!({ "objective": { "allowed_writes": [{ "workspace": "functions", "path": "/lib/other" }] } }),
    );
    let err = h
        .svc
        .spawn_child(&h.scope, &parent, outside, &alice(), None)
        .await
        .unwrap_err();
    assert!(
        matches!(&err, ServiceError::Refused(r) if r.code == "write_scope_not_within_parent"),
        "{err:?}"
    );

    let g = h
        .svc
        .acquire_lease(&h.scope, &child, "client")
        .await
        .unwrap();
    let denied = OperationSpec {
        kind: Some(OperationKind::ToolCall),
        input: Some(json!({ "tool": "/lib/studio/delete-everything" })),
        ..Default::default()
    };
    let err = h
        .svc
        .begin_operation(&h.scope, &child, &g.fence, denied)
        .await
        .unwrap_err();
    assert!(matches!(err, ServiceError::Begin(_)), "{err:?}");
    let allowed = OperationSpec {
        kind: Some(OperationKind::ToolCall),
        input: Some(json!({ "tool": "/lib/raisin/node-dev/node-read" })),
        ..Default::default()
    };
    h.svc
        .begin_operation(&h.scope, &child, &g.fence, allowed)
        .await
        .unwrap();
}

#[tokio::test]
async fn spawn_is_idempotent_and_a_snapshot_references_a_parent_checkpoint() {
    let h = Harness::manual();
    let parent = h.create("/p").await;
    let req = spawn_req(
        "snap",
        json!({ "spawn_key": "k1", "objective": { "context": { "mode": "snapshot" } } }),
    );
    let first = spawn(&h, &parent, req.clone()).await;
    let second = spawn(&h, &parent, req).await;
    assert!(first.created && !second.created);
    assert_eq!(first.child_run_id, second.child_run_id);
    let prec = h.rec(&parent).await;
    assert_eq!(prec.children.len(), 1);
    assert_eq!(prec.counters.checkpoint, 1);
    let events = h.events(&first.child_run_id).await;
    let RunEventKind::RunCreated { input, .. } = &events[0].kind else {
        panic!("first event")
    };
    assert_eq!(input["context"]["mode"], "snapshot");
    assert_eq!(input["context"]["checkpoint_no"], 1);
    let (ckpt, _) = h
        .svc
        .read_checkpoint(&h.scope, &parent, Some(1))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        ckpt.core.children.len(),
        0,
        "the snapshot is taken before the link is added"
    );

    let turns = spawn_req(
        "turns",
        json!({ "objective": { "context": { "mode": "recent_turns", "turns": 1, "items": [1, 2] } } }),
    );
    let err = h
        .svc
        .spawn_child(&h.scope, &parent, turns, &alice(), None)
        .await
        .unwrap_err();
    assert!(matches!(&err, ServiceError::Refused(r) if r.code == "context_too_large"));
}
