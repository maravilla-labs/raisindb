//! Steering: queued, consumed exactly once at a safe boundary.

use raisin_agent_runtime::control::ControlAck;
use raisin_agent_runtime::driver::{DriveOutcome, NextAction};
use raisin_agent_runtime::events::RunEventKind;
use raisin_agent_runtime::ids::TurnNo;
use raisin_agent_runtime::lifecycle::NewRequest;
use raisin_agent_runtime::record::{PendingKind, STEER_QUEUE_LIMIT};
use raisin_agent_runtime::service_exec::Recovered;
use raisin_agent_runtime::state::{RunStatus, WakeReason};

use crate::common::*;

fn wait_for_input() -> NextAction {
    NextAction::Wait(vec![NewRequest {
        kind: PendingKind::Input {
            prompt: "?".into(),
            choices: None,
            schema: None,
        },
        effect_id: None,
        expires_at_ms: None,
    }])
}

#[tokio::test(start_paused = true)]
async fn steer_during_op_is_queued_not_consumed() {
    let h = Harness::tokio();
    let run = h.create("/a").await;
    let p = planner(vec![tool_op()]);
    let driver = h.driver(p.clone(), executor(60_000));
    let (d, s, r) = (driver.clone(), h.scope.clone(), run.clone());
    let task = tokio::spawn(async move { d.drive(&s, &r, "w1").await });
    h.wait_operating(&run).await;
    let ack = h
        .svc
        .submit_control(&h.scope, &run, steer("s1", "also X"), None)
        .await
        .unwrap();
    assert!(matches!(ack, ControlAck::Applied { .. }));
    assert_eq!(h.rec(&run).await.steer_queue.len(), 1);
    let types = h.types(&run).await;
    assert!(types.contains(&"steer_queued".into()) && !types.contains(&"steer_consumed".into()));
    task.await.unwrap().unwrap();
}

#[tokio::test(start_paused = true)]
async fn steer_consumed_at_next_boundary_with_turn_number() {
    let h = Harness::tokio();
    let run = h.create("/a").await;
    let p = planner(vec![tool_op(), tool_op()]);
    let driver = h.driver(p.clone(), executor(60_000));
    let (d, s, r) = (driver.clone(), h.scope.clone(), run.clone());
    let task = tokio::spawn(async move { d.drive(&s, &r, "w1").await });
    h.wait_operating(&run).await;
    h.svc
        .submit_control(&h.scope, &run, steer("s1", "also X"), None)
        .await
        .unwrap();
    assert_eq!(
        task.await.unwrap().unwrap(),
        DriveOutcome::Exited(RunStatus::Queued)
    );
    let seen = p.steers_seen();
    assert_eq!(seen.len(), 1, "delivered to the planner exactly once");
    assert_eq!(seen[0].input["text"], "also X");
    let events = h.events(&run).await;
    let consumed: Vec<_> = events
        .iter()
        .filter_map(|e| match &e.kind {
            RunEventKind::SteerConsumed { turn, .. } => Some(*turn),
            _ => None,
        })
        .collect();
    assert_eq!(consumed, vec![TurnNo(1)]);
    assert!(h.rec(&run).await.steer_queue.is_empty());
}

#[tokio::test]
async fn steer_while_waiting_moves_to_queued_and_wakes() {
    let h = Harness::manual();
    let run = h.create("/a").await;
    let driver = h.driver(planner(vec![wait_for_input()]), executor(0));
    driver.drive(&h.scope, &run, "w1").await.unwrap();
    assert_eq!(h.status(&run).await, RunStatus::Waiting);
    h.waker.drain();
    h.svc
        .submit_control(&h.scope, &run, steer("s1", "go"), None)
        .await
        .unwrap();
    assert_eq!(h.status(&run).await, RunStatus::Queued);
    assert_eq!(h.waker.calls(), vec![(run.clone(), WakeReason::Steer)]);
}

#[tokio::test]
async fn steer_while_waiting_is_picked_up_by_recovery() {
    let h = Harness::manual();
    let run = h.create("/a").await;
    let driver = h.driver(planner(vec![wait_for_input()]), executor(0));
    driver.drive(&h.scope, &run, "w1").await.unwrap();
    h.svc
        .submit_control(&h.scope, &run, steer("s1", "go"), None)
        .await
        .unwrap();
    h.waker.drain(); // the wake is lost to a crash
    let recovered = h.svc.recover(&h.scope, "sweeper").await.unwrap();
    assert!(
        recovered.contains(&Recovered::Woken {
            run_id: run.clone()
        }),
        "{recovered:?}"
    );
    assert_eq!(h.waker.calls(), vec![(run.clone(), WakeReason::Steer)]);
}

#[tokio::test]
async fn steer_queue_bound_rejects_overflow() {
    let h = Harness::manual();
    let run = h.create("/a").await;
    for i in 0..STEER_QUEUE_LIMIT {
        let ack = h
            .svc
            .submit_control(&h.scope, &run, steer(&format!("s{i}"), "x"), None)
            .await
            .unwrap();
        assert!(matches!(ack, ControlAck::Applied { .. }));
    }
    let ack = h
        .svc
        .submit_control(&h.scope, &run, steer("overflow", "x"), None)
        .await
        .unwrap();
    assert!(matches!(ack, ControlAck::Rejected { ref reason, .. } if reason == "steer_queue_full"));
    assert_eq!(h.rec(&run).await.steer_queue.len(), STEER_QUEUE_LIMIT);
}

#[tokio::test]
async fn steer_not_consumed_twice_after_takeover() {
    let h = Harness::manual();
    let run = h.create("/a").await;
    h.svc
        .submit_control(&h.scope, &run, steer("s1", "x"), None)
        .await
        .unwrap();
    let grant = h.svc.acquire_lease(&h.scope, &run, "w1").await.unwrap();
    let consumed = h
        .svc
        .consume_steers(&h.scope, &run, &grant.fence)
        .await
        .unwrap();
    assert_eq!(consumed.len(), 1);
    // w1 dies here. Its lease expires; w2 takes over.
    h.advance(TTL + 1);
    h.svc.recover(&h.scope, "w2").await.unwrap();
    let p = planner(vec![]);
    h.driver(p.clone(), executor(0))
        .drive(&h.scope, &run, "w2")
        .await
        .unwrap();
    assert!(p.steers_seen().is_empty(), "w2 did not consume it again");
    let events = h.events(&run).await;
    assert_eq!(
        count(&events, |k| matches!(k, RunEventKind::SteerConsumed { .. })),
        1
    );
}
