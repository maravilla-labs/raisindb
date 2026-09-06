// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The unit a runtime executes: source text, or an opaque artifact.
//!
//! JavaScript, Starlark and SQL functions are *text*; a WebAssembly component
//! is *bytes* and has no readable source on the server at all. Both travel the
//! same path — loader → [`crate::types::LoadedFunction`] → `FunctionRuntime` —
//! so the code that path carries is one type with two shapes rather than a
//! hash string plus a side table keyed by it. A global side table would be a
//! cross-tenant map holding tenant artifacts, which is exactly the shape this
//! codebase treats as a leak waiting to happen.

use std::sync::Arc;

use raisin_error::Result;

use super::FunctionLanguage;

/// Default ceiling for a WebAssembly artifact, in bytes (32 MiB).
///
/// jco / StarlingMonkey components land at 8-15 MB, so the cap has to sit well
/// above that while still refusing an upload that would spend minutes inside
/// Cranelift. Made configurable by `[functions.wasm] max_artifact_bytes`.
pub const DEFAULT_MAX_WASM_ARTIFACT_BYTES: usize = 33_554_432;

/// Refuse an artifact larger than `max` bytes.
///
/// The ONE place the over-cap message is written. The cap itself differs by
/// caller — the wasm engine reads `[functions.wasm] max_artifact_bytes`, a
/// build without the engine falls back to [`DEFAULT_MAX_WASM_ARTIFACT_BYTES`]
/// — but an operator who sees this text twice must not be reading two
/// different rules.
pub fn check_wasm_artifact_size(len: usize, max: usize) -> Result<()> {
    if len > max {
        return Err(raisin_error::Error::Validation(format!(
            "wasm artifact is {len} bytes, over the {max}-byte limit"
        )));
    }
    Ok(())
}

/// A function's executable payload.
///
/// `Text` is source a runtime parses; `Bytes` is a compiled artifact it hands
/// to an engine. `Arc<[u8]>` because the artifact is cloned into every
/// execution and is measured in megabytes.
#[derive(Debug, Clone)]
pub enum FunctionCode {
    /// Source text (JavaScript, Starlark, SQL).
    Text(String),
    /// An opaque binary artifact (a WebAssembly component).
    Bytes(Arc<[u8]>),
}

impl FunctionCode {
    /// Borrow the payload as source text.
    ///
    /// Errors for `Bytes`: a text runtime asked to run an artifact is a
    /// configuration mistake (`language: wasm` on a JavaScript function, say),
    /// not a decoding problem, so this never attempts a lossy conversion.
    pub fn as_text(&self) -> Result<&str> {
        match self {
            Self::Text(s) => Ok(s),
            Self::Bytes(b) => Err(raisin_error::Error::Validation(format!(
                "this function's code is a {}-byte binary artifact, not source text; \
                 only a binary runtime (wasm) can execute it",
                b.len()
            ))),
        }
    }

    /// Borrow the payload as bytes. Text yields its UTF-8 encoding.
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            Self::Text(s) => s.as_bytes(),
            Self::Bytes(b) => b,
        }
    }

    /// Payload size in bytes.
    pub fn len(&self) -> usize {
        match self {
            Self::Text(s) => s.len(),
            Self::Bytes(b) => b.len(),
        }
    }

    /// Whether the payload is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether this is an opaque artifact rather than source text.
    pub fn is_binary(&self) -> bool {
        matches!(self, Self::Bytes(_))
    }

    /// A human-readable rendering for API responses and logs.
    ///
    /// Text is returned verbatim; an artifact becomes a one-line summary,
    /// because a `.wasm` blob spliced into a JSON field is neither useful nor
    /// valid UTF-8.
    pub fn to_display_string(&self) -> String {
        match self {
            Self::Text(s) => s.clone(),
            Self::Bytes(b) => format!("<binary artifact: {} bytes>", b.len()),
        }
    }
}

impl From<String> for FunctionCode {
    fn from(s: String) -> Self {
        Self::Text(s)
    }
}

impl From<&str> for FunctionCode {
    fn from(s: &str) -> Self {
        Self::Text(s.to_string())
    }
}

impl From<Arc<[u8]>> for FunctionCode {
    fn from(b: Arc<[u8]>) -> Self {
        Self::Bytes(b)
    }
}

impl From<Vec<u8>> for FunctionCode {
    fn from(b: Vec<u8>) -> Self {
        Self::Bytes(Arc::from(b))
    }
}

/// The language a file name implies, if any.
///
/// ONE table, three readers: the run-file validator, the synthetic metadata it
/// builds from a bare file name, and the sibling-module scan. They used to
/// carry three hand-rolled extension lists, which is how `.wasm` would have had
/// to be remembered three times — the mirrored-path bug class CLAUDE.md names.
///
/// `None` means "not a function file"; callers decide whether that is an error
/// (run-file) or simply a file to skip (module scan).
pub fn language_for_file_name(name: &str) -> Option<FunctionLanguage> {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".js") || lower.ends_with(".mjs") || lower.ends_with(".ts") {
        Some(FunctionLanguage::JavaScript)
    } else if lower.ends_with(".star") || lower.ends_with(".py") || lower.ends_with(".bzl") {
        Some(FunctionLanguage::Starlark)
    } else if lower.ends_with(".sql") {
        Some(FunctionLanguage::Sql)
    } else if lower.ends_with(".wasm") {
        Some(FunctionLanguage::Wasm)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_round_trips_and_bytes_refuse_text() {
        let text = FunctionCode::from("export function handler() {}".to_string());
        assert_eq!(text.as_text().unwrap(), "export function handler() {}");
        assert_eq!(text.len(), 28);
        assert!(!text.is_binary());

        let bytes = FunctionCode::from(vec![0x00, 0x61, 0x73, 0x6d]);
        assert!(bytes.is_binary());
        assert_eq!(bytes.as_bytes(), &[0x00, 0x61, 0x73, 0x6d]);
        let err = bytes.as_text().unwrap_err().to_string();
        assert!(err.contains("binary artifact"), "unexpected message: {err}");
    }

    #[test]
    fn the_over_cap_message_names_both_numbers() {
        assert!(check_wasm_artifact_size(10, 10).is_ok());
        let err = check_wasm_artifact_size(11, 10).unwrap_err().to_string();
        assert!(err.contains("11 bytes"), "{err}");
        assert!(err.contains("10-byte limit"), "{err}");
    }

    #[test]
    fn a_binary_artifact_never_renders_as_source() {
        let bytes = FunctionCode::from(vec![0xff; 10]);
        assert_eq!(bytes.to_display_string(), "<binary artifact: 10 bytes>");
    }

    #[test]
    fn file_names_map_to_languages() {
        for (name, expected) in [
            ("index.js", Some(FunctionLanguage::JavaScript)),
            ("index.mjs", Some(FunctionLanguage::JavaScript)),
            ("index.ts", Some(FunctionLanguage::JavaScript)),
            ("main.py", Some(FunctionLanguage::Starlark)),
            ("main.star", Some(FunctionLanguage::Starlark)),
            ("rules.bzl", Some(FunctionLanguage::Starlark)),
            ("report.sql", Some(FunctionLanguage::Sql)),
            ("main.wasm", Some(FunctionLanguage::Wasm)),
            ("MAIN.WASM", Some(FunctionLanguage::Wasm)),
            ("README.md", None),
            ("noextension", None),
        ] {
            assert_eq!(language_for_file_name(name), expected, "for {name}");
        }
    }
}
