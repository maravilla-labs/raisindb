//! Path method implementations
//!
//! These methods are used for working with hierarchical paths (e.g., "/content/blog/post1")

use crate::error::EvalError;
use crate::value::Value;

/// Evaluate `parent()` method
/// Get parent path by going N levels up (default: 1)
pub fn eval_parent(val: &Value, levels: Option<&Value>) -> Result<Value, EvalError> {
    let path = val
        .as_str()
        .ok_or_else(|| EvalError::type_error("parent", "string", val.type_name()))?;
    let levels = match levels {
        Some(v) => v
            .as_integer()
            .ok_or_else(|| EvalError::type_error("parent", "integer", v.type_name()))?
            as usize,
        None => 1,
    };

    Ok(Value::String(get_parent_at_level(path, levels)))
}

/// Evaluate `ancestor()` method
/// Get ancestor at specific absolute depth from root
pub fn eval_ancestor(val: &Value, depth: &Value) -> Result<Value, EvalError> {
    let path = val
        .as_str()
        .ok_or_else(|| EvalError::type_error("ancestor", "string", val.type_name()))?;
    let depth = depth
        .as_integer()
        .ok_or_else(|| EvalError::type_error("ancestor", "integer", depth.type_name()))?
        as i32;

    Ok(Value::String(get_ancestor(path, depth)))
}

/// Evaluate `ancestorOf()` method
/// Check if this path is an ancestor of another path
pub fn eval_ancestor_of(val: &Value, other: &Value) -> Result<Value, EvalError> {
    let path = val
        .as_str()
        .ok_or_else(|| EvalError::type_error("ancestorOf", "string", val.type_name()))?;
    let other_path = other
        .as_str()
        .ok_or_else(|| EvalError::type_error("ancestorOf", "string", other.type_name()))?;

    // A path is an ancestor if other_path starts with path + "/"
    let prefix = if path.ends_with('/') {
        path.to_string()
    } else {
        format!("{}/", path)
    };
    let is_ancestor = other_path != path && other_path.starts_with(&prefix);
    Ok(Value::Boolean(is_ancestor))
}

/// Evaluate `descendantOf()` method
/// Check if this path is a descendant of another path
pub fn eval_descendant_of(val: &Value, parent: &Value) -> Result<Value, EvalError> {
    let path = val
        .as_str()
        .ok_or_else(|| EvalError::type_error("descendantOf", "string", val.type_name()))?;
    let parent_path = parent
        .as_str()
        .ok_or_else(|| EvalError::type_error("descendantOf", "string", parent.type_name()))?;

    // A path is a descendant if it starts with parent + "/"
    let prefix = if parent_path.ends_with('/') {
        parent_path.to_string()
    } else {
        format!("{}/", parent_path)
    };
    let is_descendant = path != parent_path && path.starts_with(&prefix);
    Ok(Value::Boolean(is_descendant))
}

/// Evaluate `childOf()` method
/// Check if this path is a direct child of another path
pub fn eval_child_of(val: &Value, parent: &Value) -> Result<Value, EvalError> {
    let path = val
        .as_str()
        .ok_or_else(|| EvalError::type_error("childOf", "string", val.type_name()))?;
    let parent_path = parent
        .as_str()
        .ok_or_else(|| EvalError::type_error("childOf", "string", parent.type_name()))?;

    // A path is a direct child if it's exactly one level deeper
    let prefix = if parent_path.ends_with('/') {
        parent_path.to_string()
    } else {
        format!("{}/", parent_path)
    };

    if path == parent_path || !path.starts_with(&prefix) {
        return Ok(Value::Boolean(false));
    }

    // Check that there are no more slashes after the prefix
    let remainder = &path[prefix.len()..];
    Ok(Value::Boolean(!remainder.contains('/')))
}

