//! The domain-reducer seam: one commit per response, log as inbox, finalize.

use std::sync::Arc;

use raisin_agent_contract::{
    effect_id, Effect, EffectBody, EventKind, ReducerResponse, ToolResultEntry, CONTRACT_V1,
};
use raisin_agent_runtime::control::ControlKind;
use raisin_agent_runtime::driver::{DriveOutcome, DriverHooks, HookAction, Mode, RunDriver};
use raisin_agent_runtime::events::RunEventKind;
use raisin_agent_runtime::ids::{CallId, RunId};
use raisin_agent_runtime::lifecycle::{self, BeginRefusal, OperationSpec};
use raisin_agent_runtime::record::{ActiveOperation, OperationKind};
use raisin_agent_runtime::state::RunStatus;
use raisin_agent_runtime::store::CommitRequest;
use raisin_agent_runtime::testing::{ExecBehavior, RecordingExecutor, ScriptedReducer};
use serde_json::{json, Value};

use crate::common::*;
use crate::resume::{reducer_ref, tool_then_complete};

pub fn resp(
    req_rev: u64,
    changed: bool,
    effects: Vec<EffectBody>,
    projection: Option<Value>,
) -> ReducerResponse {
    let rev = if changed || !effects.is_empty() {
        req_rev + 1
    } else {
        req_rev
    };
    ReducerResponse {
        contract: CONTRACT_V1.into(),
        state: json!({ "rev": rev }),
        state_rev: rev,
        effects: effects
            .into_iter()
            .enumerate()
            .map(|(i, body)| Effect {
                effect_id: effect_id(rev, i),
                body,
            })
            .collect(),
        projection,
        diagnostics: vec![],
        refused: None,
    }
}

pub fn call(tool: &str, for_call: Option<&str>, interruptible: bool) -> EffectBody {
    EffectBody::CallTool {
        tool: tool.into(),
        args: json!({ "q": 1 }),
        mutating: false,
        replay_safe: true,
        interruptible,
        timeout_ms: None,
        for_call_id: for_call.map(str::to_owned),
    }
}

pub fn model_turn(results: Vec<&str>) -> EffectBody {
    EffectBody::RequestModelTurn {
        tools_offered: vec![],
        instructions: None,
        context: None,
        tool_results: results
            .into_iter()
            .map(|c| ToolResultEntry {
                call_id: c.into(),
                synthetic: false,
                content: json!("ok"),
            })
            .collect(),
        output_schema: None,
    }
}

pub fn complete() -> EffectBody {
    EffectBody::Complete {
        outcome: raisin_agent_contract::CompleteOutcome::Succeeded,
        summary: None,
        artifacts: vec![],
        evidence: vec![],
    }
}

pub async fn domain_run(h: &Harness, hash: &str) -> RunId {
    let mut req = h.request("/d");
    req.reducer = Some(reducer_ref(hash));
    h.create_with(req).await
}

