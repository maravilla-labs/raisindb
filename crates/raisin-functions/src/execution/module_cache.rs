// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! Cache for a function's resolved module set.
//!
//! Loading the modules a function can import is not one read — it is a prefix
//! scan over the function's whole subtree, plus one more scan per `../dir/` its
//! code (or any sibling's code) imports, plus a binary-storage `get` per file
//! that isn't stored inline. All of it repeated on **every invocation**, for a
//! set of files that changes only when someone edits the function.
//!
//! # Why this is on by default
//!
//! Stale *code* is the worst failure mode this codebase could hand someone: a
//! developer saves a file and the old version keeps running, silently. So being
//! on by default is only justified if every path that writes function code
//! invalidates. That was audited rather than assumed, and it holds:
//!
//! - local writes via `NodeService` and via transaction commit — event
//! - SQL DML, HTTP/WS node writes, flow node callbacks — event
//! - CLI `deploy` → package install job (transactional) — event
//! - replication of ordinary operations — event, stamped `source=replication`
//!
//! The one path that writes function code with **no** event is replication
//! checkpoint (SST) ingestion, which copies column families straight into the
//! live database. That is covered separately, by registering [`invalidate_all`]
//! with `raisin_core`'s derived-cache registry, which the ingestor calls.
//!
//! The remaining hole is overwriting a binary blob out-of-band (directly in the
//! filesystem or S3 bucket). No in-repo code path can do it: `BinaryStorage`
//! always mints a fresh key on write, which forces a node property rewrite and
//! therefore an event.
//!
//! The TTL is a floor for whatever that audit missed, not the correctness
//! mechanism. Set `RAISIN_FUNCTION_MODULE_CACHE_TTL_SECS=0` to disable the cache
//! entirely; behaviour is then byte-identical to before this module existed.
//!
//! The function's own entry node and code are deliberately NOT cached here —
//! those are two point-reads, and keeping them live means a metadata change is
//! always seen even when this cache is on.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

/// Resolved module set: relative module path -> source code.
pub type ModuleSet = Arc<HashMap<String, String>>;

/// Safety floor for how long a module set may be stale if some write path turns
/// out not to emit an event. Invalidation is event-driven; this is the backstop.
const DEFAULT_TTL_SECS: u64 = 300;

/// TTL for the module cache. `RAISIN_FUNCTION_MODULE_CACHE_TTL_SECS=0` disables
/// caching entirely.
fn cache_ttl() -> Duration {
    static TTL: std::sync::LazyLock<Duration> = std::sync::LazyLock::new(|| {
        let secs = std::env::var("RAISIN_FUNCTION_MODULE_CACHE_TTL_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(DEFAULT_TTL_SECS);

        if secs == 0 {
            tracing::info!("Function module cache DISABLED by configuration");
        }
        Duration::from_secs(secs)
    });
    *TTL
}

/// Whether the cache is enabled at all.
pub fn is_enabled() -> bool {
    !cache_ttl().is_zero()
}

static MODULE_CACHE: std::sync::OnceLock<raisin_core::TtlCache<ModuleSet>> =
    std::sync::OnceLock::new();

fn module_cache() -> &'static raisin_core::TtlCache<ModuleSet> {
    MODULE_CACHE.get_or_init(|| {
        // Replication checkpoint ingestion writes function code straight into
        // the live database with no events, so the node-event path below cannot
        // see it. Without this hook such a replica keeps executing the code it
        // had before catching up.
        raisin_core::register_invalidator(invalidate_all);
        raisin_core::TtlCache::new(cache_ttl())
    })
}

/// Cache key. Prefix-compatible with [`invalidate_workspace_functions`], which
/// relies on everything up to and including the workspace being a stable prefix.
fn cache_key(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    function_path: &str,
) -> String {
    format!("{tenant_id}:{repo_id}:{branch}:{workspace}:{function_path}")
}

/// Look up a previously resolved module set.
pub fn get(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    function_path: &str,
) -> Option<ModuleSet> {
    if !is_enabled() {
        return None;
    }
    module_cache().get(&cache_key(
        tenant_id,
        repo_id,
        branch,
        workspace,
        function_path,
    ))
}

/// Store a resolved module set.
pub fn put(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    function_path: &str,
    modules: ModuleSet,
) {
    if !is_enabled() {
        return;
    }
    module_cache().put(
        &cache_key(tenant_id, repo_id, branch, workspace, function_path),
        modules,
    );
}

/// Drop one function's cached module set.
pub fn invalidate_function_modules(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    function_path: &str,
) {
    module_cache().invalidate(&cache_key(
        tenant_id,
        repo_id,
        branch,
        workspace,
        function_path,
    ));
}

/// Drop every cached module set in a workspace.
///
/// This is what node events use. A per-function invalidation would be wrong:
/// editing `shared/util.js` must invalidate every function that imports it, and
/// the import graph is exactly what the cache is holding — so the changed node's
/// path cannot tell us which entries are affected. Dropping the workspace is the
/// only conservative answer, and it is cheap: the next invocation re-resolves.
pub fn invalidate_workspace_functions(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
) {
    let prefix = format!("{tenant_id}:{repo_id}:{branch}:{workspace}:");
    module_cache().invalidate_prefix(&prefix);
    tracing::debug!(scope = %prefix, "Function module cache invalidated");
}

/// Drop every cached module set. For tests and administrative resets.
pub fn invalidate_all() {
    module_cache().invalidate_all();
}
