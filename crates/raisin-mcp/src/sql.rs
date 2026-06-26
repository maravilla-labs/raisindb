// SPDX-License-Identifier: BSL-1.1

//! SQL identifier safety for workspace names interpolated into queries.
//!
//! The data-tool and registry queries name the workspace as a quoted string in
//! the `FROM '<workspace>'` clause, which cannot be supplied as a bound
//! parameter. Workspace names originate from author-controlled
//! `raisin:McpServer` content, so they are validated against a strict character
//! set before interpolation (defense in depth) rather than trusted or escaped.

use crate::error::{McpError, Result};

/// Maximum accepted workspace-identifier length.
const MAX_WORKSPACE_LEN: usize = 128;

/// Validate `workspace` and return it as a single-quoted SQL identifier.
///
/// Accepts only `[A-Za-z0-9_:.-]` (non-empty, at most [`MAX_WORKSPACE_LEN`]),
/// which still covers system workspaces such as `raisin:access_control`. Any
/// other input — notably quotes, whitespace, or statement separators that could
/// break out of the quoted literal — is rejected, never escaped.
pub(crate) fn quote_workspace(workspace: &str) -> Result<String> {
    let valid = !workspace.is_empty()
        && workspace.len() <= MAX_WORKSPACE_LEN
        && workspace
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b':' | b'.'));

    if !valid {
        return Err(McpError::invalid_params(format!(
            "invalid workspace identifier: `{workspace}`"
        )));
    }

    Ok(format!("'{workspace}'"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_plain_and_system_workspaces() {
        for ws in ["mcp", "content", "default", "raisin:access_control", "a-b_c.d"] {
            assert_eq!(quote_workspace(ws).unwrap(), format!("'{ws}'"));
        }
    }

    #[test]
    fn rejects_injection_and_malformed_names() {
        for ws in [
            "",
            "ws'; DROP TABLE x; --",
            "a' OR '1'='1",
            "has space",
            "tab\tname",
            "semi;colon",
            "paren(s)",
        ] {
            assert!(quote_workspace(ws).is_err(), "should reject `{ws}`");
        }
    }

    #[test]
    fn rejects_overlong_names() {
        let long = "a".repeat(MAX_WORKSPACE_LEN + 1);
        assert!(quote_workspace(&long).is_err());
    }
}
