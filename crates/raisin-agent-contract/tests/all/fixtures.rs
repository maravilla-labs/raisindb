//! Runs every shared fixture file and asserts the file count.

use std::path::Path;

use raisin_agent_contract::fixtures::{
    check_reducer_fixture, check_tool_result_fixture, fixture_files, FIXTURES_DIR,
    REDUCER_FIXTURE_COUNT, TOOL_RESULT_FIXTURE_COUNT,
};
use raisin_agent_contract::{Effect, EffectBody, ReducerResponse};
use serde_json::Value;

fn run(sub: &str, expected: usize, check: fn(&Value) -> Result<(), String>) {
    let dir = Path::new(FIXTURES_DIR).join(sub);
    let files = fixture_files(&dir);
    assert_eq!(files.len(), expected, "fixture count in {}", dir.display());
    let mut failures = Vec::new();
    for file in &files {
        let text = std::fs::read_to_string(file).unwrap();
        let value: Value = serde_json::from_str(&text).unwrap();
        if let Err(e) = check(&value) {
            failures.push(format!(
                "{}: {e}",
                file.file_name().unwrap().to_string_lossy()
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "fixture failures:\n{}",
        failures.join("\n")
    );
}

#[test]
fn reducer_fixtures_pass_with_expected_count() {
    run("reducer/v1", REDUCER_FIXTURE_COUNT, check_reducer_fixture);
}

#[test]
fn tool_result_fixtures_pass_with_expected_count() {
    run(
        "tool-result/v1",
        TOOL_RESULT_FIXTURE_COUNT,
        check_tool_result_fixture,
    );
}

#[test]
fn every_refusal_code_has_a_fixture() {
    let dir = Path::new(FIXTURES_DIR).join("reducer/v1");
    let mut codes: Vec<String> = fixture_files(&dir)
        .iter()
        .map(|f| {
            let v: Value = serde_json::from_str(&std::fs::read_to_string(f).unwrap()).unwrap();
            v["expect"].as_str().unwrap().to_owned()
        })
        .filter(|c| c != "ok")
        .collect();
    codes.sort();
    codes.dedup();
    // R1 and R9 are exercised by unit-level tests below rather than files.
    assert_eq!(codes.len(), 11, "{codes:?}");
}

#[test]
fn unknown_effect_kind_decodes_as_unknown_and_roundtrips_known_kinds() {
    let v = serde_json::json!({"effect_id": "1:0", "kind": "teleport", "x": 1});
    let e: Effect = serde_json::from_value(v).unwrap();
    assert_eq!(e.body, EffectBody::Unknown);
    let v = serde_json::json!({"effect_id": "1:0", "kind": "withdraw_request", "request_id": "r"});
    let e: Effect = serde_json::from_value(v.clone()).unwrap();
    assert_eq!(serde_json::to_value(&e).unwrap(), v);
}

#[test]
fn contract_mismatch_and_oversized_state_are_refused() {
    let fixture: Value = serde_json::from_str(
        &std::fs::read_to_string(Path::new(FIXTURES_DIR).join("reducer/v1/ok-complete.json"))
            .unwrap(),
    )
    .unwrap();
    let req = serde_json::from_value(fixture["request"].clone()).unwrap();
    let mut resp: ReducerResponse = serde_json::from_value(fixture["response"].clone()).unwrap();
    resp.contract = "raisin.agent-run.reducer/9".into();
    let err = raisin_agent_contract::validate_response(&req, &resp).unwrap_err();
    assert_eq!(err.code, "contract_mismatch");
    resp.contract = raisin_agent_contract::CONTRACT_V1.into();
    resp.state = serde_json::json!({"blob": "x".repeat(300 * 1024)});
    let err = raisin_agent_contract::validate_response(&req, &resp).unwrap_err();
    assert_eq!(err.code, "state_too_large");
}
