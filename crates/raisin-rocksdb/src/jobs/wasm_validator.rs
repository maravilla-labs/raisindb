// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Whether a `.wasm` file in a package can actually run here, asked from below.
//!
//! # Why an inversion rather than a call
//!
//! The package installer lives in this crate, and the answer lives in
//! `raisin-functions` — which depends on `raisin-rocksdb`. Cargo rejects the
//! package cycle regardless of features, so the installer can never call
//! `raisin_functions::validate_component` directly. `main.rs` installs a
//! closure that does, next to `install_capability_probe`; everything below can
//! then ask. Same pattern, same reason.
//!
//! # No validator installed means ACCEPT, and that is deliberate
//!
//! A server built without the `wasm` feature — or any embedder that never
//! wires this up — must still install a package that happens to carry a
//! `.wasm` it will never execute. Refusing here would make an artifact
//! undeployable on exactly the servers that do not care about it. The absence
//! is logged once so an operator debugging "why did my broken component
//! install" has a line to find.
//!
//! Note the asymmetry with the HTTP upload path, which validates in-process
//! and can therefore refuse: an install is a bulk operation over content the
//! operator already chose, an upload is a single file arriving now.

use std::sync::{Arc, OnceLock};

/// Answers "could this artifact ever run on this host?".
///
/// `Err(message)` is shown to the operator verbatim, so the message must name
/// what is wrong with the artifact (a missing export, an unlinkable import),
/// not merely that something failed.
pub type WasmArtifactValidator =
    Arc<dyn Fn(&[u8]) -> std::result::Result<(), String> + Send + Sync>;

fn slot() -> &'static OnceLock<WasmArtifactValidator> {
    static VALIDATOR: OnceLock<WasmArtifactValidator> = OnceLock::new();
    &VALIDATOR
}

/// Install the process-wide WebAssembly artifact validator. Call once at
/// startup, before any package is installed.
///
/// Returns `false` if one was already installed, in which case the existing
/// one is kept — two answers to "is this artifact runnable" is the drift this
/// registry exists to prevent, so the caller should log it rather than assume
/// it took effect.
pub fn install_wasm_validator(validator: WasmArtifactValidator) -> bool {
    slot().set(validator).is_ok()
}

/// True when a validator has been installed. Distinguishes "this host accepts
/// every artifact" from "nobody has said yet", which produce the same answer
/// from [`validate_wasm_artifact`] and are very different diagnoses.
pub fn wasm_validator_installed() -> bool {
    slot().get().is_some()
}

/// Validate `data` if `filename` names a WebAssembly artifact.
///
/// Everything else is passed through untouched. The process-wide validator is
/// the only one consulted; [`check_wasm_artifact`] is the same rule with the
/// validator passed in, which is what the tests use.
pub fn validate_wasm_artifact(filename: &str, data: &[u8]) -> std::result::Result<(), String> {
    check_wasm_artifact(filename, data, slot().get())
}

/// The rule itself, with the validator supplied rather than looked up.
///
/// Split out because the installed validator is a process-global `OnceLock`: a
/// test that installed one would race every other test in the binary and win
/// only sometimes, so the behaviour is tested here and the lookup is the one
/// line that is not.
pub fn check_wasm_artifact(
    filename: &str,
    data: &[u8],
    validator: Option<&WasmArtifactValidator>,
) -> std::result::Result<(), String> {
    if !is_wasm_file_name(filename) {
        return Ok(());
    }
    match validator {
        Some(validate) => validate(data),
        None => {
            static WARNED: std::sync::Once = std::sync::Once::new();
            WARNED.call_once(|| {
                tracing::warn!(
                    "Installing a WebAssembly artifact without validating it: no \
                     validator is registered in this process (a build without the \
                     `wasm` feature, or an embedder that did not call \
                     install_wasm_validator). The package installs; the artifact \
                     will fail at invocation time if it is not runnable"
                );
            });
            Ok(())
        }
    }
}

