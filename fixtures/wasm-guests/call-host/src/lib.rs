//! `call_host.wasm` — exercises the generic host gateway three ways: a method
//! with no arguments, a method with arguments, and a method that does not
//! exist (which must be an `Err`, not a trap).

wit_bindgen::generate!({
    path: "../../../crates/raisin-functions/wit",
    world: "function",
});

use raisin::function::host;

struct Component;

/// Minimal JSON string escaping — enough for an error message.
fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

impl Guest for Component {
    fn handler(_name: String, _input: String) -> Result<String, String> {
        let context = host::call("context_get", "[]")?;
        let http = host::call(
            "http_request",
            r#"["GET","https://example.com/thing",{"headers":{"x-from":"wasm"}}]"#,
        )?;
        let unknown = match host::call("no_such_method", "[]") {
            Ok(v) => format!("unexpectedly ok: {v}"),
            Err(e) => e,
        };
        Ok(format!(
            r#"{{"context":{context},"http":{http},"unknown_error":{}}}"#,
            quote(&unknown)
        ))
    }
}

export!(Component);
