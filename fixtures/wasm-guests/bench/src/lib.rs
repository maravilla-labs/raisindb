//! `bench.wasm` — the WebAssembly arm of the cross-runtime benchmark.
//!
//! Four handlers, each mirrored one-for-one by a JavaScript and a Starlark
//! source in `runtime/bench_runtimes.rs`. They exist to decompose the cost of
//! an invocation rather than produce a single number:
//!
//! * `noop`      — dispatch floor: parse/compile, instantiate, call, return.
//! * `compute`   — guest execution speed, no host calls at all.
//! * `hostcalls` — marshalling cost, `n` identical gateway round-trips.
//! * `realistic` — one node read plus one SQL query, results aggregated:
//!                 the shape of an ordinary RaisinDB function.
//!
//! Every handler returns JSON so the harness can assert all three runtimes
//! computed the SAME answer — a benchmark whose arms disagree is timing
//! different work.

wit_bindgen::generate!({
    path: "../../../crates/raisin-functions/wit",
    world: "function",
});

use raisin::function::host;
use serde_json::Value;

struct Component;

/// The parsed input. Workspace and path come from it rather than being baked
/// in, so the same artifact serves both the mock-host benchmark and the
/// end-to-end one against a live server holding real nodes.
fn parsed(input: &str) -> Value {
    serde_json::from_str::<Value>(input).unwrap_or(Value::Null)
}

fn n_of(input: &Value) -> u64 {
    input.get("n").and_then(|n| n.as_u64()).unwrap_or(1)
}

fn str_of<'a>(input: &'a Value, key: &str, default: &'a str) -> &'a str {
    input.get(key).and_then(|v| v.as_str()).unwrap_or(default)
}

fn get_children(input: &Value) -> Result<Vec<Value>, String> {
    let args = serde_json::json!([
        str_of(input, "workspace", "content"),
        str_of(input, "path", "/pages"),
        input.get("limit").and_then(|v| v.as_u64()).unwrap_or(3),
    ]);
    let raw = host::call("nodes_getChildren", &args.to_string())?;
    match serde_json::from_str::<Value>(&raw) {
        Ok(Value::Array(items)) => Ok(items),
        Ok(other) => Err(format!("expected an array of children, got {other}")),
        Err(e) => Err(format!("children were not JSON: {e}")),
    }
}

impl Guest for Component {
    fn handler(name: String, input: String) -> Result<String, String> {
        let input = parsed(&input);
        match name.as_str() {
            "noop" => Ok("{}".to_string()),

            "compute" => {
                // Wrapping so the arms agree exactly: JS numbers are f64 and
                // Starlark integers are arbitrary precision, so the sum is
                // taken modulo 2^32 everywhere rather than trusting three
                // different numeric towers to coincide.
                let mut sum: u32 = 0;
                for i in 0..n_of(&input) {
                    sum = sum.wrapping_add((i % 1000) as u32);
                }
                Ok(format!(r#"{{"sum":{sum}}}"#))
            }

            "hostcalls" => {
                let n = n_of(&input);
                let mut children = 0u64;
                for _ in 0..n {
                    children += get_children(&input)?.len() as u64;
                }
                Ok(format!(r#"{{"calls":{n},"children":{children}}}"#))
            }

            "realistic" => {
                // One node read, one SQL query, then aggregate both — the
                // shape of an ordinary function. Only the ROW COUNT is
                // reported: how a row is encoded differs between the mock host
                // and a real server, and an arm-comparison must not depend on
                // that.
                let node_type = str_of(&input, "node_type", "raisin:Page");
                let children = get_children(&input)?;
                let pages = children
                    .iter()
                    .filter(|c| c.get("node_type").and_then(|t| t.as_str()) == Some(node_type))
                    .count();

                let sql = serde_json::json!([
                    str_of(
                        &input,
                        "sql",
                        "SELECT id, name FROM 'content' WHERE node_type = $1"
                    ),
                    [node_type],
                ]);
                // `sql_query` answers a JSON ARRAY of row objects.
                let raw = host::call("sql_query", &sql.to_string())?;
                let rows = match serde_json::from_str::<Value>(&raw) {
                    Ok(Value::Array(rows)) => rows.len(),
                    Ok(other) => return Err(format!("expected an array of rows, got {other}")),
                    Err(e) => return Err(format!("sql result was not JSON: {e}")),
                };

                Ok(format!(r#"{{"pages":{pages},"rows":{rows}}}"#))
            }

            other => Err(format!(
                "unknown handler {other:?}; registered: noop, compute, hostcalls, realistic"
            )),
        }
    }
}

export!(Component);