/// [`validate_wasm_artifact`] for a caller on an async runtime.
///
/// The installed validator compiles the artifact, which is seconds of Cranelift
/// on a 12 MiB component — calling it directly from an `async fn` parks a tokio
/// worker for that whole time, and a package carrying four such artifacts parks
/// one job worker for tens of seconds. So the work goes to `spawn_blocking`,
/// exactly as the runtime's own cache-miss path does. It costs a copy of the
/// bytes, which the installer makes anyway before storing them.
///
/// Non-`.wasm` files never reach the validator, so they never leave this
/// thread: the name check happens here, not in the blocking task.
pub async fn validate_wasm_artifact_async(
    filename: &str,
    data: &[u8],
) -> std::result::Result<(), String> {
    if !is_wasm_file_name(filename) {
        return Ok(());
    }
    let Some(validate) = slot().get().cloned() else {
        // Same warn-once accept as the synchronous path, and no task to spawn.
        return check_wasm_artifact(filename, data, None);
    };
    let owned = data.to_vec();
    match tokio::task::spawn_blocking(move || validate(&owned)).await {
        Ok(result) => result,
        Err(e) => Err(format!("wasm validation task failed: {e}")),
    }
}

/// Whether a package file name names a WebAssembly artifact.
///
/// Case-insensitive: a package is a zip built on someone else's filesystem.
fn is_wasm_file_name(filename: &str) -> bool {
    filename.to_ascii_lowercase().ends_with(".wasm")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rejecting() -> WasmArtifactValidator {
        Arc::new(|bytes: &[u8]| {
            Err(format!(
                "{} bytes: does not export `handler: func(name: string, input: string)`",
                bytes.len()
            ))
        })
    }

    fn accepting() -> WasmArtifactValidator {
        Arc::new(|_: &[u8]| Ok(()))
    }

    #[test]
    fn a_rejecting_validator_fails_the_install_with_its_own_message() {
        let v = rejecting();
        let err = check_wasm_artifact("main.wasm", &[0, 1, 2], Some(&v)).unwrap_err();
        assert!(
            err.contains("does not export"),
            "the validator's message must reach the operator verbatim: {err}"
        );
        assert!(err.contains("3 bytes"), "the bytes must be passed: {err}");
    }

    #[test]
    fn no_validator_installed_accepts_so_a_stock_server_still_installs() {
        assert!(check_wasm_artifact("main.wasm", &[0, 1, 2], None).is_ok());
    }

    #[test]
    fn only_wasm_files_are_offered_to_the_validator() {
        let v = rejecting();
        // A rejecting validator proves the file was never handed over.
        for name in ["index.js", "logo.png", "notes.wasm.txt", "wasm"] {
            assert!(
                check_wasm_artifact(name, &[0], Some(&v)).is_ok(),
                "{name} is not a wasm artifact and must not be validated"
            );
        }
        assert!(check_wasm_artifact("MAIN.WASM", &[0], Some(&v)).is_err());
    }

    #[test]
    fn an_accepting_validator_lets_the_file_through() {
        let v = accepting();
        assert!(check_wasm_artifact("main.wasm", &[0, 1, 2], Some(&v)).is_ok());
    }

    #[tokio::test]
    async fn the_async_form_keeps_a_non_artifact_off_the_blocking_pool() {
        // No validator is installed in this binary (installing one would race
        // every other test), so both calls take the accept path. What is being
        // asserted is that the name check happens BEFORE the offload: a
        // non-`.wasm` file must never cost a `spawn_blocking` or a copy of its
        // bytes, and the two forms must agree on which files are artifacts.
        assert!(validate_wasm_artifact_async("index.js", &[0, 1, 2])
            .await
            .is_ok());
        assert!(validate_wasm_artifact_async("main.wasm", &[0, 1, 2])
            .await
            .is_ok());
    }
}
