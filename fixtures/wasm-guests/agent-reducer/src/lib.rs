//! `agent_reducer.wasm` — a scripted AgentRun domain reducer for the
//! `agent_reducer` tests of the WebAssembly reducer adapter.
//!
//! Handler `reduce` answers a `raisin.agent-run.reducer/1` request from a small
//! table keyed by `event.data.mode`:
//!
//! - (none)      a valid response: `state_rev + 1` and one `call_tool` effect;
//! - `two_ops`   an R4 violation (two operation effects);
//! - `call_host` tries a host call — which the reducer adapter DENIES — and
//!   reports what it got as a refusal;
//! - `trap`      traps;
//! - `ambient`   reads the wall clock, the monotonic clock and a fresh
//!   `RandomState` hash, and returns them in its state — the adapter must make
//!   all three constant.
//!
//! No raisin-sdk dependency on purpose, like every guest here.

wit_bindgen::generate!({
    path: "../../../crates/raisin-functions/wit",
    world: "function",
});

use raisin::function::host;
use serde_json::{json, Value};

struct Component;

fn effect(rev: u64, i: usize) -> Value {
    json!({
        "effect_id": format!("{rev}:{i}"),
        "kind": "call_tool",
        "tool": "/lib/demo/read",
        "args": { "q": i },
        "mutating": false,
        "replay_safe": true
    })
}

impl Guest for Component {
    fn handler(name: String, input: String) -> Result<String, String> {
        if name != "reduce" {
            return Err(format!("unknown handler '{name}'; registered: reduce"));
        }
        let req: Value = serde_json::from_str(&input).map_err(|e| e.to_string())?;
        let rev = req["state_rev"].as_u64().unwrap_or(0) + 1;
        let contract = req["contract"].clone();
        let seq = req["event"]["seq"].clone();
        let mode = req["event"]["data"]["mode"].as_str().unwrap_or("").to_owned();
        let response = match mode.as_str() {
            "two_ops" => json!({
                "contract": contract, "state": { "last_event_seq": seq }, "state_rev": rev,
                "effects": [effect(rev, 0), effect(rev, 1)]
            }),
            "call_host" => {
                let got = host::call("context_get", "[]");
                let message = match got {
                    Ok(v) => format!("unexpectedly allowed: {v}"),
                    Err(e) => e,
                };
                json!({
                    "contract": contract, "state": {}, "state_rev": req["state_rev"],
                    "effects": [], "refused": { "code": "host_denied", "message": message }
                })
            }
            "trap" => unreachable!("scripted trap"),
            "ambient" => {
                use std::hash::{BuildHasher, Hasher};
                let wall = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos() as u64)
                    .unwrap_or(u64::MAX);
                let start = std::time::Instant::now();
                let mono = start.elapsed().as_nanos() as u64;
                let mut h = std::collections::hash_map::RandomState::new().build_hasher();
                h.write_u64(42);
                json!({
                    "contract": contract,
                    "state": { "last_event_seq": seq, "wall_ns": wall, "mono_ns": mono,
                               "hash": h.finish().to_string() },
                    "state_rev": rev, "effects": []
                })
            }
            _ => json!({
                "contract": contract, "state": { "last_event_seq": seq }, "state_rev": rev,
                "effects": [effect(rev, 0)]
            }),
        };
        Ok(response.to_string())
    }
}

export!(Component);
