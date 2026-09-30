// SPDX-License-Identifier: BSL-1.1

//! Who may invoke a function from a client.
//!
//! The WebSocket invoke paths found the function node without looking at the
//! caller at all, so any connection — anonymous included — could run any
//! function by path, including `execution_context: "system"` ones that bypass
//! row-level security. SQL `INVOKE`/`INVOKE_SYNC` did the same.
//!
//! Every CLIENT entry point (WS `function_invoke`/`function_invoke_sync`, HTTP
//! invoke, SQL `INVOKE`/`INVOKE_SYNC`) asks this before loading any code:
//! * the system and `system_admin` (admin console, API keys, CLI) may invoke
//!   anything;
//! * a caller who is not signed in may invoke nothing;
//! * a signed-in caller may invoke `execution_context: "user"` functions,
//!   which run under their own permissions, but not `"system"` ones.
//!
//! This is the interim rule. The permanent one is a grantable `execute`
//! permission on the function node ([`Operation::Execute`]); this rule is a
//! strict subset of what that grants by default. Internal runs (triggers,
//! flows, schedules, agent tool calls, function-to-function calls) are not
//! client invokes and never come here.
//!
//! [`Operation::Execute`]: raisin_models::permissions::Operation::Execute

use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;

/// The workspace functions live in.
pub const FUNCTIONS_WORKSPACE: &str = "functions";

/// Why an invoke was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvokeRefusal {
    /// Not signed in.
    Unauthenticated,
    /// Signed in, but the function runs as the system.
    Forbidden,
}

impl std::fmt::Display for InvokeRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Unauthenticated => "this function may not be invoked without signing in",
            Self::Forbidden => "not allowed to execute this function",
        })
    }
}

fn runs_as_system(function: &Node) -> bool {
    matches!(
        function.properties.get("execution_context"),
        Some(PropertyValue::String(c)) if c.eq_ignore_ascii_case("system")
    )
}

/// The decision. `auth` is the invoking caller as the transport resolved it
/// (`None`: no identity at all); `function` is the `raisin:Function` node as
/// stored; `branch` the branch it was found on.
pub fn authorize_invoke(
    auth: Option<&AuthContext>,
    function: &Node,
    branch: &str,
) -> Result<(), InvokeRefusal> {
    authorize_invoke_in(auth, function, FUNCTIONS_WORKSPACE, branch)
}

/// [`authorize_invoke`] for a function found in `workspace` (SQL `INVOKE`
/// names the workspace; everything else uses `functions`).
pub fn authorize_invoke_in(
    auth: Option<&AuthContext>,
    function: &Node,
    _workspace: &str,
    _branch: &str,
) -> Result<(), InvokeRefusal> {
    let Some(auth) = auth else {
        return Err(InvokeRefusal::Unauthenticated);
    };
    if auth.is_system || auth.permissions().is_some_and(|p| p.is_system_admin) {
        return Ok(());
    }
    if auth.is_anonymous_principal() {
        return Err(InvokeRefusal::Unauthenticated);
    }
    if runs_as_system(function) {
        return Err(InvokeRefusal::Forbidden);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_models::permissions::ResolvedPermissions;

    fn function(path: &str, system: bool) -> Node {
        let mut node = Node {
            id: format!("fn{path}"),
            name: path.rsplit('/').next().unwrap_or_default().to_string(),
            path: path.to_string(),
            node_type: "raisin:Function".to_string(),
            workspace: Some(FUNCTIONS_WORKSPACE.to_string()),
            ..Default::default()
        };
        if system {
            node.properties.insert(
                "execution_context".into(),
                PropertyValue::String("system".into()),
            );
        }
        node
    }

    fn user() -> AuthContext {
        AuthContext::for_user("ed").with_permissions(ResolvedPermissions::empty("ed"))
    }

    #[test]
    fn signed_in_callers_invoke_user_functions_only() {
        assert_eq!(
            authorize_invoke(Some(&user()), &function("/f", false), "main"),
            Ok(())
        );
        assert_eq!(
            authorize_invoke(Some(&user()), &function("/f", true), "main"),
            Err(InvokeRefusal::Forbidden)
        );
    }

    #[test]
    fn anonymous_callers_invoke_nothing() {
        let anon = AuthContext::anonymous_user("anon")
            .with_permissions(ResolvedPermissions::anonymous(vec![]));
        for who in [Some(&anon), None, Some(&AuthContext::deny_all())] {
            for system in [false, true] {
                assert_eq!(
                    authorize_invoke(who, &function("/f", system), "main"),
                    Err(InvokeRefusal::Unauthenticated)
                );
            }
        }
    }

    #[test]
    fn administrators_and_the_system_invoke_anything() {
        let admin =
            AuthContext::for_user("root").with_permissions(ResolvedPermissions::system_admin());
        for who in [admin, AuthContext::system()] {
            for system in [false, true] {
                assert_eq!(
                    authorize_invoke(Some(&who), &function("/f", system), "main"),
                    Ok(())
                );
            }
        }
    }

    #[test]
    fn execute_is_a_known_operation() {
        use raisin_models::permissions::Operation;
        assert_eq!(Operation::parse("execute"), Some(Operation::Execute));
        assert_eq!(Operation::Execute.to_string(), "execute");
        assert!(Operation::all().contains(&Operation::Execute));
    }
}