/// model turn → tool call answering c1 → model turn with c1's result → done.
fn chat_reducer() -> Arc<ScriptedReducer> {
    ScriptedReducer::new("h", |req| {
        let r = req.state_rev;
        Ok(match req.event.kind {
            EventKind::RunStarted => resp(r, true, vec![model_turn(vec![])], None),
            EventKind::ModelTurnCompleted => {
                let calls = req.event.data["tool_calls"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                match calls.first() {
                    Some(c) => resp(r, true, vec![call("/lib/read", c.as_str(), true)], None),
                    None => resp(r, true, vec![complete()], None),
                }
            }
            EventKind::ToolResult => {
                let id = req.event.data["call_id"].as_str().unwrap().to_owned();
                resp(r, true, vec![model_turn(vec![id.as_str()])], None)
            }
            _ => resp(r, false, vec![], None),
        })
    })
}

fn chat_executor() -> Arc<RecordingExecutor> {
    let exec = Arc::new(RecordingExecutor::new(ExecBehavior::default()));
    exec.push(ExecBehavior {
        tool_calls: vec![CallId("c1".into())],
        payload: Some(json!({ "message": { "text": "reading" } })),
        ..ExecBehavior::default()
    });
    exec.push(ExecBehavior { payload: Some(json!({ "envelope": "raisin.tool-result/1", "operation_id": "x", "status": "succeeded" })), ..ExecBehavior::default() });
    exec.push(ExecBehavior {
        payload: Some(json!({ "message": { "text": "done" } })),
        ..ExecBehavior::default()
    });
    exec
}

#[tokio::test]
async fn domain_planner_maps_effects_to_operations() {
    let h = Harness::manual();
    let run = domain_run(&h, "h").await;
    let exec = chat_executor();
    let out = RunDriver::new(h.svc.clone(), Mode::Domain(chat_reducer()), exec.clone())
        .drive(&h.scope, &run, "w")
        .await
        .unwrap();
    assert_eq!(out, DriveOutcome::Exited(RunStatus::Completed));
    let events = h.events(&run).await;
    let kinds: Vec<OperationKind> = events
        .iter()
        .filter_map(|e| match &e.kind {
            RunEventKind::OperationStarted { kind, .. } => Some(kind.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        kinds,
        vec![
            OperationKind::ModelTurn,
            OperationKind::ToolCall,
            OperationKind::ModelTurn
        ]
    );
    let started = exec.started();
    assert_eq!(started.len(), 3);
    assert!(h.rec(&run).await.unanswered_calls.is_empty());
}

#[tokio::test]
async fn reducer_response_and_begin_operation_commit_atomically() {
    let h = Harness::manual();
    let run = domain_run(&h, "h").await;
    h.store
        .fail_commits_when(Some(Arc::new(|req: &CommitRequest| {
            req.events
                .iter()
                .any(|e| matches!(e.kind, RunEventKind::DomainApplied { .. }))
        })));
    let driver = RunDriver::new(h.svc.clone(), Mode::Domain(chat_reducer()), chat_executor());
    assert!(driver.drive(&h.scope, &run, "w").await.is_err());
    let types = h.types(&run).await;
    assert!(
        !types.contains(&"domain_applied".into()) && !types.contains(&"operation_started".into()),
        "{types:?}"
    );
    h.store.fail_commits_when(None);
    let events = h.events(&run).await;
    let mut ok = RunDriver::new(
        h.svc.clone(),
        Mode::Domain(tool_then_complete("h")),
        executor(0),
    );
    let _ = &mut ok;
    // The next successful step writes both together: find them adjacent.
    let g = h.rec(&run).await.state.lease().cloned().unwrap();
    let fence = lifecycle::LeaseFence {
        owner: g.owner,
        epoch: g.epoch,
    };
    h.svc
        .domain_step(&h.scope, &run, &fence, tool_then_complete("h").as_ref())
        .await
        .unwrap();
    let after = h.events(&run).await;
    let new: Vec<String> = after[events.len()..]
        .iter()
        .map(|e| e.kind.type_name())
        .collect();
    assert_eq!(new[0], "domain_applied");
    assert!(new.contains(&"operation_started".to_string()), "{new:?}");
}

struct CrashAtBegin(std::sync::atomic::AtomicBool);

#[async_trait::async_trait]
impl DriverHooks for CrashAtBegin {
    async fn after_begin_commit(&self, _op: &ActiveOperation) -> HookAction {
        if self.0.swap(true, std::sync::atomic::Ordering::SeqCst) {
            HookAction::Continue
        } else {
            HookAction::Crash
        }
    }
}

#[tokio::test]
async fn crash_after_reducer_commit_before_dispatch_resumes_same_effect() {
    let h = Harness::manual();
    let run = domain_run(&h, "h").await;
    let reducer = tool_then_complete("h");
    let exec = executor(0);
    let d = RunDriver::new(h.svc.clone(), Mode::Domain(reducer.clone()), exec.clone())
        .with_hooks(Arc::new(CrashAtBegin(Default::default())));
    assert_eq!(
        d.drive(&h.scope, &run, "w1").await.unwrap(),
        DriveOutcome::Crashed
    );
    assert_eq!(reducer.calls().len(), 1);
    assert_eq!(exec.total_side_effects(), 0, "never dispatched");
    h.advance(TTL + 1);
    let out = d.recover_and_drive(&h.scope, "w2").await.unwrap();
    assert_eq!(
        out,
        vec![(run.clone(), DriveOutcome::Exited(RunStatus::Completed))]
    );
    let calls = reducer.calls();
    assert_eq!(
        calls
            .iter()
            .filter(|c| c.event.kind == EventKind::RunStarted)
            .count(),
        1,
        "the reducer was not re-asked"
    );
    let tool_result = calls
        .iter()
        .find(|c| c.event.kind == EventKind::ToolResult)
        .unwrap();
    assert_eq!(
        tool_result.event.effect_id.as_deref(),
        Some("1:0"),
        "the SAME effect was resumed"
    );
    assert_eq!(exec.total_side_effects(), 1);
}

#[tokio::test]
async fn duplicate_reducer_invocation_same_state_rev_same_effect_ids() {
    let h = Harness::manual();
    let run = domain_run(&h, "h").await;
    let reducer = tool_then_complete("h");
    let g = h.svc.acquire_lease(&h.scope, &run, "w").await.unwrap();
    let once = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = once.clone();
    h.store
        .fail_commits_when(Some(Arc::new(move |req: &CommitRequest| {
            req.events
                .iter()
                .any(|e| matches!(e.kind, RunEventKind::DomainApplied { .. }))
                && !flag.swap(true, std::sync::atomic::Ordering::SeqCst)
        })));
    assert!(h
        .svc
        .domain_step(&h.scope, &run, &g.fence, reducer.as_ref())
        .await
        .is_err());
    h.svc
        .domain_step(&h.scope, &run, &g.fence, reducer.as_ref())
        .await
        .unwrap();
    let calls = reducer.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(
        calls[0], calls[1],
        "re-delivered the same event against the same state_rev"
    );
    let events = h.events(&run).await;
    let started: Vec<_> = events
        .iter()
        .filter_map(|e| match &e.kind {
            RunEventKind::OperationStarted { effect_id, .. } => effect_id.clone(),
            _ => None,
        })
        .collect();
    assert_eq!(started, vec!["1:0".to_string()]);
}

#[tokio::test]
async fn synthesized_events_are_persisted_before_delivery() {
    let h = Harness::manual();
    let mut req = h.request("/d");
    req.reducer = Some(reducer_ref("h"));
    req.budgets.max_operations = Some(0);
    let run = h.create_with(req).await;
    let reducer = tool_then_complete("h");
    let d = RunDriver::new(h.svc.clone(), Mode::Domain(reducer.clone()), executor(0));
    d.drive(&h.scope, &run, "w").await.unwrap();
    let inc = raisin_agent_runtime::record::RunBudgets {
        max_operations: Some(9),
        ..Default::default()
    };
    h.svc
        .submit_control(
            &h.scope,
            &run,
            cmd(
                "r",
                ControlKind::Resume {
                    budget_increase: Some(inc),
                    accept_reducer_change: false,
                },
            ),
            None,
        )
        .await
        .unwrap();
    d.drive(&h.scope, &run, "w").await.unwrap();
    let events = h.events(&run).await;
    for kind in [EventKind::BudgetExceeded, EventKind::Resumed] {
        let call = reducer
            .calls()
            .into_iter()
            .find(|c| c.event.kind == kind)
            .unwrap_or_else(|| panic!("{kind:?} delivered"));
        let persisted = &events[(call.event.seq - 1) as usize];
        let want = match kind {
            EventKind::BudgetExceeded => "budget_exceeded",
            _ => "resumed",
        };
        assert_eq!(
            persisted.kind.type_name(),
            want,
            "delivered with its persisted seq"
        );
    }
}

#[tokio::test]
async fn unanswered_tool_calls_refused() {
    // Core-level: a model turn cannot begin while a call is unanswered.
    let h = Harness::manual();
    let run = h.create("/a").await;
    let mut rec = h.rec(&run).await;
    rec = lifecycle::apply_acquire(&rec, "w", T0, TTL).unwrap().record;
    rec.unanswered_calls = vec![CallId("c1".into())];
    let spec = OperationSpec {
        kind: Some(OperationKind::ModelTurn),
        ..OperationSpec::default()
    };
    assert!(matches!(
        lifecycle::apply_begin(&rec, spec, T0),
        Err(BeginRefusal::UnansweredCalls(_))
    ));
    // Reducer-level: R12 fails the run honestly.
    let run = domain_run(&h, "h2").await;
    let reducer = ScriptedReducer::new("h2", |req| {
        let r = req.state_rev;
        Ok(match req.event.kind {
            EventKind::RunStarted => resp(r, true, vec![model_turn(vec![])], None),
            _ => resp(r, true, vec![model_turn(vec![])], None), // forgets c1
        })
    });
    let exec = Arc::new(RecordingExecutor::new(ExecBehavior {
        tool_calls: vec![CallId("c1".into())],
        ..ExecBehavior::default()
    }));
    RunDriver::new(h.svc.clone(), Mode::Domain(reducer), exec)
        .drive(&h.scope, &run, "w")
        .await
        .unwrap();
    let rec = h.rec(&run).await;
    assert_eq!(rec.state.status(), RunStatus::Failed);
    assert_eq!(
        rec.status_reason.as_deref(),
        Some("reducer_refused:unanswered_tool_calls")
    );
}

/// The inbox is the whole log, not its next page: more non-deliverable events
/// (here 400, from rejected controls) than one read returns must not hide the
/// deliverable event behind them and fail the run as `reducer_stalled`.
#[tokio::test(start_paused = true)]
async fn deliverable_event_behind_a_page_of_noise_is_found() {
    use raisin_agent_runtime::control::ActorRef;
    let h = Harness::tokio();
    let run = domain_run(&h, "h").await;
    let reducer = ScriptedReducer::new("h", |req| {
        let r = req.state_rev;
        Ok(match req.event.kind {
            EventKind::RunStarted => resp(
                r,
                true,
                vec![EffectBody::AskUser {
                    question: "which?".into(),
                    choices: None,
                    schema: None,
                    expires_in_ms: None,
                }],
                None,
            ),
            EventKind::RequestResolved => resp(r, true, vec![complete()], None),
            _ => resp(r, true, vec![], None),
        })
    });
    let d = RunDriver::new(h.svc.clone(), Mode::Domain(reducer.clone()), executor(0));
    assert_eq!(
        d.drive(&h.scope, &run, "w").await.unwrap(),
        DriveOutcome::Exited(RunStatus::Waiting)
    );
    for i in 0..200 {
        let mut c = stop(&format!("noise-{i}"));
        c.issued_by = ActorRef::user("mallory");
        h.svc.submit_control(&h.scope, &run, c, None).await.unwrap();
    }
    let request_id = h.rec(&run).await.state.open_requests()[0]
        .request_id
        .clone();
    let answer = cmd(
        "answer",
        ControlKind::ProvideInput {
            request_id,
            value: json!("a"),
        },
    );
    h.svc
        .submit_control(&h.scope, &run, answer, None)
        .await
        .unwrap();
    assert_eq!(
        d.drive(&h.scope, &run, "w").await.unwrap(),
        DriveOutcome::Exited(RunStatus::Completed)
    );
    let kinds: Vec<EventKind> = reducer.calls().iter().map(|c| c.event.kind).collect();
    assert_eq!(
        kinds,
        vec![EventKind::RunStarted, EventKind::RequestResolved]
    );
}
