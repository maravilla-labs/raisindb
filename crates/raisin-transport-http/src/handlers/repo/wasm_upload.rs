// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Refuse a WebAssembly artifact that could never run, at the moment it is
//! uploaded.
//!
//! # Why here rather than at invocation
//!
//! A `.wasm` uploaded under a `raisin:Function` is that function's *code*. Left
//! unchecked it is accepted with a 200, and the failure surfaces later as a
//! runtime error on whatever trigger, job or HTTP call first reaches it —
//! usually to someone who did not upload it. The engine can answer the
//! question in milliseconds, so the author who is standing right there gets the
//! answer instead: a 400 naming the missing export or the unlinkable import.
//!
//! # It is the SAME check the runtime performs
//!
//! `raisin_functions::validate_component_async` is the compile the cache-miss
//! path takes, so an artifact accepted here cannot be rejected at run time (or
//! the other way round). A build without the `wasm` feature gets the accepting
//! stub behind that same name — no `cfg` here. It is the ASYNC form because
//! this is a tokio worker and Cranelift is seconds of CPU.
//!
//! # Not every `.wasm` is a function
//!
//! Only an upload landing directly under a `raisin:Function` node is a
//! function artifact. A `.wasm` uploaded as ordinary content — a browser
//! module served to a front-end, say — is none of this module's business and
//! passes through untouched.

use raisin_binary::BinaryStorage;
use raisin_core::NodeService;
use raisin_storage::{transactional::TransactionalStorage, Storage};

use crate::{error::ApiError, state::AppState};

/// Node type an upload must sit under before its bytes are treated as code.
const FUNCTION_NODE_TYPE: &str = "raisin:Function";

/// Validate a just-stored upload, deleting the blob and failing with 400 if it
/// is a `.wasm` under a `raisin:Function` that this host could not run.
///
/// `node_path` is the path the asset node will occupy; its parent is the
/// candidate function. Returns `Ok(())` for everything that is not a function
/// artifact, which is the overwhelming majority of uploads.
pub(super) async fn reject_unrunnable_wasm_upload<S: Storage + TransactionalStorage + 'static>(
    state: &AppState,
    nodes_svc: &NodeService<S>,
    node_path: &str,
    file_name: Option<&str>,
    stored: &raisin_binary::StoredObject,
) -> Result<(), ApiError> {
    let name = artifact_name(node_path, file_name);
    if !is_wasm_file_name(&name) {
        return Ok(());
    }

    let Some(parent_path) = parent_path(node_path) else {
        return Ok(());
    };
    match nodes_svc.get_by_path(&parent_path).await {
        Ok(Some(parent)) if parent.node_type == FUNCTION_NODE_TYPE => {}
        // Anything else — a plain folder, a missing parent, a lookup the
        // caller is not allowed to make — means this is not function code, and
        // an upload is not the place to re-litigate that.
        _ => return Ok(()),
    }

    // Never read an unbounded blob into memory to inspect it: the size cap is
    // the runtime's own (`[functions.wasm] max_artifact_bytes`), asked through
    // the one accessor, so an artifact refused here would have been refused at
    // load time too.
    let max = raisin_functions::max_wasm_artifact_bytes();
    if let Err(e) = raisin_functions::check_wasm_artifact_size(stored.size as usize, max) {
        return Err(reject(state, stored, e.to_string()).await);
    }

    let bytes = match state.bin.get(&stored.key).await {
        Ok(bytes) => bytes,
        Err(e) => {
            // The blob is unreadable a moment after it was written; deleting it
            // is the honest outcome, and the upload failed, not the artifact.
            let _ = state.bin.delete(&stored.key).await;
            return Err(ApiError::internal(format!(
                "Stored artifact could not be read back for validation: {e}"
            )));
        }
    };

    // `validate_component_async`, never the synchronous form: this runs on a
    // tokio worker, and the compile behind it is seconds of Cranelift on a
    // 12 MiB jco artifact. The async form hands that to `spawn_blocking`, so a
    // burst of uploads cannot occupy every worker in the runtime.
    if let Err(e) =
        raisin_functions::validate_component_async(std::sync::Arc::from(&bytes[..])).await
    {
        return Err(reject(state, stored, e.to_string()).await);
    }

    tracing::debug!(
        path = %node_path,
        size = stored.size,
        "Validated WebAssembly function artifact on upload"
    );
    Ok(())
}

/// Delete the orphaned blob and build the 400.
///
/// The node is never created, so leaving the bytes behind would be a leak that
/// nothing references and nothing collects.
async fn reject(
    state: &AppState,
    stored: &raisin_binary::StoredObject,
    message: String,
) -> ApiError {
    if let Err(e) = state.bin.delete(&stored.key).await {
        tracing::warn!(
            key = %stored.key,
            error = %e,
            "Could not delete the blob of a rejected wasm upload"
        );
    }
    tracing::info!(
        key = %stored.key,
        reason = %message,
        "Rejected a WebAssembly function artifact on upload"
    );
    ApiError::new(
        axum::http::StatusCode::BAD_REQUEST,
        "INVALID_WASM_COMPONENT",
        format!("This WebAssembly artifact cannot run on this server: {message}"),
    )
}

/// The file name an upload should be judged by: the multipart file name if
/// there is one, else the last segment of the node path.
fn artifact_name(node_path: &str, file_name: Option<&str>) -> String {
    file_name
        .map(|s| s.to_string())
        .or_else(|| node_path.rsplit('/').next().map(|s| s.to_string()))
        .unwrap_or_default()
}

/// Whether a file name names a WebAssembly artifact (case-insensitive).
fn is_wasm_file_name(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with(".wasm")
}

/// The parent of `node_path`, or `None` at the root.
fn parent_path(node_path: &str) -> Option<String> {
    let trimmed = node_path.trim_end_matches('/');
    let (parent, _) = trimmed.rsplit_once('/')?;
    if parent.is_empty() {
        None
    } else {
        Some(parent.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_wasm_names_are_candidates() {
        assert!(is_wasm_file_name("main.wasm"));
        assert!(is_wasm_file_name("MAIN.WASM"));
        assert!(!is_wasm_file_name("main.wasm.txt"));
        assert!(!is_wasm_file_name("index.js"));
        assert!(!is_wasm_file_name("wasm"));
    }

    #[test]
    fn the_multipart_file_name_wins_over_the_path() {
        assert_eq!(
            artifact_name("/lib/greet/upload_tmp", Some("main.wasm")),
            "main.wasm"
        );
        assert_eq!(artifact_name("/lib/greet/main.wasm", None), "main.wasm");
        assert_eq!(artifact_name("", None), "");
    }

    #[test]
    fn the_candidate_function_is_the_parent_not_the_file() {
        assert_eq!(
            parent_path("/lib/greet/main.wasm").as_deref(),
            Some("/lib/greet")
        );
        // A file at the workspace root has no function above it.
        assert_eq!(parent_path("/main.wasm"), None);
        assert_eq!(parent_path("main.wasm"), None);
    }
}
