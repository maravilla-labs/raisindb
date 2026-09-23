//! Shared harness: in-memory store, recording waker, injectable clock.

use std::sync::Arc;
use std::time::Duration;

use raisin_agent_runtime::clock::{Clock, ManualClock, TokioClock};
use raisin_agent_runtime::conformance::{create_request, scope};
use raisin_agent_runtime::control::{ActorRef, ControlCommand, ControlKind};
use raisin_agent_runtime::driver::{Mode, NextAction, RunDriver, StepPlanner};
use raisin_agent_runtime::events::{RunEvent, RunEventKind};
use raisin_agent_runtime::ids::{ControlId, RunId, RunScope, Seq};
use raisin_agent_runtime::lifecycle::OperationSpec;
use raisin_agent_runtime::memory::InMemoryAgentRunStore;
use raisin_agent_runtime::record::{AgentRunRecord, OperationKind};
use raisin_agent_runtime::service::{AgentRunService, CreateRun, ServiceConfig};
use raisin_agent_runtime::state::{Activity, RunState, RunStatus};
use raisin_agent_runtime::store::CreateOutcome;
use raisin_agent_runtime::testing::{ExecBehavior, RecordingExecutor, ScriptedPlanner};
use raisin_agent_runtime::wake::RecordingWaker;

pub const T0: u64 = 1_000_000;
pub const TTL: u64 = 90_000;

pub struct Harness {
    pub svc: Arc<AgentRunService>,
    pub store: Arc<InMemoryAgentRunStore>,
    pub waker: Arc<RecordingWaker>,
    pub manual: Option<Arc<ManualClock>>,
    pub scope: RunScope,
}

impl Harness {
    fn build(clock: Arc<dyn Clock>, manual: Option<Arc<ManualClock>>) -> Self {
        let store = Arc::new(InMemoryAgentRunStore::default());
        let waker = Arc::new(RecordingWaker::default());
        let svc = Arc::new(AgentRunService::new(
            store.clone(),
            waker.clone(),
            clock,
            ServiceConfig::default(),
        ));
        Self {
            svc,
            store,
            waker,
            manual,
            scope: scope("t1"),
        }
    }

    /// A harness on a hand-moved clock.
    pub fn manual() -> Self {
        let clock = Arc::new(ManualClock::new(T0));
        Self::build(clock.clone(), Some(clock))
    }

    /// A harness on tokio time (use with `start_paused`).
    pub fn tokio() -> Self {
        Self::build(Arc::new(TokioClock::new(T0)), None)
    }

    pub fn advance(&self, ms: u64) {
        self.manual.as_ref().expect("manual clock").advance(ms);
    }

    pub fn request(&self, path: &str) -> CreateRun {
        create_request(&self.scope, path)
    }

    pub async fn create_with(&self, req: CreateRun) -> RunId {
        match self.svc.create(req, None).await.expect("create") {
            CreateOutcome::Created { run_id, .. } => run_id,
            other => panic!("expected Created, got {other:?}"),
        }
    }

    pub async fn create(&self, path: &str) -> RunId {
        self.create_with(self.request(path)).await
    }

    pub async fn rec(&self, run: &RunId) -> AgentRunRecord {
        self.svc
            .get(&self.scope, run)
            .await
            .unwrap()
            .expect("record")
    }

    pub async fn status(&self, run: &RunId) -> RunStatus {
        self.rec(run).await.state.status()
    }

    pub async fn events(&self, run: &RunId) -> Vec<RunEvent> {
        self.svc
            .read_events(&self.scope, run, Seq(0), 10_000)
            .await
            .unwrap()
    }

    pub async fn types(&self, run: &RunId) -> Vec<String> {
        self.events(run)
            .await
            .iter()
            .map(|e| e.kind.type_name())
            .collect()
    }

    pub fn driver(
        &self,
        planner: Arc<dyn StepPlanner>,
        exec: Arc<RecordingExecutor>,
    ) -> Arc<RunDriver> {
        Arc::new(RunDriver::new(
            self.svc.clone(),
            Mode::Planner(planner),
            exec,
        ))
    }

    /// Poll (on tokio time) until `pred` holds for the record.
    pub async fn wait_until(&self, run: &RunId, pred: impl Fn(&AgentRunRecord) -> bool) {
        for _ in 0..10_000 {
            if pred(&self.rec(run).await) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("condition never held");
    }

    pub async fn wait_operating(&self, run: &RunId) {
        self.wait_until(run, |r| {
            matches!(
                r.state,
                RunState::Running {
                    activity: Activity::Operating { .. },
                    ..
                }
            )
        })
        .await;
    }
}

pub fn cmd(id: &str, kind: ControlKind) -> ControlCommand {
    ControlCommand {
        control_id: ControlId(id.into()),
        kind,
        issued_by: ActorRef::user("alice"),
        at_ms: T0,
    }
}

pub fn stop(id: &str) -> ControlCommand {
    cmd(
        id,
        ControlKind::Stop {
            reason: Some("user".into()),
        },
    )
}

pub fn steer(id: &str, text: &str) -> ControlCommand {
    cmd(
        id,
        ControlKind::Steer {
            input: serde_json::json!({ "text": text }),
        },
    )
}

pub fn tool_op() -> NextAction {
    NextAction::Operation(OperationSpec {
        kind: Some(OperationKind::ToolCall),
        ..OperationSpec::default()
    })
}

pub fn op_with(f: impl FnOnce(&mut OperationSpec)) -> NextAction {
    let mut spec = OperationSpec {
        kind: Some(OperationKind::ToolCall),
        ..OperationSpec::default()
    };
    f(&mut spec);
    NextAction::Operation(spec)
}

pub fn planner(actions: Vec<NextAction>) -> Arc<ScriptedPlanner> {
    Arc::new(ScriptedPlanner::new(actions))
}

pub fn executor(delay_ms: u64) -> Arc<RecordingExecutor> {
    Arc::new(RecordingExecutor::new(ExecBehavior {
        delay_ms,
        ..ExecBehavior::default()
    }))
}

/// Whether `needles` occur in `hay` in order (not necessarily adjacent).
pub fn subsequence(hay: &[String], needles: &[&str]) -> bool {
    let mut it = hay.iter();
    needles.iter().all(|n| it.any(|h| h == n))
}

pub fn count(events: &[RunEvent], f: impl Fn(&RunEventKind) -> bool) -> usize {
    events.iter().filter(|e| f(&e.kind)).count()
}

/// Assert the terminal-record guarantees.
pub fn assert_clean_terminal(rec: &AgentRunRecord) {
    assert!(
        rec.state.is_terminal(),
        "not terminal: {:?}",
        rec.state.status()
    );
    assert!(rec.state.lease().is_none());
    assert!(rec.state.active_op().is_none());
    assert!(rec.state.open_requests().is_empty());
    assert!(rec.steer_queue.is_empty());
    assert!(rec.unanswered_calls.is_empty());
    assert!(rec.domain.as_ref().is_none_or(|d| d.outbox.is_none()));
}
