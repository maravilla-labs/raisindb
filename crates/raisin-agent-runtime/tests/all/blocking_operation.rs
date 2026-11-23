//! An operation that blocks its thread must not hold the run hostage.
//!
//! The model-turn function awaits its provider call INSIDE the JavaScript
//! interpreter's poll (`block_in_place` + `block_on`), so the executor future
//! does not return to the scheduler until the HTTP call ends. When the driver
//! polled that future in its own select loop, nothing else in the loop ran
//! meanwhile: no lease renewal (the lease expired and the sweeper re-issued
//! the operation as attempt 2) and no reaction to a stop (the run sat in
//! `cancelling` for minutes while the model kept being called). Seen live on
//! a production run. The driver now runs the operation on its own task.

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use raisin_agent_runtime::driver::{DriveOutcome, ExecContext, Mode, OperationExecutor, RunDriver};
use raisin_agent_runtime::events::{OpOutcome, RunEventKind};
use raisin_agent_runtime::lifecycle::OperationResult;
use raisin_agent_runtime::service::ServiceConfig;
use raisin_agent_runtime::state::RunStatus;

use crate::common::*;

/// Blocks its worker thread for `ms`, ignoring the token, like a host call
/// awaited inside the interpreter.
struct ThreadBlockingExecutor {
    ms: u64,
}

#[async_trait]
impl OperationExecutor for ThreadBlockingExecutor {
    async fn execute(&self, _ctx: ExecContext) -> OperationResult {
        let ms = self.ms;
        tokio::task::block_in_place(|| std::thread::sleep(Duration::from_millis(ms)));
        OperationResult {
            outcome: Some(OpOutcome::Succeeded),
            ..OperationResult::default()
        }
    }
}

fn driver(h: &Harness, ms: u64) -> Arc<RunDriver> {
    Arc::new(RunDriver::new(
        h.svc.clone(),
        Mode::Planner(planner(vec![tool_op(), tool_op()])),
        Arc::new(ThreadBlockingExecutor { ms }),
    ))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_stop_ends_the_run_while_the_operation_still_blocks_its_thread() {
    let h = Harness::tokio();
    let run = h.create("/a").await;
    let d = driver(&h, 4_000);
    let (s, r) = (h.scope.clone(), run.clone());
    let task = tokio::spawn(async move { d.drive(&s, &r, "w1").await });
    h.wait_operating(&run).await;

    let stopped_at = Instant::now();
    h.svc
        .submit_control(&h.scope, &run, stop("c1"), None)
        .await
        .unwrap();
    let outcome = task.await.unwrap().unwrap();
    assert_eq!(outcome, DriveOutcome::Exited(RunStatus::Stopped));
    assert!(
        stopped_at.elapsed() < Duration::from_millis(1_500),
        "the stop waited for the blocked operation ({:?})",
        stopped_at.elapsed()
    );
    let events = h.events(&run).await;
    assert!(events
        .iter()
        .any(|e| matches!(e.kind, RunEventKind::OperationCancelled { .. })));
    assert!(
        !events
            .iter()
            .any(|e| matches!(e.kind, RunEventKind::OperationCompleted { .. })),
        "the late result must be discarded"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_lease_is_renewed_while_the_operation_blocks_its_thread() {
    let h = Harness::tokio_with(ServiceConfig {
        lease_ttl_ms: 400,
        renew_every_ms: 100,
        ..ServiceConfig::default()
    });
    let run = h.create("/a").await;
    let d = Arc::new(RunDriver::new(
        h.svc.clone(),
        Mode::Planner(planner(vec![tool_op()])),
        Arc::new(ThreadBlockingExecutor { ms: 1_500 }),
    ));
    let outcome = d.drive(&h.scope, &run, "w1").await.unwrap();
    assert_ne!(
        outcome,
        DriveOutcome::LeaseLost,
        "the lease ran out mid-operation"
    );
    let events = h.events(&run).await;
    assert!(events
        .iter()
        .any(|e| matches!(e.kind, RunEventKind::OperationCompleted { .. })));
}
