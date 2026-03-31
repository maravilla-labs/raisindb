//! Evaluation context for variable bindings

use crate::value::Value;
use std::collections::HashMap;

/// Context for expression evaluation
///
/// The context holds variable bindings that can be referenced in expressions.
/// Typically contains objects like "input" and "context".
#[derive(Debug, Clone, Default)]
pub struct EvalContext {
    /// Root variables (e.g., "input", "context")
    variables: HashMap<String, Value>,
}

impl EvalContext {
    /// Create a new empty context
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a context with a single variable
    pub fn with_var(name: impl Into<String>, value: Value) -> Self {
        let mut ctx = Self::new();
        ctx.set(name, value);
        ctx
    }

    /// Set a variable in the context
    pub fn set(&mut self, name: impl Into<String>, value: Value) -> &mut Self {
        self.variables.insert(name.into(), value);
        self
    }

    /// Get a variable from the context
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.variables.get(name)
    }

    /// Check if a variable exists in the context
    pub fn contains(&self, name: &str) -> bool {
        self.variables.contains_key(name)
    }

    /// Get all variable names
    pub fn variables(&self) -> impl Iterator<Item = &String> {
        self.variables.keys()
    }

    /// Create context from a JSON object
    ///
    /// Each top-level key becomes a variable in the context.
    pub fn from_json(json: serde_json::Value) -> Result<Self, &'static str> {
        match json {
            serde_json::Value::Object(obj) => {
                let mut ctx = Self::new();
                for (key, value) in obj {
                    ctx.set(key, Value::from_json(value));
                }
                Ok(ctx)
            }
            _ => Err("Context must be a JSON object"),
        }
    }

    /// Convert context to a JSON object
    pub fn to_json(&self) -> serde_json::Value {
        let obj: serde_json::Map<String, serde_json::Value> = self
            .variables
            .iter()
            .map(|(k, v)| (k.clone(), v.clone().to_json()))
            .collect();
        serde_json::Value::Object(obj)
    }

    /// Merge another context into this one
    ///
    /// Variables from `other` will overwrite variables with the same name.
    pub fn merge(&mut self, other: &EvalContext) -> &mut Self {
        for (key, value) in &other.variables {
            self.variables.insert(key.clone(), value.clone());
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_new_context() {
        let ctx = EvalContext::new();
        assert!(ctx.get("foo").is_none());
    }

    #[test]
    fn test_set_and_get() {
        let mut ctx = EvalContext::new();
        ctx.set("x", Value::Integer(42));
        assert_eq!(ctx.get("x"), Some(&Value::Integer(42)));
    }

    #[test]
    fn test_with_var() {
        let ctx = EvalContext::with_var("x", Value::Integer(42));
        assert_eq!(ctx.get("x"), Some(&Value::Integer(42)));
    }

    #[test]
    fn test_from_json() {
        let json = serde_json::json!({
            "input": {
                "value": 42,
                "name": "test"
            },
            "context": {
                "userId": "123"
            }
        });

        let ctx = EvalContext::from_json(json).unwrap();
        assert!(ctx.contains("input"));
        assert!(ctx.contains("context"));

        if let Some(Value::Object(input)) = ctx.get("input") {
            assert_eq!(input.get("value"), Some(&Value::Integer(42)));
        } else {
            panic!("Expected input to be an object");
        }
    }

    #[test]
    fn test_from_json_invalid() {
        let json = serde_json::json!([1, 2, 3]);
        assert!(EvalContext::from_json(json).is_err());
    }

    #[test]
    fn test_to_json() {
        let mut ctx = EvalContext::new();
        ctx.set("x", Value::Integer(42));
        ctx.set("name", Value::String("test".to_string()));

        let json = ctx.to_json();
        assert_eq!(json["x"], 42);
        assert_eq!(json["name"], "test");
    }

    #[test]
    fn test_merge() {
        let mut ctx1 = EvalContext::new();
        ctx1.set("a", Value::Integer(1));
        ctx1.set("b", Value::Integer(2));

        let mut ctx2 = EvalContext::new();
        ctx2.set("b", Value::Integer(3));
        ctx2.set("c", Value::Integer(4));

        ctx1.merge(&ctx2);

        assert_eq!(ctx1.get("a"), Some(&Value::Integer(1)));
        assert_eq!(ctx1.get("b"), Some(&Value::Integer(3))); // Overwritten
        assert_eq!(ctx1.get("c"), Some(&Value::Integer(4)));
    }
}
