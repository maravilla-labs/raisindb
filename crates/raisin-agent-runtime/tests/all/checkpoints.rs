//! Structured checkpoints for compaction, and server-driven (domain) children.

use std::sync::Arc;

use async_trait::async_trait;
use raisin_agent_runtime::domain::{DomainReducer, ReducerCallError, ReducerRef};
use raisin_agent_runtime::events::{CheckpointReason, OpOutcome, RunEventKind};
use raisin_agent_runtime::host::{AgentRunHost, ReducerResolver};
use raisin_agent_runtime::ids::{RunScope, SubjectRef};
use raisin_agent_runtime::lifecycle::{OperationResult, OperationSpec};
use raisin_agent_runtime::record::OperationKind;
use raisin_agent_runtime::service::ServiceError;
use raisin_agent_runtime::service_child_ctl::CheckpointWrite;
use raisin_agent_runtime::state::RunStatus;
use raisin_agent_runtime::testing::ScriptedReducer;
use serde_json::json;

use crate::children::{alice, pump, spawn_req};
use crate::common::*;
use crate::resume::{reducer_ref, tool_then_complete};

struct OneReducer(Arc<ScriptedReducer>);

#[async_trait]
impl ReducerResolver for OneReducer {
    async fn bind(&self, _: &RunScope, _: &str, _: &str) -> Result<ReducerRef, ReducerCallError> {
        Ok(self.0.reducer_ref().clone())
    }
    fn reducer(&self, _: &RunScope, _: &ReducerRef) -> Arc<dyn DomainReducer> {
        self.0.clone()
    }
}

#[tokio::test]
async fn compaction_checkpoint_references_large_outputs_and_structured_state() {
    let h = Harness::manual();
    let run = h.create("/c").await;
    let g = h.svc.acquire_lease(&h.scope, &run, "client").await.unwrap();
    let op = h
        .svc
        .begin_operation(&h.scope, &run, &g.fence, OperationSpec::default())
        .await
        .unwrap()
        .op;
    let big = OperationResult {
        payload: Some(json!({ "rows": "x".repeat(8 * 1024) })),
        ..Default::default()
    };
    h.svc
        .finish_operation(&h.scope, &run, &g.fence, &op.op_id, big)
        .await
        .unwrap();
    let rec = h.rec(&run).await;
    assert_eq!(rec.large_results.len(), 1);
    let big_key = rec.large_results[0].key.clone();

    let write = CheckpointWrite {
        fence: Some(g.fence.clone()),
        summary: Some("prose, never load-bearing".into()),
        transcript_cutoff: Some(SubjectRef {
            workspace: "ws".into(),
            path: "/chat/1/msg-9".into(),
            node_id: None,
        }),
        state: Some(
            json!({ "objective": "board app", "decisions": ["use /local"], "pending_questions": [] }),
        ),
        ..Default::default()
    };
    let ckpt = h.svc.write_checkpoint(&h.scope, &run, write).await.unwrap();
    assert_eq!(ckpt.reason, CheckpointReason::Compaction);
    assert!(
        ckpt.large_refs.iter().any(|r| r.key == big_key),
        "large output referenced, not copied"
    );
    assert!(ckpt.structured_ref.is_some());
    let (read, state) = h
        .svc
        .read_checkpoint(&h.scope, &run, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(read.checkpoint_no, ckpt.checkpoint_no);
    assert_eq!(state.unwrap()["decisions"][0], "use /local");
    assert_eq!(read.transcript_cutoff.unwrap().path, "/chat/1/msg-9");

    // An unknown ref is refused; a caller that is neither the lease holder nor
    // the in-flight operation cannot write one while a driver holds the run.
    let bad = CheckpointWrite {
        fence: Some(g.fence.clone()),
        large_refs: vec![raisin_agent_runtime::events::ResultRef {
            key: "nope".into(),
            bytes: 1,
            content_type: "application/json".into(),
        }],
        ..Default::default()
    };
    assert!(matches!(
        h.svc.write_checkpoint(&h.scope, &run, bad).await,
        Err(ServiceError::Invalid(_))
    ));
    let anon = CheckpointWrite::default();
    assert!(matches!(
        h.svc.write_checkpoint(&h.scope, &run, anon).await,
        Err(ServiceError::Invalid(_))
    ));

    // A compaction running AS an operation checkpoints by naming itself.
    let spec = OperationSpec {
        kind: Some(OperationKind::Compaction),
        ..Default::default()
    };
    let op = h
        .svc
        .begin_operation(&h.scope, &run, &g.fence, spec)
        .await
        .unwrap()
        .op;
    let by_op = CheckpointWrite {
        operation_id: Some(op.op_id.0.clone()),
        state: Some(json!({ "phase": "apply" })),
        ..Default::default()
    };
    let c2 = h.svc.write_checkpoint(&h.scope, &run, by_op).await.unwrap();
    assert_eq!(c2.checkpoint_no, ckpt.checkpoint_no + 1);
    let wrong = CheckpointWrite {
        operation_id: Some("x/op/99".into()),
        ..Default::default()
    };
    assert!(h.svc.write_checkpoint(&h.scope, &run, wrong).await.is_err());
    // Checkpoint 1 is still readable by number.
    assert!(h
        .svc
        .read_checkpoint(&h.scope, &run, Some(ckpt.checkpoint_no))
        .await
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn a_server_driven_child_runs_on_the_job_queue_and_hands_back() {
    let h = Harness::manual();
    let reducer = tool_then_complete("h1");
    let host = AgentRunHost::new(
        h.svc.clone(),
        Arc::new(OneReducer(reducer)),
        executor(0),
        "node-a",
    );
    let parent = h.create("/p").await;
    let mut req = spawn_req("domain child", json!({}));
    req.reducer = Some(reducer_ref("h1"));
    let child = h
        .svc
        .spawn_child(&h.scope, &parent, req, &alice(), None)
        .await
        .unwrap()
        .child_run_id;
    pump(&h, &host).await;
    assert_eq!(h.status(&child).await, RunStatus::Completed);
    let prec = h.rec(&parent).await;
    assert!(prec.children[0].delivered);
    assert_eq!(prec.usage.child_operations, 1);
    assert_eq!(prec.mailbox[0].status, Some(RunStatus::Completed));
}

#[tokio::test]
async fn a_domain_child_calling_a_denied_tool_gets_a_blocked_result_not_a_crash() {
    let h = Harness::manual();
    let host = AgentRunHost::new(
        h.svc.clone(),
        Arc::new(OneReducer(tool_then_complete("h1"))),
        executor(0),
        "node-a",
    );
    let parent = h.create("/p").await;
    let mut req = spawn_req(
        "narrow",
        json!({ "objective": { "allowed_tools": ["/lib/only/this"] } }),
    );
    req.reducer = Some(reducer_ref("h1"));
    let child = h
        .svc
        .spawn_child(&h.scope, &parent, req, &alice(), None)
        .await
        .unwrap()
        .child_run_id;
    pump(&h, &host).await;
    let blocked = count(&h.events(&child).await, |k| {
        matches!(
            k,
            RunEventKind::OperationCompleted {
                outcome: OpOutcome::Blocked,
                ..
            }
        )
    });
    assert_eq!(blocked, 1, "{:?}", h.types(&child).await);
    // The scripted reducer does not re-plan, so the child ends — honestly —
    // and still hands back.
    assert!(h.status(&child).await.is_terminal());
    assert!(h.rec(&parent).await.children[0].delivered);
}
