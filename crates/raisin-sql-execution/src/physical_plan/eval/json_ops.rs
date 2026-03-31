//! JSON operation helpers

/// Check if JSON object contains pattern
///
/// Implements PostgreSQL @> (contains) operator semantics:
/// - For objects: all key-value pairs in pattern must exist in object
/// - For arrays: all elements in pattern must exist in array
/// - Recursive matching for nested structures
pub(super) fn json_contains(object: &serde_json::Value, pattern: &serde_json::Value) -> bool {
    match (object, pattern) {
        (serde_json::Value::Object(obj), serde_json::Value::Object(pat)) => {
            for (key, pat_val) in pat {
                match obj.get(key) {
                    Some(obj_val) if json_contains(obj_val, pat_val) => continue,
                    _ => return false,
                }
            }
            true
        }
        (serde_json::Value::Array(arr), serde_json::Value::Array(pat)) => {
            pat.iter().all(|p| arr.iter().any(|a| json_contains(a, p)))
        }
        (a, b) => a == b,
    }
}
