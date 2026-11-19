//! Real transcripts of the JS reducer (agent runs with client tools): core
//! decodes every response exactly as `FunctionDomainReducer::reduce` does
//! (`serde_json::from_value::<ReducerResponse>`), then validates it.
//!
//! Regenerate the file with
//! `node builtin-packages/ai-tools/tests/client-tools-fixtures.mjs --write`
//! (a JS test fails when it drifts from what the reducer produces).

use raisin_agent_contract::reducer::{EffectBody, ReducerRequest, ReducerResponse};
use raisin_agent_contract::validate_response;
use serde_json::Value;

fn load() -> Vec<Value> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fixtures/js-reducer/client-tools.json"
    );
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    serde_json::from_str(&text).expect("fixture is a JSON array")
}

#[test]
fn every_js_reducer_response_decodes_and_validates() {
    let entries = load();
    assert_eq!(entries.len(), 8, "scenario count; regenerate the fixture");
    for e in &entries {
        let scenario = e["scenario"].as_str().unwrap_or("?");
        let req: ReducerRequest = serde_json::from_value(e["request"].clone())
            .unwrap_or_else(|err| panic!("{scenario}: request does not decode: {err}"));
        let resp: ReducerResponse = serde_json::from_value(e["response"].clone())
            .unwrap_or_else(|err| panic!("{scenario}: response_undecodable: {err}"));
        assert!(
            resp.refused.is_none(),
            "{scenario}: refused {:?}",
            resp.refused
        );
        validate_response(&req, &resp)
            .unwrap_or_else(|r| panic!("{scenario}: {} {}", r.code, r.message));
    }
}

#[test]
fn the_page_tools_are_offered_and_the_call_is_dispatched() {
    let entries = load();
    let by = |name: &str| -> ReducerResponse {
        let e = entries.iter().find(|e| e["scenario"] == name).expect(name);
        serde_json::from_value(e["response"].clone()).unwrap()
    };
    let started = by("run_started_with_client_tools");
    let EffectBody::RequestModelTurn { tools_offered, .. } = &started.effects[0].body else {
        panic!("expected a model turn")
    };
    let names: Vec<&str> = tools_offered.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["search", "page.get_document", "page.insert_blocks"]);
    let dispatch = by("page_call_dispatch");
    let EffectBody::CallTool {
        tool, for_call_id, ..
    } = &dispatch.effects[0].body
    else {
        panic!("expected a tool call")
    };
    assert_eq!(tool, "/lib/raisin/ai/client-tool-relay");
    assert_eq!(for_call_id.as_deref(), Some("c1"));
}
