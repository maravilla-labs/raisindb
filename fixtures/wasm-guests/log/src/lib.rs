//! `log.wasm` — logs through the host import at two levels and writes one line
//! to each of stdout and stderr, so the runtime's pipe draining is observable.

wit_bindgen::generate!({
    path: "../../../crates/raisin-functions/wit",
    world: "function",
});

use raisin::function::host;

struct Component;

impl Guest for Component {
    fn handler(_name: String, _input: String) -> Result<String, String> {
        host::log(host::LogLevel::Info, "info from wasm");
        host::log(host::LogLevel::Error, "error from wasm");
        println!("stdout from wasm");
        eprintln!("stderr from wasm");
        Ok(r#"{"logged":true}"#.to_string())
    }
}

export!(Component);
