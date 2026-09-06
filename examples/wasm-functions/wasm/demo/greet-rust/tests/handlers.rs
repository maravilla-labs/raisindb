//! Native tests — no server, no wasm runtime. `cargo test` runs these; the same
//! code compiled for `wasm32-wasip2` is what ships.

use greet_rust::{raisin_dispatch, Output};
use raisin_sdk::testing::{with_mock, MockHost};

fn host() -> MockHost {
    MockHost::new().expect_any(
        "nodes_getChildren",
        Ok(r#"[{"path":"/people/ada"},{"path":"/people/grace"}]"#.to_string()),
    )
}

#[test]
fn the_default_handler_greets() {
    let (out, mock) = with_mock(host(), || {
        raisin_dispatch("default", r#"{"name":"Ada"}"#).expect("runs")
    });
    let out: Output = serde_json::from_str(&out).expect("json");
    assert_eq!(out.greeting, "Hello, Ada!");
    assert_eq!(out.people, 2);
    assert_eq!(out.handler, "default");
    assert_eq!(mock.logs().len(), 1);
}

#[test]
fn the_shout_handler_is_the_same_artifact() {
    let (out, _) = with_mock(host(), || {
        raisin_dispatch("shout", r#"{"name":"Ada"}"#).expect("runs")
    });
    let out: Output = serde_json::from_str(&out).expect("json");
    assert_eq!(out.greeting, "HELLO, ADA!");
    assert_eq!(out.handler, "shout");
}

#[test]
fn an_unknown_handler_names_the_registered_set() {
    let err = raisin_dispatch("nope", "{}").expect_err("unknown");
    assert!(err.contains("default"), "{err}");
    assert!(err.contains("shout"), "{err}");
}
