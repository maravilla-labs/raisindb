//! Decoders for the host gateway's `Ok` payload.
//!
//! The payload is `InvokeResult::to_json_string()`: a JSON value for
//! Json/OptionalJson/JsonArray, a bare `true`/`false`, a bare number, a JSON
//! string, `null` for an absent optional, and literal `true` for `Void`.
//!
//! Defensive rule, applied in ONE place ([`envelope_error`]) so every decoder
//! inherits it: an `Ok` body that parses to `{"error": true, ...}` is an error.
//! A few host methods answer with that envelope rather than an `Err`, and a
//! guest that treated it as data would carry a failure forward as success.

use crate::error::{Error, Result};
use serde::de::DeserializeOwned;
use serde_json::Value;

/// Parse `raw`, rejecting an `{"error": true, ...}` envelope.
fn parse(raw: &str) -> Result<Value> {
    let value: Value = serde_json::from_str(raw)
        .map_err(|e| Error::Wire(format!("host reply is not JSON ({e}): {raw}")))?;
    match envelope_error(&value) {
        Some(message) => Err(Error::Host(message)),
        None => Ok(value),
    }
}

/// The message inside an `{"error": true, ...}` envelope, if `value` is one.
pub fn envelope_error(value: &Value) -> Option<String> {
    let object = value.as_object()?;
    if object.get("error") != Some(&Value::Bool(true)) {
        return None;
    }
    let message = object
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("host reported an error");
    Some(message.to_string())
}

/// A JSON value.
pub fn decode_json(raw: &str) -> Result<Value> {
    parse(raw)
}

/// A JSON value, deserialised into `T`.
pub fn decode_json_as<T: DeserializeOwned>(raw: &str) -> Result<T> {
    Ok(serde_json::from_value(parse(raw)?)?)
}

/// A JSON value, or `None` for `null`.
pub fn decode_optional_json(raw: &str) -> Result<Option<Value>> {
    Ok(match parse(raw)? {
        Value::Null => None,
        value => Some(value),
    })
}

/// A JSON value deserialised into `T`, or `None` for `null`.
pub fn decode_optional_json_as<T: DeserializeOwned>(raw: &str) -> Result<Option<T>> {
    Ok(match parse(raw)? {
        Value::Null => None,
        value => Some(serde_json::from_value(value)?),
    })
}

/// A JSON array.
pub fn decode_json_array(raw: &str) -> Result<Vec<Value>> {
    match parse(raw)? {
        Value::Array(items) => Ok(items),
        Value::Null => Ok(Vec::new()),
        other => Err(mismatch("an array", &other)),
    }
}

/// A JSON array whose elements deserialise into `T`.
pub fn decode_json_array_as<T: DeserializeOwned>(raw: &str) -> Result<Vec<T>> {
    decode_json_array(raw)?
        .into_iter()
        .map(|item| Ok(serde_json::from_value(item)?))
        .collect()
}

/// A boolean.
pub fn decode_bool(raw: &str) -> Result<bool> {
    match parse(raw)? {
        Value::Bool(b) => Ok(b),
        other => Err(mismatch("a boolean", &other)),
    }
}

/// A 64-bit integer.
pub fn decode_i64(raw: &str) -> Result<i64> {
    match parse(raw)? {
        Value::Number(n) => n
            .as_i64()
            .ok_or_else(|| Error::Wire(format!("expected an integer, host sent {n}"))),
        other => Err(mismatch("an integer", &other)),
    }
}

/// A string. `null` is an error: a method declared to return a string that
/// answers `null` has failed to produce one, and an empty string would hide it.
pub fn decode_string(raw: &str) -> Result<String> {
    match parse(raw)? {
        Value::String(s) => Ok(s),
        other => Err(mismatch("a string", &other)),
    }
}

/// Success with no value. The wire form is literal `true`.
pub fn decode_void(raw: &str) -> Result<()> {
    parse(raw)?;
    Ok(())
}

/// Uniform "expected X, host sent Y" message.
fn mismatch(expected: &str, got: &Value) -> Error {
    Error::Wire(format!("expected {expected}, host sent {got}"))
}
