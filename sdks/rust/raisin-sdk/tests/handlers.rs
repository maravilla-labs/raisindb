//! Handler registration, name routing, and the unknown-name answer.
//!
//! These run natively: `#[handler]` and `export!` generate the same dispatch
//! table on every target, and only the WIT export itself is wasm-only.

use raisin_sdk::testing::MockHost;
use raisin_sdk::Result;
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
struct Input {
    name: String,
}

#[derive(Serialize, Deserialize)]
struct Output {
    greeting: String,
}

#[raisin_sdk::handler]
fn greet(input: Input) -> Result<Output> {
    raisin_sdk::log::info(format!("greeting {}", input.name));
    Ok(Output {
        greeting: format!("Hello, {}!", input.name),
    })
}

#[raisin_sdk::handler(name = "shout")]
fn shout(input: Input) -> Result<Output> {
    Ok(Output {
        greeting: format!("HELLO, {}!", input.name.to_uppercase()),
    })
}

#[raisin_sdk::handler(name = "boom")]
fn boom(_input: serde_json::Value) -> Result<Output> {
    Err(raisin_sdk::Error::host("deliberate failure"))
}

raisin_sdk::export!(greet, shout, boom);

#[test]
fn an_unnamed_handler_registers_as_default() {
    assert_eq!(
        raisin_sdk::registered_names(RAISIN_HANDLERS),
        vec!["default", "shout", "boom"]
    );
}

#[test]
fn every_registered_handler_is_invocable_by_name() {
    let (out, _) = raisin_sdk::testing::with_mock(MockHost::new(), || {
        raisin_dispatch("default", r#"{"name":"Ada"}"#).expect("default runs")
    });
    let parsed: Output = serde_json::from_str(&out).expect("json out");
    assert_eq!(parsed.greeting, "Hello, Ada!");

    let out = raisin_dispatch("shout", r#"{"name":"Ada"}"#).expect("shout runs");
    let parsed: Output = serde_json::from_str(&out).expect("json out");
    assert_eq!(parsed.greeting, "HELLO, ADA!");
}

#[test]
fn an_unknown_name_lists_the_registered_set() {
    let err = raisin_dispatch("nope", "{}").expect_err("unknown handler");
    assert!(err.contains("unknown handler 'nope'"), "{err}");
    for name in ["default", "shout", "boom"] {
        assert!(err.contains(name), "{err} should name {name}");
    }
}

#[test]
fn a_handler_error_becomes_the_err_message() {
    let err = raisin_dispatch("boom", "{}").expect_err("handler failed");
    assert_eq!(err, "deliberate failure");
}

#[test]
fn undecodable_input_fails_before_the_handler_runs() {
    let err = raisin_dispatch("default", r#"{"nom":"Ada"}"#).expect_err("bad input");
    assert!(err.starts_with("could not decode handler input"), "{err}");
}

#[test]
fn a_handler_logs_through_the_host() {
    let (_, mock) = raisin_sdk::testing::with_mock(MockHost::new(), || {
        raisin_dispatch("default", r#"{"name":"Grace"}"#).expect("runs")
    });
    assert_eq!(
        mock.logs(),
        &[(raisin_sdk::LogLevel::Info, "greeting Grace".to_string())]
    );
}
