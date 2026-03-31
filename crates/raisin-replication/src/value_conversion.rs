//! Utilities for converting between serde_json::Value and rmpv::Value
//!
//! This module provides bidirectional conversion between JSON and MessagePack
//! value representations, allowing code that works with JSON to interoperate
//! with the MessagePack-based replication system.

use rmpv::Value as MsgPackValue;
use serde_json::Value as JsonValue;

/// Convert an rmpv::Value to serde_json::Value
///
/// This function recursively converts MessagePack value types to their JSON equivalents:
/// - MessagePack Nil → JSON null
/// - MessagePack Boolean → JSON bool
/// - MessagePack Integer → JSON number
/// - MessagePack F32/F64 → JSON number
/// - MessagePack String → JSON string
/// - MessagePack Binary → JSON string (base64 encoded)
/// - MessagePack Array → JSON array
/// - MessagePack Map → JSON object
/// - MessagePack Ext → JSON null (not representable in JSON)
pub fn msgpack_to_json(msgpack: &MsgPackValue) -> JsonValue {
    match msgpack {
        MsgPackValue::Nil => JsonValue::Null,
        MsgPackValue::Boolean(b) => JsonValue::Bool(*b),
        MsgPackValue::Integer(i) => {
            if i.is_u64() {
                JsonValue::Number(i.as_u64().unwrap().into())
            } else if i.is_i64() {
                let num = i.as_i64().unwrap();
                JsonValue::Number(num.into())
            } else {
                // Fallback to null if we can't represent the integer
                JsonValue::Null
            }
        }
        MsgPackValue::F32(f) => serde_json::Number::from_f64(*f as f64)
            .map(JsonValue::Number)
            .unwrap_or(JsonValue::Null),
        MsgPackValue::F64(f) => serde_json::Number::from_f64(*f)
            .map(JsonValue::Number)
            .unwrap_or(JsonValue::Null),
        MsgPackValue::String(s) => {
            // Convert Utf8StringRef to String
            JsonValue::String(s.as_str().unwrap_or("").to_string())
        }
        MsgPackValue::Binary(b) => {
            // Encode binary as base64 string
            use base64::Engine;
            JsonValue::String(base64::engine::general_purpose::STANDARD.encode(b))
        }
        MsgPackValue::Array(arr) => {
            let values: Vec<JsonValue> = arr.iter().map(msgpack_to_json).collect();
            JsonValue::Array(values)
        }
        MsgPackValue::Map(map) => {
            let mut obj = serde_json::Map::new();
            for (k, v) in map {
                // Keys should be strings in JSON objects
                if let MsgPackValue::String(key_str) = k {
                    obj.insert(
                        key_str.as_str().unwrap_or("").to_string(),
                        msgpack_to_json(v),
                    );
                }
            }
            JsonValue::Object(obj)
        }
        MsgPackValue::Ext(_, _) => {
            // Extension types don't have a JSON equivalent
            JsonValue::Null
        }
    }
}

/// Convert a serde_json::Value to rmpv::Value
///
/// This function recursively converts JSON value types to their MessagePack equivalents:
/// - JSON null → MessagePack Nil
/// - JSON bool → MessagePack Boolean
/// - JSON number → MessagePack Integer or F64
/// - JSON string → MessagePack String
/// - JSON array → MessagePack Array
/// - JSON object → MessagePack Map
pub fn json_to_msgpack(json: &JsonValue) -> MsgPackValue {
    match json {
        JsonValue::Null => MsgPackValue::Nil,
        JsonValue::Bool(b) => MsgPackValue::Boolean(*b),
        JsonValue::Number(n) => {
            if let Some(i) = n.as_i64() {
                MsgPackValue::Integer(i.into())
            } else if let Some(u) = n.as_u64() {
                MsgPackValue::Integer(u.into())
            } else if let Some(f) = n.as_f64() {
                MsgPackValue::F64(f)
            } else {
                MsgPackValue::Nil
            }
        }
        JsonValue::String(s) => MsgPackValue::String(s.as_str().into()),
        JsonValue::Array(arr) => {
            let values: Vec<MsgPackValue> = arr.iter().map(json_to_msgpack).collect();
            MsgPackValue::Array(values)
        }
        JsonValue::Object(obj) => {
            let mut map = Vec::new();
            for (k, v) in obj {
                map.push((MsgPackValue::String(k.as_str().into()), json_to_msgpack(v)));
            }
            MsgPackValue::Map(map)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_msgpack_to_json_null() {
        let msgpack = MsgPackValue::Nil;
        let json = msgpack_to_json(&msgpack);
        assert_eq!(json, JsonValue::Null);
    }

    #[test]
    fn test_msgpack_to_json_bool() {
        let msgpack = MsgPackValue::Boolean(true);
        let json = msgpack_to_json(&msgpack);
        assert_eq!(json, JsonValue::Bool(true));
    }

    #[test]
    fn test_msgpack_to_json_integer() {
        let msgpack = MsgPackValue::Integer(rmpv::Integer::from(42));
        let json = msgpack_to_json(&msgpack);
        assert_eq!(json, JsonValue::Number(42.into()));
    }

    #[test]
    fn test_msgpack_to_json_string() {
        let msgpack = MsgPackValue::String("hello".into());
        let json = msgpack_to_json(&msgpack);
        assert_eq!(json, JsonValue::String("hello".to_string()));
    }
}
