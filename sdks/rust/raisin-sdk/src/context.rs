//! The execution context, typed.
//!
//! Byte-identical to `raisin.context.get()` in JavaScript and Starlark — the
//! host builds it once per execution (`FunctionApi::get_context`) and the
//! runtime hands it over through the `context` import, not the gateway, so it
//! costs no round trip.

use crate::error::{Error, Result};
use crate::host;
use serde::de::DeserializeOwned;
use serde_json::Value;

/// One execution's context.
#[derive(Debug, Clone, PartialEq)]
pub struct Context(Value);

impl Context {
    /// Read the context the host installed for this execution.
    pub fn get() -> Result<Self> {
        let raw = host::context();
        let value: Value = serde_json::from_str(&raw)
            .map_err(|e| Error::Wire(format!("context is not JSON ({e}): {raw}")))?;
        Ok(Self(value))
    }

    /// The context as a raw JSON value.
    pub fn as_value(&self) -> &Value {
        &self.0
    }

    /// The context, deserialised into a type of your own.
    pub fn parse_as<T: DeserializeOwned>(&self) -> Result<T> {
        Ok(serde_json::from_value(self.0.clone())?)
    }

    /// A top-level field, if present.
    pub fn get_field(&self, key: &str) -> Option<&Value> {
        self.0.get(key)
    }

    fn string_field(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(Value::as_str)
    }

    /// The tenant this execution belongs to.
    pub fn tenant_id(&self) -> Option<&str> {
        self.string_field("tenant_id")
    }

    /// The repository this execution belongs to.
    pub fn repo_id(&self) -> Option<&str> {
        self.string_field("repo_id")
    }

    /// The branch this execution reads and writes.
    pub fn branch(&self) -> Option<&str> {
        self.string_field("branch")
    }

    /// The workspace this execution was invoked against, if any.
    pub fn workspace_id(&self) -> Option<&str> {
        self.string_field("workspace_id")
    }

    /// Who is executing (a user id, an agent, or `"system"`).
    pub fn actor(&self) -> Option<&str> {
        self.string_field("actor")
    }

    /// This execution's id — the same one the SSE log stream carries.
    pub fn execution_id(&self) -> Option<&str> {
        self.string_field("execution_id")
    }

    /// The trigger name, when a trigger started this execution.
    pub fn trigger_name(&self) -> Option<&str> {
        self.string_field("trigger_name")
    }

    /// The trigger event payload, when there is one.
    pub fn event(&self) -> Option<&Value> {
        self.0.get("event")
    }

    /// The full input, as the host saw it.
    pub fn input(&self) -> Option<&Value> {
        self.0.get("input")
    }
}
