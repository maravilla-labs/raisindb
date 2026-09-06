//! Implementation details the macros expand into. Not a stable surface.

use serde::de::DeserializeOwned;
use serde::Serialize;

/// The JSON envelope `#[handler]` wraps a typed handler function in.
///
/// An empty input is read as `null`, so `fn(())` and `fn(Option<T>)` handlers
/// work when a caller sends no body at all.
pub fn invoke_typed<I, O, E, F>(input: &str, f: F) -> Result<String, String>
where
    I: DeserializeOwned,
    O: Serialize,
    E: core::fmt::Display,
    F: FnOnce(I) -> Result<O, E>,
{
    let body = if input.trim().is_empty() {
        "null"
    } else {
        input
    };
    let parsed: I = serde_json::from_str(body)
        .map_err(|e| format!("could not decode handler input ({e}): {body}"))?;
    let output = f(parsed).map_err(|e| e.to_string())?;
    serde_json::to_string(&output).map_err(|e| format!("could not encode handler output: {e}"))
}
