//! Who may change the SCHEMA over SQL.
//!
//! NodeTypes, Archetypes, ElementTypes, Mixins and workspace definitions decide
//! what every piece of content in a repository may be, and which workspaces
//! exist at all. Changing them is an operator's act, like workspace management
//! over REST (`require_operator` in raisin-transport-http) and like ACL DDL.
//!
//! Until this existed nothing checked it: an ANONYMOUS HTTP caller on a
//! repository with anonymous access enabled — every public site — could
//! `CREATE NODETYPE`, and any signed-in site user could rewrite an archetype.
//! Measured on a dev server 2026-09-21.

use raisin_error::Error;
use raisin_models::auth::AuthContext;

/// Refuse a schema change unless the caller is an operator.
///
/// `None` is the engine's internal context (migrations, package install, tests
/// that construct an engine directly) — the same meaning `ExecutionContext`
/// gives it. Every external surface sets an explicit context: HTTP resolves an
/// anonymous or user context, functions default to `AuthContext::system()`.
pub fn require_schema_operator(auth: Option<&AuthContext>, operation: &str) -> Result<(), Error> {
    if is_operator(auth) {
        return Ok(());
    }
    Err(Error::Forbidden(format!(
        "{operation} changes the repository schema and requires an operator \
         (a system context or the system_admin role)"
    )))
}

/// Refuse any other repository-wide administrative statement (branch
/// CREATE/DROP/ALTER/MERGE) unless the caller is an operator. Same rule as
/// [`require_schema_operator`], including `None` as the internal context.
pub fn require_operator(auth: Option<&AuthContext>, operation: &str) -> Result<(), Error> {
    if is_operator(auth) {
        return Ok(());
    }
    Err(Error::Forbidden(format!(
        "{operation} requires an operator (a system context or the system_admin role)"
    )))
}

fn is_operator(auth: Option<&AuthContext>) -> bool {
    let Some(auth) = auth else {
        return true;
    };
    if auth.is_system {
        return true;
    }
    !auth.is_anonymous_principal()
        && (auth
            .resolved_permissions
            .as_ref()
            .is_some_and(|p| p.is_system_admin)
            || auth.has_role("system_admin"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_internal_context_and_system_may_change_the_schema() {
        assert!(require_schema_operator(None, "CREATE NODETYPE").is_ok());
        assert!(require_schema_operator(Some(&AuthContext::system()), "CREATE NODETYPE").is_ok());
    }

    #[test]
    fn anonymous_callers_may_not() {
        let err = require_schema_operator(Some(&AuthContext::anonymous()), "CREATE NODETYPE")
            .unwrap_err();
        assert!(matches!(err, Error::Forbidden(_)), "{err:?}");
        let err = require_schema_operator(
            Some(&AuthContext::anonymous_user("anon")),
            "INSERT INTO Archetypes",
        )
        .unwrap_err();
        assert!(matches!(err, Error::Forbidden(_)), "{err:?}");
    }
}
