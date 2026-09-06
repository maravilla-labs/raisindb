//! `echo.wasm` — proves ONE artifact carries N handlers.
//!
//! Registers `default` and `reverse`. An unknown name comes back as an `Err`
//! naming the registered set, which is the guest's job: the host never
//! validates a handler name against a list.

wit_bindgen::generate!({
    path: "../../../crates/raisin-functions/wit",
    world: "function",
});

use raisin::function::host;

/// The names this component answers to. Named in the error for a miss.
const REGISTERED: &[&str] = &["default", "reverse"];

struct Component;

impl Guest for Component {
    fn handler(name: String, input: String) -> Result<String, String> {
        match name.as_str() {
            "default" => Ok(format!(
                r#"{{"handler":"default","echo":{input},"abi":"{abi}"}}"#,
                abi = host::abi_version()
            )),
            "reverse" => Ok(format!(r#"{{"handler":"reverse","echo":{input}}}"#)),
            other => Err(format!(
                "unknown handler '{other}'; registered: {}",
                REGISTERED.join(", ")
            )),
        }
    }
}

export!(Component);
