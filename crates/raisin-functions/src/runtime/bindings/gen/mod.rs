// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Guest-SDK binding generator.
//!
//! The bindings registry is the single source of truth for the `raisin.*`
//! surface. QuickJS reads it through `api_wrapper.js`, Starlark generates its
//! whole namespace from it — and the WebAssembly guest SDKs generate theirs
//! here, so a new host method reaches Rust, Go and TypeScript in one step
//! instead of three hand-edited copies that drift.
//!
//! Run it with `cargo run -p raisin-functions --bin gen-bindings` (or
//! `make gen-bindings`); `--check` verifies the committed output is current,
//! which is what the freshness tests in [`tests`] assert.

pub mod assemblyscript;
mod dts;
pub mod go;
pub mod model;
pub mod rust;
pub mod ts;

#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

/// The WIT contract, compiled in so every copy is checked against the original.
pub const WIT_SOURCE: &str = include_str!("../../../../wit/raisin-function.wit");

/// Repo-relative path of the WIT source of truth.
pub const WIT_SOURCE_PATH: &str = "crates/raisin-functions/wit/raisin-function.wit";

/// What kind of artifact a [`GeneratedFile`] is, so each freshness test can
/// speak about its own slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputKind {
    /// Generated language bindings (Rust / Go / TypeScript declarations).
    Binding,
    /// A byte-identical copy of the WIT contract.
    Wit,
    /// A byte-identical copy of `quickjs/api_wrapper.js`.
    ApiWrapper,
}

/// One file the generator owns, start to finish.
pub struct GeneratedFile {
    /// Path relative to the repository root.
    pub path: &'static str,
    /// Full intended contents.
    pub contents: String,
    /// Which slice of the output this belongs to.
    pub kind: OutputKind,
}

/// Every file the generator writes.
pub fn render_all() -> Vec<GeneratedFile> {
    let mut files = vec![
        GeneratedFile {
            path: "sdks/rust/raisin-sdk/src/generated.rs",
            contents: rust::render(),
            kind: OutputKind::Binding,
        },
        GeneratedFile {
            path: "sdks/go/raisin/generated.go",
            contents: go::render(),
            kind: OutputKind::Binding,
        },
        GeneratedFile {
            path: "sdks/assemblyscript/assembly/generated.ts",
            contents: assemblyscript::render(),
            kind: OutputKind::Binding,
        },
        GeneratedFile {
            path: "sdks/ts/function-wasm/src/generated/raisin.d.ts",
            contents: ts::render_dts(),
            kind: OutputKind::Binding,
        },
        GeneratedFile {
            path: "packages/raisindb-functions-types/raisin.generated.d.ts",
            contents: dts::render_package_dts(),
            kind: OutputKind::Binding,
        },
        GeneratedFile {
            path: "sdks/ts/function-wasm/src/generated/api_wrapper.js",
            contents: ts::render_api_wrapper(),
            kind: OutputKind::ApiWrapper,
        },
    ];
    for path in WIT_COPIES {
        files.push(GeneratedFile {
            path,
            contents: WIT_SOURCE.to_string(),
            kind: OutputKind::Wit,
        });
    }
    files
}

/// SDK-local copies of the WIT contract. cargo-component, TinyGo and jco all
/// want the WIT inside the project, so the copies are unavoidable — the
/// freshness test is what stops them becoming a second source of truth.
pub const WIT_COPIES: &[&str] = &[
    "sdks/rust/raisin-sdk/wit/raisin-function.wit",
    "sdks/go/raisin/wit/raisin-function.wit",
    "sdks/ts/function-wasm/wit/raisin-function.wit",
    "sdks/assemblyscript/wit/raisin-function.wit",
];

/// The repository root, derived from this crate's manifest directory.
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("crates/raisin-functions must live two levels below the repo root")
        .to_path_buf()
}

/// Write every generated file, creating directories as needed.
/// Returns the repo-relative paths that actually changed.
pub fn write_all(root: &Path) -> std::io::Result<Vec<&'static str>> {
    let mut changed = Vec::new();
    for file in render_all() {
        let target = root.join(file.path);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let current = std::fs::read_to_string(&target).ok();
        if current.as_deref() != Some(file.contents.as_str()) {
            std::fs::write(&target, &file.contents)?;
            changed.push(file.path);
        }
    }
    Ok(changed)
}

/// Repo-relative paths whose committed contents differ from what the registry
/// would produce right now (a missing file counts as stale).
pub fn stale_files(root: &Path) -> Vec<&'static str> {
    render_all()
        .into_iter()
        .filter(|file| {
            std::fs::read_to_string(root.join(file.path))
                .ok()
                .as_deref()
                != Some(&file.contents)
        })
        .map(|file| file.path)
        .collect()
}

/// The command an out-of-date checkout should run.
pub const REGENERATE_HINT: &str =
    "run `make gen-bindings` (cargo run -p raisin-functions --bin gen-bindings) and commit the result";
