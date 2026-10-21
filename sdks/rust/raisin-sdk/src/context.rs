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

    /// Who invoked this function, as the server resolved it — present even
    /// when the function runs as the system (`execution_context: "system"`),
    /// so it can refuse a caller who may not ask for what it does. `None`
    /// when nobody invoked it (a trigger, a schedule, a flow step).
    pub fn caller(&self) -> Option<Caller> {
        self.0
            .get("caller")
            .and_then(|c| serde_json::from_value(c.clone()).ok())
    }

    /// The flow function step that started this call, as the flow RUNTIME
    /// recorded it. Unforgeable: it is not part of the input, and a direct
    /// HTTP/WS invoke (whatever it puts in its input, `__raisin_flow`
    /// included) never produces it. `None` for every call that did not come
    /// from a flow function step.
    pub fn flow(&self) -> Option<FlowRun> {
        let flow = self.0.get("flow")?;
        if flow.get("trigger").and_then(Value::as_str) != Some("flow") {
            return None;
        }
        serde_json::from_value(flow.clone()).ok()
    }
}

/// The flow function step that is running this call; see [`Context::flow`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct FlowRun {
    /// The flow instance executing the step.
    pub instance_id: String,
    /// The step that invoked the function.
    pub step_id: String,
    /// Path of the flow definition the instance runs.
    #[serde(default)]
    pub flow_path: Option<String>,
    /// Retry attempt of the step, 0-indexed.
    #[serde(default)]
    pub attempt: u32,
}

/// The invoking identity; see [`Context::caller`].
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
pub struct Caller {
    /// The caller's user id, if it has one.
    #[serde(default)]
    pub user_id: Option<String>,
    /// Effective roles (direct and through groups).
    #[serde(default)]
    pub roles: Vec<String>,
    /// The system itself (admin credential, API key, superadmin).
    #[serde(default)]
    pub is_system: bool,
    /// Holds `system_admin`.
    #[serde(default)]
    pub is_system_admin: bool,
    /// Not signed in.
    #[serde(default)]
    pub anonymous: bool,
    /// The client address as the server's TRUSTED proxy reported it for a
    /// client connection (WebSocket invoke). `None` when the deployment has no
    /// trusted proxy, or the call did not come from a client. Unlike anything
    /// in the input, the caller cannot choose it.
    #[serde(default)]
    pub client_ip: Option<String>,
}

impl Caller {
    /// Whether the caller holds any of `roles`, or is the system or a
    /// `system_admin` (who may do anything).
    pub fn has_any_role(&self, roles: &[&str]) -> bool {
        if self.anonymous {
            return false;
        }
        self.is_system
            || self.is_system_admin
            || self.roles.iter().any(|r| roles.contains(&r.as_str()))
    }
}

#[cfg(test)]
mod flow_tests {
    use super::*;

    #[test]
    fn flow_is_read_from_the_context_and_never_from_the_input() {
        let ctx = Context(serde_json::json!({
            "flow": {"instance_id": "i1", "step_id": "remind", "flow_path": "/flows/f",
                     "attempt": 1, "trigger": "flow"}
        }));
        let f = ctx.flow().unwrap();
        assert_eq!(
            (f.instance_id.as_str(), f.step_id.as_str()),
            ("i1", "remind")
        );
        assert_eq!(f.flow_path.as_deref(), Some("/flows/f"));
        assert_eq!(f.attempt, 1);

        let forged = Context(serde_json::json!({
            "input": {"__raisin_flow": {"instance_id": "i1", "step_id": "remind"}}
        }));
        assert!(forged.flow().is_none());
    }
}

#[cfg(test)]
mod caller_tests {
    use super::*;

    #[test]
    fn caller_is_read_from_the_context() {
        let ctx = Context(serde_json::json!({
            "actor": "system",
            "caller": {"user_id": "u1", "roles": ["studio_editor"], "is_system": false,
                       "is_system_admin": false, "anonymous": false}
        }));
        let c = ctx.caller().unwrap();
        assert_eq!(c.user_id.as_deref(), Some("u1"));
        assert!(c.has_any_role(&["studio_admin", "studio_editor"]));
        assert!(!c.has_any_role(&["studio_admin"]));
        assert!(Context(serde_json::json!({})).caller().is_none());
        let anon = Caller {
            anonymous: true,
            roles: vec!["studio_admin".into()],
            ..Caller::default()
        };
        assert!(!anon.has_any_role(&["studio_admin"]));
    }

    #[test]
    fn client_ip_is_optional_on_the_caller() {
        let with = Context(
            serde_json::json!({"caller": {"anonymous": false, "client_ip": "203.0.113.7"}}),
        );
        assert_eq!(
            with.caller().unwrap().client_ip.as_deref(),
            Some("203.0.113.7")
        );
        let without = Context(serde_json::json!({"caller": {"anonymous": false}}));
        assert_eq!(without.caller().unwrap().client_ip, None);
    }
}