/// Evaluate `depth()` method
/// Get hierarchy depth of path
pub fn eval_depth(val: &Value) -> Result<Value, EvalError> {
    let path = val
        .as_str()
        .ok_or_else(|| EvalError::type_error("depth", "string", val.type_name()))?;
    let depth = path.split('/').filter(|s| !s.is_empty()).count();
    Ok(Value::Integer(depth as i64))
}

// === Path helper functions (ported from raisin-sql-execution) ===

/// Get ancestor at specific absolute depth from root
fn get_ancestor(path: &str, depth: i32) -> String {
    if depth <= 0 {
        return String::new();
    }

    path.char_indices()
        .filter(|(_, ch)| *ch == '/')
        .nth(depth as usize)
        .map_or_else(
            || {
                let segment_count = path.split('/').filter(|s| !s.is_empty()).count();
                if segment_count == depth as usize {
                    path.to_string()
                } else {
                    String::new()
                }
            },
            |(idx, _)| path[..idx].to_string(),
        )
}

/// Get parent path by going N levels up
fn get_parent_at_level(path: &str, levels: usize) -> String {
    if path == "/" {
        return String::new();
    }

    path.char_indices()
        .rev()
        .filter(|(_, ch)| *ch == '/')
        .nth(levels.saturating_sub(1))
        .map_or_else(String::new, |(idx, _)| {
            if idx == 0 {
                "/".to_string()
            } else {
                path[..idx].to_string()
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parent() {
        let path = Value::String("/content/blog/post1".to_string());
        assert_eq!(
            eval_parent(&path, None).unwrap(),
            Value::String("/content/blog".to_string())
        );
        assert_eq!(
            eval_parent(&path, Some(&Value::Integer(2))).unwrap(),
            Value::String("/content".to_string())
        );
    }

    #[test]
    fn test_depth() {
        let path = Value::String("/content/blog/post1".to_string());
        assert_eq!(eval_depth(&path).unwrap(), Value::Integer(3));

        let root = Value::String("/".to_string());
        assert_eq!(eval_depth(&root).unwrap(), Value::Integer(0));
    }

    #[test]
    fn test_descendant_of() {
        let path = Value::String("/content/blog/post1".to_string());
        assert_eq!(
            eval_descendant_of(&path, &Value::String("/content".to_string())).unwrap(),
            Value::Boolean(true)
        );
        assert_eq!(
            eval_descendant_of(&path, &Value::String("/other".to_string())).unwrap(),
            Value::Boolean(false)
        );
        // Not a descendant of itself
        assert_eq!(
            eval_descendant_of(&path, &Value::String("/content/blog/post1".to_string())).unwrap(),
            Value::Boolean(false)
        );
    }

    #[test]
    fn test_ancestor_of() {
        let path = Value::String("/content".to_string());
        assert_eq!(
            eval_ancestor_of(&path, &Value::String("/content/blog".to_string())).unwrap(),
            Value::Boolean(true)
        );
        assert_eq!(
            eval_ancestor_of(&path, &Value::String("/other/path".to_string())).unwrap(),
            Value::Boolean(false)
        );
    }

    #[test]
    fn test_child_of() {
        let child = Value::String("/content/blog".to_string());
        assert_eq!(
            eval_child_of(&child, &Value::String("/content".to_string())).unwrap(),
            Value::Boolean(true)
        );

        let grandchild = Value::String("/content/blog/post1".to_string());
        assert_eq!(
            eval_child_of(&grandchild, &Value::String("/content".to_string())).unwrap(),
            Value::Boolean(false) // Not a direct child
        );
    }

    #[test]
    fn test_ancestor() {
        let path = Value::String("/content/blog/post1".to_string());
        assert_eq!(
            eval_ancestor(&path, &Value::Integer(1)).unwrap(),
            Value::String("/content".to_string())
        );
        assert_eq!(
            eval_ancestor(&path, &Value::Integer(2)).unwrap(),
            Value::String("/content/blog".to_string())
        );
    }
}
