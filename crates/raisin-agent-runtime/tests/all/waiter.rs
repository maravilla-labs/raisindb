//! A flow step waiting for a run: the result is owed durably until the sink
//! takes it, delivered once, and repaired by the sweeper's lineage pass.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use raisin_agent_runtime::events::RunEventKind;
use raisin_agent_runtime::record::AgentRunRecord;
use raisin_agent_runtime::state::RunStatus;
use raisin_agent_runtime::waiter::{RunWaiter, WaiterSink, FLOW_INSTANCE};
use serde_json::{json, Value};

use crate::common::*;

#[derive(Default)]
struct Sink {
    fail: Mutex<bool>,
    got: Mutex<Vec<Value>>,
}

#[async_trait]
impl WaiterSink for Sink {
    async fn notify(
        &self,
        _run: &AgentRunRecord,
        _w: &RunWaiter,
        result: Value,
    ) -> Result<(), String> {
        if *self.fail.lock().unwrap() {
            return Err("queue down".into());
        }
        self.got.lock().unwrap().push(result);
        Ok(())
    }
}

fn waiter(instance: &str) -> RunWaiter {
    RunWaiter {
        kind: FLOW_INSTANCE.into(),
        target: instance.into(),
        branch: "main".into(),
        data: json!({ "step_id": "ask" }),
        delivered: false,
    }
}

async fn stopped_run(h: &Harness, path: &str, instance: &str) -> raisin_agent_runtime::ids::RunId {
    let mut req = h.request(path);
    req.waiter = Some(waiter(instance));
    let run = h.create_with(req).await;
    h.svc
        .submit_control(&h.scope, &run, stop("s"), None)
        .await
        .unwrap();
    assert_eq!(h.status(&run).await, RunStatus::Stopped);
    run
}

#[tokio::test]
async fn a_waiter_is_owed_until_the_sink_takes_it_and_told_once() {
    let h = Harness::manual();
    let run = stopped_run(&h, "/w", "inst-1").await;
    // No sink on this node yet: owed, and on the worklist.
    assert!(!h.svc.deliver_waiter(&h.scope, &run).await.unwrap());
    assert!(h.rec(&run).await.handback_owed());
    assert_eq!(
        h.svc
            .store()
            .scan_handback_owed(&h.scope, 10)
            .await
            .unwrap(),
        vec![run.clone()]
    );

    let sink = Arc::new(Sink::default());
    h.svc.set_waiter_sink(sink.clone());
    *sink.fail.lock().unwrap() = true;
    assert!(
        h.svc.deliver_waiter(&h.scope, &run).await.is_err(),
        "a failed hand-over stays owed"
    );
    assert!(h.rec(&run).await.handback_owed());

    *sink.fail.lock().unwrap() = false;
    assert!(h.svc.deliver_waiter(&h.scope, &run).await.unwrap());
    assert!(
        !h.svc.deliver_waiter(&h.scope, &run).await.unwrap(),
        "told once"
    );
    let got = sink.got.lock().unwrap().clone();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0]["agent_run_id"], json!(run));
    assert_eq!(got[0]["status"], json!("stopped"));
    assert_eq!(got[0]["waiter"]["data"]["step_id"], json!("ask"));
    assert!(!h.rec(&run).await.handback_owed());
    assert!(h
        .svc
        .store()
        .scan_handback_owed(&h.scope, 10)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        count(&h.events(&run).await, |k| matches!(
            k,
            RunEventKind::WaiterNotified { .. }
        )),
        1
    );
}

#[tokio::test]
async fn the_lineage_pass_delivers_an_owed_waiter() {
    let h = Harness::manual();
    let sink = Arc::new(Sink::default());
    h.svc.set_waiter_sink(sink.clone());
    let run = stopped_run(&h, "/w2", "inst-2").await;
    h.svc.repair_lineage(&h.scope).await.unwrap();
    assert_eq!(sink.got.lock().unwrap().len(), 1);
    assert!(!h.rec(&run).await.handback_owed());
    // Settling the terminal run again changes nothing.
    h.svc.settle_lineage(&h.scope, &run).await.unwrap();
    assert_eq!(sink.got.lock().unwrap().len(), 1);
}
