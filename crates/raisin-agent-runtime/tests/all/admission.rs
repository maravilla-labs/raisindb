//! Admission: at most one live run per subject.

use raisin_agent_runtime::state::RunStatus;
use raisin_agent_runtime::store::CreateOutcome;

use crate::common::*;

#[tokio::test]
async fn second_create_for_same_subject_returns_existing_with_status() {
    let h = Harness::manual();
    let run = h.create("/chat-1").await;
    h.svc.acquire_lease(&h.scope, &run, "w1").await.unwrap();
    let again = h.svc.create(h.request("/chat-1"), None).await.unwrap();
    assert_eq!(
        again,
        CreateOutcome::Existing {
            run_id: run,
            status: RunStatus::Running
        }
    );
}

#[tokio::test]
async fn create_key_dedupes_retried_trigger() {
    let h = Harness::manual();
    let mut req = h.request("/chat-1");
    req.create_key = Some("message-42".into());
    let run = h.create_with(req.clone()).await;
    h.svc
        .submit_control(&h.scope, &run, stop("c"), None)
        .await
        .unwrap();
    // The subject is free again, but the retried trigger carries the same key.
    let again = h.svc.create(req, None).await.unwrap();
    assert_eq!(
        again,
        CreateOutcome::Existing {
            run_id: run,
            status: RunStatus::Stopped
        }
    );
}

#[tokio::test]
async fn subject_free_again_after_terminal() {
    let h = Harness::manual();
    let run = h.create("/chat-1").await;
    h.svc
        .submit_control(&h.scope, &run, stop("c"), None)
        .await
        .unwrap();
    let next = h.svc.create(h.request("/chat-1"), None).await.unwrap();
    assert!(matches!(next, CreateOutcome::Created { ref run_id, .. } if *run_id != run));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn create_racing_terminal_commit_treats_terminal_run_as_free() {
    let h = Harness::manual();
    for i in 0..30 {
        let path = format!("/race-{i}");
        let run = h.create(&path).await;
        let (svc, s, r) = (h.svc.clone(), h.scope.clone(), run.clone());
        let stopper =
            tokio::spawn(async move { svc.submit_control(&s, &r, stop("c"), None).await });
        let outcome = h.svc.create(h.request(&path), None).await.unwrap();
        stopper.await.unwrap().unwrap();
        match outcome {
            CreateOutcome::Existing { run_id, status } => {
                assert_eq!(run_id, run);
                assert!(
                    !status.is_terminal(),
                    "a terminal run was reported as the live run"
                );
            }
            CreateOutcome::Created { .. } => assert!(h.status(&run).await.is_terminal()),
        }
    }
}
