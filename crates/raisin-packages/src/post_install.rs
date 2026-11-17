// SPDX-License-Identifier: BSL-1.1

//! `post_install` hooks: functions a package asks the installer to run after
//! every install.
//!
//! ```yaml
//! post_install:
//!   - function: /lib/studio/setup-store
//!     input: { store: main }
//!   - function: /lib/studio/publish-all
//! ```
//!
//! The installer runs the entries in order, on the `main` branch only, after
//! the package is marked installed, as `system`, and stops at the first one
//! that fails. A hook therefore runs again on every install (skip mode
//! included) and must be idempotent.
//!
//! This one type is shared by the upload path ([`crate::Manifest`]) and the
//! install job's own manifest struct, so the two can never disagree about what
//! a valid hook is.

use serde::{Deserialize, Serialize};

/// Upper bound on the number of hooks one manifest may declare. Every hook is
/// a function job the install job waits for, inside the install job's own
/// time budget; a list longer than this is a design problem, not a setup step.
pub const MAX_POST_INSTALL_HOOKS: usize = 32;

/// One `post_install` entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PostInstallHook {
    /// Absolute path of a `raisin:Function` in the `functions` workspace,
    /// e.g. `/lib/studio/setup-store`. It must exist when the hook runs:
    /// shipped by this package or installed by an earlier one.
    pub function: String,

    /// The function's input. Must be a JSON/YAML object; `{}` when absent.
    #[serde(default = "empty_object")]
    pub input: serde_json::Value,
}

fn empty_object() -> serde_json::Value {
    serde_json::Value::Object(serde_json::Map::new())
}

/// Validate a manifest's `post_install` list.
///
/// Returns the first problem as a sentence naming the entry, so the CLI, the
/// upload handler and the install job report it the same way.
pub fn validate_post_install(hooks: &[PostInstallHook]) -> Result<(), String> {
    if hooks.len() > MAX_POST_INSTALL_HOOKS {
        return Err(format!(
            "post_install declares {} entries; at most {} are allowed",
            hooks.len(),
            MAX_POST_INSTALL_HOOKS
        ));
    }
    for (i, hook) in hooks.iter().enumerate() {
        let path = hook.function.trim();
        if path.is_empty() {
            return Err(format!("post_install[{i}].function must not be empty"));
        }
        if path != hook.function {
            return Err(format!(
                "post_install[{i}].function '{}' must not have surrounding whitespace",
                hook.function
            ));
        }
        if !path.starts_with('/') {
            return Err(format!(
                "post_install[{i}].function '{path}' must be an absolute path in the \
                 functions workspace (starting with '/')"
            ));
        }
        if path.split('/').any(|seg| seg == "." || seg == "..") {
            return Err(format!(
                "post_install[{i}].function '{path}' must not contain '.' or '..' segments"
            ));
        }
        if !hook.input.is_object() {
            return Err(format!(
                "post_install[{i}].input for '{path}' must be an object, got {}",
                json_kind(&hook.input)
            ));
        }
    }
    Ok(())
}

/// Name of the `raisin:Package` property that keeps the installed manifest's
/// hooks, so an export rebuilt from the package node can write them back.
pub const POST_INSTALL_PROPERTY: &str = "post_install";

/// The hooks as stored in [`POST_INSTALL_PROPERTY`]: a JSON string, so the
/// inputs round-trip byte for byte (a node property would reinterpret an
/// input object that happens to look like a reference or a resource).
/// `None` for an empty list: the property is removed.
pub fn post_install_to_property(hooks: &[PostInstallHook]) -> Option<String> {
    if hooks.is_empty() {
        return None;
    }
    serde_json::to_string(hooks).ok()
}

/// Read [`POST_INSTALL_PROPERTY`] back.
pub fn post_install_from_property(json: &str) -> Result<Vec<PostInstallHook>, String> {
    serde_json::from_str(json).map_err(|e| format!("stored post_install is not valid: {e}"))
}

fn json_kind(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "a boolean",
        serde_json::Value::Number(_) => "a number",
        serde_json::Value::String(_) => "a string",
        serde_json::Value::Array(_) => "an array",
        serde_json::Value::Object(_) => "an object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(yaml: &str) -> Vec<PostInstallHook> {
        serde_yaml::from_str(yaml).unwrap()
    }

    #[test]
    fn parses_ordered_entries_and_defaults_input() {
        let hooks =
            parse("- { function: /lib/a, input: { store: main, stock: 5 } }\n- function: /lib/b\n");
        assert_eq!(hooks.len(), 2);
        assert_eq!(hooks[0].function, "/lib/a");
        assert_eq!(hooks[0].input["store"], "main");
        assert_eq!(hooks[0].input["stock"], 5);
        assert_eq!(hooks[1].function, "/lib/b");
        assert_eq!(hooks[1].input, serde_json::json!({}));
        assert!(validate_post_install(&hooks).is_ok());
    }

    #[test]
    fn rejects_unknown_keys() {
        let err = serde_yaml::from_str::<Vec<PostInstallHook>>("- { functon: /lib/a }\n")
            .unwrap_err()
            .to_string();
        assert!(err.contains("functon"), "{err}");
    }

    #[test]
    fn rejects_relative_empty_and_dotted_paths() {
        for (path, needle) in [
            ("", "must not be empty"),
            ("lib/a", "absolute"),
            ("/lib/../a", "'..'"),
            (" /lib/a", "whitespace"),
        ] {
            let hooks = vec![PostInstallHook {
                function: path.to_string(),
                input: empty_object(),
            }];
            let err = validate_post_install(&hooks).unwrap_err();
            assert!(err.contains(needle), "{path:?}: {err}");
        }
    }

    #[test]
    fn rejects_non_object_input() {
        for input in ["null", "[1]", "'x'", "3"] {
            let hooks = parse(&format!("- {{ function: /lib/a, input: {input} }}\n"));
            let err = validate_post_install(&hooks).unwrap_err();
            assert!(err.contains("must be an object"), "{input}: {err}");
        }
    }

    #[test]
    fn property_round_trips_inputs_exactly() {
        let hooks = parse(
            "- { function: /lib/a, input: { raisin:ref: /x, n: 1, s: '10' } }\n- function: /lib/b\n",
        );
        let stored = post_install_to_property(&hooks).unwrap();
        assert_eq!(post_install_from_property(&stored).unwrap(), hooks);
        assert_eq!(post_install_to_property(&[]), None);
    }

    #[test]
    fn rejects_too_many_entries() {
        let hooks = vec![
            PostInstallHook {
                function: "/lib/a".into(),
                input: empty_object(),
            };
            MAX_POST_INSTALL_HOOKS + 1
        ];
        assert!(validate_post_install(&hooks)
            .unwrap_err()
            .contains("at most"));
        assert!(validate_post_install(&hooks[..MAX_POST_INSTALL_HOOKS]).is_ok());
    }
}
