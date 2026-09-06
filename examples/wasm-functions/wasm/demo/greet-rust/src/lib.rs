//! `greet-rust` — one artifact, two handlers.
//!
//! `default` greets; `shout` greets in capitals. Both call one host method and
//! log one line, so an end-to-end run exercises the gateway, the logger and the
//! name routing at once.
//!
//! Build: `cargo build --release --target wasm32-wasip2`, then copy
//! `target/wasm32-wasip2/release/greet_rust.wasm` to the Function node's
//! `main.wasm` (`raisindb function build` does both).

use raisin_sdk::prelude::*;
use serde::{Deserialize, Serialize};

/// What either handler accepts.
#[derive(Deserialize)]
pub struct Input {
    /// Who to greet.
    pub name: String,
}

/// What either handler answers.
#[derive(Serialize, Deserialize)]
pub struct Output {
    /// The greeting.
    pub greeting: String,
    /// Children of `/people` in the `content` workspace.
    pub people: usize,
    /// Which registered handler answered.
    pub handler: String,
}

/// Count `/people` in the `content` workspace.
///
/// A missing workspace is not a failure of the demo — it logs and answers 0, so
/// the function runs on a repository that has no `content/people` yet.
fn people_count() -> usize {
    match raisin_sdk::nodes::get_children("content", "/people", None) {
        Ok(children) => children.len(),
        Err(e) => {
            raisin_sdk::log::warn(format!("could not count /people: {e}"));
            0
        }
    }
}

/// Greet by name. Registered as `default` — the handler a bare `main.wasm`
/// entry file selects.
#[raisin_sdk::handler]
pub fn greet(input: Input) -> Result<Output> {
    raisin_sdk::log::info(format!("greeting {}", input.name));
    Ok(Output {
        greeting: format!("Hello, {}!", input.name),
        people: people_count(),
        handler: "default".to_string(),
    })
}

/// Greet in capitals. Registered as `shout`, selected by
/// `entry_file: ../greet-rust/main.wasm:shout` on a second Function node.
#[raisin_sdk::handler(name = "shout")]
pub fn shout(input: Input) -> Result<Output> {
    raisin_sdk::log::info(format!("SHOUTING AT {}", input.name.to_uppercase()));
    Ok(Output {
        greeting: format!("HELLO, {}!", input.name.to_uppercase()),
        people: people_count(),
        handler: "shout".to_string(),
    })
}

raisin_sdk::export!(greet, shout);
