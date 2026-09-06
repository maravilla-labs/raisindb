// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! `entry_file` resolution: which asset holds the code, and which handler in it
//! to call.
//!
//! The grammar is `<asset path>[:<handler name>]`. The asset path is resolved
//! against the function node's own path, so `main.wasm` is the sibling asset and
//! `../shared/main.wasm` is the one next door — which is what lets N
//! `raisin:Function` nodes point at ONE uploaded artifact. Resolution stays
//! inside the functions workspace: a path that would climb above its root is
//! refused rather than silently clamped, because a clamped `../../../etc/passwd`
//! resolves to a perfectly ordinary node path and reads whatever happens to live
//! there.

use std::path::{Component, Path, PathBuf};

use raisin_error::Result;

use crate::types::FunctionLanguage;

/// The handler name a bare `entry_file` (no `:suffix`) implies.
///
/// `wasm` answers `"default"`: the WIT export is `handler(name, input)`, so the
/// name is *data* the guest routes on, and a guest with a single handler
/// registers it as `"default"`. Text languages answer `"handler"`, the exported
/// function name JavaScript and Starlark functions have always used.
pub fn default_handler_name(language: FunctionLanguage) -> &'static str {
    match language {
        FunctionLanguage::Wasm => "default",
        _ => "handler",
    }
}

/// Resolve `entry_file` into `(asset path, handler name)`.
///
/// The handler name is passed through **verbatim**. There is deliberately no
/// allow-list: for wasm the guest owns its handler namespace and answers an
/// unknown name with an `Err` listing what it registered, and inventing a
/// second, host-side list of legal names here would make a correct guest
/// unreachable.
///
/// # Errors
///
/// [`raisin_error::Error::Validation`] when the asset path climbs above the
/// functions workspace root.
pub fn resolve_entry_file(
    function_path: &str,
    entry_file: &str,
    language: FunctionLanguage,
) -> Result<(String, String)> {
    let (file_part, handler) = match entry_file.rsplit_once(':') {
        Some((file, handler)) if !handler.trim().is_empty() => {
            (file.trim(), handler.trim().to_string())
        }
        // A trailing bare `:` is a typo, not a nameless handler.
        Some((file, _)) => (file.trim(), default_handler_name(language).to_string()),
        None => (
            entry_file.trim(),
            default_handler_name(language).to_string(),
        ),
    };

    let joined = Path::new(function_path).join(Path::new(file_part));
    let normalized = normalize_within_root(&joined).ok_or_else(|| {
        raisin_error::Error::Validation(format!(
            "entry_file '{}' resolves outside the functions workspace (from '{}')",
            entry_file, function_path
        ))
    })?;

    let path_str = normalized.to_string_lossy().to_string();
    let full_path = if path_str.starts_with('/') {
        path_str
    } else {
        format!("/{}", path_str)
    };

    Ok((full_path, handler))
}

/// Normalise `.` and `..`, refusing to climb above the root.
///
/// Returns `None` for a path whose `..` segments outnumber the segments they
/// could pop. `PathBuf::pop` clamps at `/` instead, which turns an escape into a
/// plausible-looking absolute node path — the failure mode this exists to make
/// loud.
fn normalize_within_root(path: &Path) -> Option<PathBuf> {
    let mut result = PathBuf::new();
    let mut depth = 0usize;

    for component in path.components() {
        match component {
            Component::ParentDir => {
                if depth == 0 {
                    return None;
                }
                result.pop();
                depth -= 1;
            }
            Component::CurDir => {}
            Component::RootDir => result.push("/"),
            Component::Normal(name) => {
                result.push(name);
                depth += 1;
            }
            Component::Prefix(prefix) => result.push(prefix.as_os_str()),
        }
    }

    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn js(function_path: &str, entry_file: &str) -> (String, String) {
        resolve_entry_file(function_path, entry_file, FunctionLanguage::JavaScript).unwrap()
    }

    fn wasm(function_path: &str, entry_file: &str) -> (String, String) {
        resolve_entry_file(function_path, entry_file, FunctionLanguage::Wasm).unwrap()
    }

    #[test]
    fn a_sibling_asset_resolves_next_to_the_function() {
        let (path, handler) = js("/lib/raisin/agent-handler", "index.js:handleUserMessage");
        assert_eq!(path, "/lib/raisin/agent-handler/index.js");
        assert_eq!(handler, "handleUserMessage");

        let (path, handler) = js("/lib/raisin/agent-handler", "src/handlers/main.js:run");
        assert_eq!(path, "/lib/raisin/agent-handler/src/handlers/main.js");
        assert_eq!(handler, "run");
    }

    #[test]
    fn a_bare_entry_file_takes_the_language_default_handler() {
        let (path, handler) = js("/lib/raisin/agent-handler", "main.js");
        assert_eq!(path, "/lib/raisin/agent-handler/main.js");
        assert_eq!(handler, "handler");

        // Wasm answers "default", not "handler": the name is routed inside the
        // single WIT export, and a one-handler guest registers it as "default".
        let (path, handler) = wasm("/lib/greet", "main.wasm");
        assert_eq!(path, "/lib/greet/main.wasm");
        assert_eq!(handler, "default");
    }

    #[test]
    fn a_named_handler_is_passed_through_verbatim() {
        // No allow-list: the guest owns its handler namespace. Names that are
        // not Rust/JS identifiers are still names.
        for name in ["on-order", "default", "Weird.Name_9", "handler"] {
            let (path, handler) = wasm("/lib/greet", &format!("main.wasm:{name}"));
            assert_eq!(path, "/lib/greet/main.wasm");
            assert_eq!(handler, name);
        }
    }

    #[test]
    fn a_parent_relative_artifact_is_shared_between_functions() {
        // The one-artifact-N-functions path: two Function nodes, one `.wasm`.
        let (path, handler) = wasm("/lib/greet-shout", "../greet/main.wasm:shout");
        assert_eq!(path, "/lib/greet/main.wasm");
        assert_eq!(handler, "shout");

        let (path, handler) = js("/lib/raisin/agent-handler", "../shared/utils.js:helper");
        assert_eq!(path, "/lib/raisin/shared/utils.js");
        assert_eq!(handler, "helper");
    }

    #[test]
    fn a_path_that_escapes_the_workspace_root_is_refused() {
        // Two segments deep, three `..`: `PathBuf::pop` would clamp this to
        // "/etc/passwd" and read whichever node lives there.
        let err = resolve_entry_file(
            "/lib/greet",
            "../../../etc/passwd:default",
            FunctionLanguage::Wasm,
        )
        .unwrap_err()
        .to_string();
        assert!(
            err.contains("outside the functions workspace"),
            "unexpected message: {err}"
        );

        assert!(resolve_entry_file("/lib/greet", "../../..", FunctionLanguage::Wasm).is_err());
        // Exactly at the root is still inside it.
        let (path, _) = wasm("/lib/greet", "../../shared.wasm");
        assert_eq!(path, "/shared.wasm");
    }

    #[test]
    fn a_trailing_colon_is_a_typo_not_a_nameless_handler() {
        let (path, handler) = wasm("/lib/greet", "main.wasm:");
        assert_eq!(path, "/lib/greet/main.wasm");
        assert_eq!(handler, "default");
    }
}
