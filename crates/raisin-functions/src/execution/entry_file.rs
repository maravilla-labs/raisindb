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

/// Split an `entry_file` into `(file part, handler name)` WITHOUT resolving the
/// path.
///
/// This is the ONE parser for the grammar. `FunctionMetadata::entry_file_path`
/// and `entry_function_name` call it, and so does [`resolve_entry_file`] — they
/// used to hold their own copies, and a bare `main.wasm` therefore invoked as
/// handler `"default"` through the job path and as handler `"main.wasm"` through
/// the sync HTTP path, which the guest answered with
/// `unknown handler 'main.wasm'`.
///
/// Three shapes, in the order they are tested:
///
/// 1. `file:handler` — split on the LAST colon, both parts verbatim.
/// 2. a bare name that looks like a FILE (it has an extension) — the handler is
///    the language default.
/// 3. a bare name with no extension — the LEGACY `entrypoint` spelling, where
///    the whole string was the exported function name and the asset was always
///    `index.js`. Kept because old `raisin:Function` nodes still carry it.
pub fn split_entry_file(entry_file: &str, language: FunctionLanguage) -> (&str, &str) {
    match entry_file.rsplit_once(':') {
        Some((file, handler)) if !handler.trim().is_empty() => (file.trim(), handler.trim()),
        // A trailing bare `:` is a typo, not a nameless handler.
        Some((file, _)) => (file.trim(), default_handler_name(language)),
        None => {
            let bare = entry_file.trim();
            // A bare name is the LEGACY `entrypoint` spelling only when it looks
            // like a plain identifier: no separator and no extension. Anything
            // carrying a `/` or a `.` is a path, and must stay one — otherwise
            // `../../..` would be read as a function name instead of being
            // refused for climbing out of the workspace.
            if bare.contains('.') || bare.contains('/') {
                (bare, default_handler_name(language))
            } else {
                ("index.js", bare)
            }
        }
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
    let (file_part, handler) = split_entry_file(entry_file, language);
    let handler = handler.to_string();

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
    fn one_parser_serves_the_metadata_accessors_and_the_resolver() {
        use crate::types::FunctionMetadata;

        // Regression: `FunctionMetadata` held its own copy of this grammar that
        // returned the WHOLE string as the handler when there was no colon. A
        // wasm function with `entry_file = "main.wasm"` therefore ran as handler
        // "default" through the job path and as handler "main.wasm" through the
        // sync HTTP path, where the guest answered
        // `unknown handler 'main.wasm'; registered: default, shout`.
        let mut wasm_meta = FunctionMetadata::new("greet", FunctionLanguage::Wasm);
        wasm_meta.entry_file = "main.wasm".to_string();
        assert_eq!(wasm_meta.entry_file_path(), "main.wasm");
        assert_eq!(wasm_meta.entry_function_name(), "default");

        let (_, resolved_handler) = wasm("/lib/greet", &wasm_meta.entry_file);
        assert_eq!(resolved_handler, wasm_meta.entry_function_name());

        // A named handler agrees on both sides too.
        wasm_meta.entry_file = "main.wasm:shout".to_string();
        assert_eq!(wasm_meta.entry_file_path(), "main.wasm");
        assert_eq!(wasm_meta.entry_function_name(), "shout");
        assert_eq!(wasm("/lib/greet", &wasm_meta.entry_file).1, "shout");

        // And so does JavaScript, whose default handler is "handler".
        let mut js_meta = FunctionMetadata::new("greet", FunctionLanguage::JavaScript);
        js_meta.entry_file = "index.js".to_string();
        assert_eq!(js_meta.entry_file_path(), "index.js");
        assert_eq!(js_meta.entry_function_name(), "handler");
        assert_eq!(js("/lib/greet", &js_meta.entry_file).1, "handler");
    }

    #[test]
    fn a_bare_name_with_no_extension_is_the_legacy_entrypoint_spelling() {
        use crate::types::FunctionMetadata;

        // Old `raisin:Function` nodes carry `entrypoint: handler`, where the
        // whole string is the EXPORTED FUNCTION NAME and the asset is always
        // index.js. That has to keep working, which is why the split tests for
        // an extension rather than assuming a bare name is a file.
        let mut meta = FunctionMetadata::new("legacy", FunctionLanguage::JavaScript);
        meta.entry_file = "handleUserMessage".to_string();
        assert_eq!(meta.entry_file_path(), "index.js");
        assert_eq!(meta.entry_function_name(), "handleUserMessage");

        let (path, handler) = js("/lib/legacy", "handleUserMessage");
        assert_eq!(path, "/lib/legacy/index.js");
        assert_eq!(handler, "handleUserMessage");
    }

    #[test]
    fn a_trailing_colon_is_a_typo_not_a_nameless_handler() {
        let (path, handler) = wasm("/lib/greet", "main.wasm:");
        assert_eq!(path, "/lib/greet/main.wasm");
        assert_eq!(handler, "default");
    }
}
