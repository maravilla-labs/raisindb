// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! FunctionApi trait definition
//!
//! This module defines the API surface exposed to user-defined functions.
//! The API mirrors the structure of raisin-client-js for familiarity.

use async_trait::async_trait;
use raisin_error::Result;
use serde_json::Value;

use super::callbacks::FunctionExecuteContext;

/// API surface exposed to functions
///
/// This trait defines all operations available to user-defined functions.
/// Implementations provide access to RaisinDB features in a sandboxed manner.
///
/// # JavaScript API Shape
///
/// In JavaScript functions, this is exposed as the `raisin` global object:
///
/// ```javascript
/// // Node operations
/// const node = await raisin.nodes.get("default", "/my-path");
/// await raisin.nodes.create("default", "/parent", { type: "Page", properties: {} });
/// await raisin.nodes.update("default", "/my-path", { properties: { title: "New" } });
/// await raisin.nodes.delete("default", "/my-path");
/// const nodes = await raisin.nodes.query("default", { node_type: "Page", limit: 10 });
///
/// // SQL operations
/// const result = await raisin.sql.query("SELECT * FROM nodes WHERE node_type = $1", ["Page"]);
/// const count = await raisin.sql.execute("UPDATE nodes SET properties = $1 WHERE id = $2", [props, id]);
///
/// // HTTP operations (allowlisted URLs only)
/// const response = await raisin.http.fetch("https://api.example.com/data", {
///     method: "POST",
///     body: { key: "value" },
///     headers: { "Authorization": "Bearer token" }
/// });
///
/// // AI operations
/// const response = await raisin.ai.completion({
///     model: "gpt-4o",
///     messages: [
///         { role: "system", content: "You are helpful" },
///         { role: "user", content: "Hello!" }
///     ],
///     temperature: 0.7,
///     max_tokens: 1000
/// });
/// const models = await raisin.ai.listModels();
/// const defaultModel = await raisin.ai.getDefaultModel("chat");
///
/// // Events
/// await raisin.events.emit("custom:event", { data: "value" });
///
/// // Logging
/// console.log("Info message");
/// console.error("Error message");
///
/// // Context
/// const tenant = raisin.context.tenant_id;
/// const branch = raisin.context.branch;
/// ```
#[async_trait]
pub trait FunctionApi: Send + Sync {
    // ========== Node Operations ==========

    /// Get a node by path
    async fn node_get(&self, workspace: &str, path: &str) -> Result<Option<Value>>;

    /// Get a node by ID
    async fn node_get_by_id(&self, workspace: &str, id: &str) -> Result<Option<Value>>;

    /// List a node's revision history (git-style "file history"), newest first.
    ///
    /// Returns an array of revision entries (`revision`, `updated_at`,
    /// `updated_by`, `deleted`). `limit` caps the number returned.
    async fn node_history(
        &self,
        workspace: &str,
        node_id: &str,
        limit: Option<u32>,
    ) -> Result<Vec<Value>>;

    /// Create a new node
    async fn node_create(&self, workspace: &str, parent_path: &str, data: Value) -> Result<Value>;

    /// Update a node
    async fn node_update(&self, workspace: &str, path: &str, data: Value) -> Result<Value>;

    /// Delete a node
    async fn node_delete(&self, workspace: &str, path: &str) -> Result<()>;

    /// Update a specific property on a node by path
    async fn node_update_property(
        &self,
        workspace: &str,
        node_path: &str,
        property_path: &str,
        value: Value,
    ) -> Result<()>;

    /// Move a node to a new parent path
    async fn node_move(
        &self,
        workspace: &str,
        node_path: &str,
        new_parent_path: &str,
    ) -> Result<Value>;

    /// Query nodes
    async fn node_query(&self, workspace: &str, query: Value) -> Result<Vec<Value>>;

    /// Get children of a node
    async fn node_get_children(
        &self,
        workspace: &str,
        parent_path: &str,
        limit: Option<u32>,
    ) -> Result<Vec<Value>>;

    /// Replay a parent's child order from `source_branch` onto `target_branch`.
    ///
    /// Used by selective publish flows to carry node sibling-order across
    /// branches (a per-node SQL UPSERT publish never touches the ordering index).
    async fn node_apply_child_order(
        &self,
        workspace: &str,
        parent_path: &str,
        source_branch: &str,
        target_branch: &str,
    ) -> Result<()>;

    /// Move a child to a position among its siblings (editorial ordering).
    ///
    /// Positions are 0-based and a position past the end appends. The ordering
    /// index owns the fractional order key — callers name a position, not a key.
    ///
    /// `before`/`after` name a sibling to sit next to instead of an index; see
    /// [`Self::node_move_child_relative`].
    async fn node_reorder_child(
        &self,
        workspace: &str,
        parent_path: &str,
        child_name: &str,
        position: u32,
    ) -> Result<()>;

    /// Move a child so it sits immediately before or after a named sibling.
    ///
    /// `before` selects which side: `true` places `child_name` before
    /// `reference_child_name`, `false` after it.
    async fn node_move_child_relative(
        &self,
        workspace: &str,
        parent_path: &str,
        child_name: &str,
        reference_child_name: &str,
        before: bool,
    ) -> Result<()>;

    // ========== SQL Operations ==========

    /// Execute a SQL query and return results
    async fn sql_query(&self, sql: &str, params: Vec<Value>) -> Result<Value>;

    /// Execute a SQL statement (INSERT, UPDATE, DELETE)
    async fn sql_execute(&self, sql: &str, params: Vec<Value>) -> Result<i64>;

    // ========== HTTP Operations ==========

    /// Make an HTTP request to an allowlisted URL
    async fn http_request(&self, method: &str, url: &str, options: Value) -> Result<Value>;

    // ========== Plugin Bindings ==========

    /// Dispatch a `raisin.<ns>.<method>` call contributed by a registered
    /// [`crate::plugin::FunctionBindingPlugin`]. `method` is the logical
    /// `"<ns>.<method>"` name and `args` is the guest's positional-args JSON
    /// array. Default: no plugin bound → error. Runs in trusted core and is NOT
    /// subject to the tenant `network_policy`.
    async fn plugin_call(&self, method: &str, _args: Value) -> Result<Value> {
        Err(raisin_error::Error::Validation(format!(
            "No plugin binding registered for '{}'",
            method
        )))
    }

    // ========== Event Operations ==========

    /// Emit a custom event
    async fn emit_event(&self, event_type: &str, data: Value) -> Result<()>;

    // ========== AI Operations ==========

    /// Call AI completion
    async fn ai_completion(&self, request: Value) -> Result<Value>;

    /// List available AI models
    async fn ai_list_models(&self) -> Result<Vec<Value>>;

    /// Get default model for a use case
    async fn ai_get_default_model(&self, use_case: &str) -> Result<Option<String>>;

    /// Generate embeddings for text or image input
    async fn ai_embed(&self, request: Value) -> Result<Value>;

    // ========== Resource Operations ==========

    /// Get binary data from storage by key
    async fn resource_get_binary(&self, storage_key: &str) -> Result<String>;

    /// Upload and attach a resource to a node property
    async fn node_add_resource(
        &self,
        workspace: &str,
        node_path: &str,
        property_path: &str,
        upload_data: Value,
    ) -> Result<Value>;

    // ========== PDF Operations ==========

    /// Process a PDF file from storage (storage-key based, no base64 overhead)
    async fn pdf_process_from_storage(&self, storage_key: &str, options: Value) -> Result<Value>;

    // ========== Task Operations ==========

    /// Create a human task in a user's inbox (fire-and-forget)
    async fn task_create(&self, request: Value) -> Result<Value>;

    /// Update a task
    async fn task_update(&self, task_id: &str, updates: Value) -> Result<Value>;

    /// Complete a task with a response
    async fn task_complete(&self, task_id: &str, response: Value) -> Result<Value>;

    /// Query tasks
    async fn task_query(&self, query: Value) -> Result<Vec<Value>>;

    // ========== Function Operations ==========

    /// Execute another function with tool call lifecycle management
    async fn function_execute(
        &self,
        function_path: &str,
        arguments: Value,
        context: FunctionExecuteContext,
    ) -> Result<Value>;

    /// Call another function directly (function-to-function calls)
    async fn function_call(&self, function_path: &str, arguments: Value) -> Result<Value>;

    // ========== Flow Operations ==========

    /// Start a `raisin:Flow` by path (fire-and-forget).
    ///
    /// Returns `{ instance_id, job_id, status: "queued" }` - the flow runs
    /// asynchronously via the job system; poll the flow instance API to
    /// observe progress.
    async fn flow_run(&self, flow_path: &str, input: Value) -> Result<Value> {
        let _ = input;
        Err(raisin_error::Error::Validation(format!(
            "Flow execution is not available in this runtime (flow: {})",
            flow_path
        )))
    }

    // ========== Scheduled Invocation Operations ==========

    /// Schedule a one-shot invocation of a function or flow at a fixed time.
    ///
    /// `request`: `{ targetKind, targetPath, input?, runAt (RFC3339),
    /// externalKey?, branch?, workspace?, maxRetries? }`.
    /// Returns `{ job_id, invocation_id, status: "scheduled", run_at }`.
    async fn scheduler_schedule(&self, request: Value) -> Result<Value> {
        let _ = request;
        Err(raisin_error::Error::Validation(
            "Scheduled invocations are not available in this runtime".to_string(),
        ))
    }

    /// Cancel a pending scheduled invocation by job id or external key.
    ///
    /// Returns `{ job_id, status: "cancelled" }`.
    async fn scheduler_cancel(&self, job_id_or_key: &str) -> Result<Value> {
        Err(raisin_error::Error::Validation(format!(
            "Scheduled invocations are not available in this runtime (id: {})",
            job_id_or_key
        )))
    }

    /// List this repository's scheduled invocations.
    ///
    /// `filter`: `{ externalKey?, status? }`. Returns `{ invocations: [...] }`.
    async fn scheduler_list(&self, filter: Value) -> Result<Value> {
        let _ = filter;
        Err(raisin_error::Error::Validation(
            "Scheduled invocations are not available in this runtime".to_string(),
        ))
    }

    /// Fetch a single scheduled invocation by job id or external key.
    async fn scheduler_get(&self, job_id_or_key: &str) -> Result<Value> {
        Err(raisin_error::Error::Validation(format!(
            "Scheduled invocations are not available in this runtime (id: {})",
            job_id_or_key
        )))
    }

    // ========== Branch Operations ==========

    /// Per-node diff of `branch` relative to `base_branch`'s merge-base.
    ///
    /// Returns `{ common_ancestor, added: [...], modified: [...], deleted: [...] }`
    /// where each entry is `{ node_id, workspace, path, operation, translation_locale? }`.
    async fn branch_diff(&self, branch: &str, base_branch: &str) -> Result<Value> {
        let _ = base_branch;
        Err(raisin_error::Error::Validation(format!(
            "Branch diff is not available in this runtime (branch: {})",
            branch
        )))
    }

    /// Branch divergence (ahead/behind counts) of `branch` relative to `base_branch`.
    ///
    /// Returns `{ ahead, behind, common_ancestor }`.
    async fn branch_compare(&self, branch: &str, base_branch: &str) -> Result<Value> {
        let _ = base_branch;
        Err(raisin_error::Error::Validation(format!(
            "Branch compare is not available in this runtime (branch: {})",
            branch
        )))
    }

    /// Copy a node set from `source_branch` onto `target_branch`, preserving
    /// node ids, in one atomic commit (branch promotion).
    ///
    /// `opts`: `{ workspace, roots: [paths], recursive?, deleteMissing? }`.
    /// Returns `{ copied, deleted, revision, changes: [...] }`.
    async fn branch_copy_nodes(
        &self,
        source_branch: &str,
        target_branch: &str,
        opts: Value,
    ) -> Result<Value> {
        let _ = (target_branch, opts);
        Err(raisin_error::Error::Validation(format!(
            "Branch node copy is not available in this runtime (branch: {})",
            source_branch
        )))
    }

    // ========== Transaction Operations ==========

    /// Begin a new transaction, returns transaction ID
    async fn tx_begin(&self) -> Result<String>;

    /// Commit a transaction
    async fn tx_commit(&self, tx_id: &str) -> Result<()>;

    /// Rollback a transaction
    async fn tx_rollback(&self, tx_id: &str) -> Result<()>;

    /// Set actor for a transaction
    async fn tx_set_actor(&self, tx_id: &str, actor: &str) -> Result<()>;

    /// Set message for a transaction
    async fn tx_set_message(&self, tx_id: &str, message: &str) -> Result<()>;

    /// Create a node within a transaction (auto-generates ID, builds path from parent+name)
    async fn tx_create(
        &self,
        tx_id: &str,
        workspace: &str,
        parent_path: &str,
        data: Value,
    ) -> Result<Value>;

    /// Add a node within a transaction (uses provided path, auto-generates ID if missing)
    async fn tx_add(&self, tx_id: &str, workspace: &str, data: Value) -> Result<Value>;

    /// Put a node within a transaction (create or update by ID)
    async fn tx_put(&self, tx_id: &str, workspace: &str, data: Value) -> Result<()>;

    /// Upsert a node within a transaction (create or update by PATH)
    async fn tx_upsert(&self, tx_id: &str, workspace: &str, data: Value) -> Result<()>;

    /// Create a node within a transaction with deep parent creation
    async fn tx_create_deep(
        &self,
        tx_id: &str,
        workspace: &str,
        parent_path: &str,
        data: Value,
        parent_node_type: &str,
    ) -> Result<Value>;

    /// Upsert a node within a transaction with deep parent creation
    async fn tx_upsert_deep(
        &self,
        tx_id: &str,
        workspace: &str,
        data: Value,
        parent_node_type: &str,
    ) -> Result<()>;

    /// Update a node within a transaction
    async fn tx_update(&self, tx_id: &str, workspace: &str, path: &str, data: Value) -> Result<()>;

    /// Delete a node by path within a transaction
    async fn tx_delete(&self, tx_id: &str, workspace: &str, path: &str) -> Result<()>;

    /// Delete a node by ID within a transaction
    async fn tx_delete_by_id(&self, tx_id: &str, workspace: &str, id: &str) -> Result<()>;

    /// Get a node by ID within a transaction
    async fn tx_get(&self, tx_id: &str, workspace: &str, id: &str) -> Result<Option<Value>>;

    /// Get a node by path within a transaction
    async fn tx_get_by_path(
        &self,
        tx_id: &str,
        workspace: &str,
        path: &str,
    ) -> Result<Option<Value>>;

    /// List children within a transaction
    async fn tx_list_children(
        &self,
        tx_id: &str,
        workspace: &str,
        parent_path: &str,
    ) -> Result<Vec<Value>>;

    /// Move a node to a new parent within a transaction
    async fn tx_move(
        &self,
        tx_id: &str,
        workspace: &str,
        node_path: &str,
        new_parent_path: &str,
    ) -> Result<Value>;

    /// Update a specific property on a node within a transaction
    async fn tx_update_property(
        &self,
        tx_id: &str,
        workspace: &str,
        node_path: &str,
        property_path: &str,
        value: Value,
    ) -> Result<()>;

    // ========== Date/Time Operations ==========

    /// Get current UTC datetime as ISO 8601 string
    fn date_now(&self) -> String;

    /// Get current Unix timestamp in seconds
    fn date_timestamp(&self) -> i64;

    /// Get current Unix timestamp in milliseconds
    fn date_timestamp_millis(&self) -> i64;

    /// Parse a date string to Unix timestamp
    fn date_parse(&self, date_str: &str, format: Option<&str>) -> Result<i64>;

    /// Format a Unix timestamp to string
    fn date_format(&self, timestamp: i64, format: Option<&str>) -> Result<String>;

    /// Add days to a timestamp
    fn date_add_days(&self, timestamp: i64, days: i64) -> Result<i64>;

    /// Get difference in days between two timestamps
    fn date_diff_days(&self, ts1: i64, ts2: i64) -> i64;

    // ========== Logging ==========

    /// Log a message
    fn log(&self, level: &str, message: &str);

    // ========== Context ==========

    /// Get the current execution context
    fn get_context(&self) -> Value;

    // ========== Admin Escalation ==========

    /// Check if this function is allowed to escalate to admin context.
    fn allows_admin_escalation(&self) -> bool;

    // ========== Admin Node Operations ==========

    /// Get a node by path (admin context - bypasses RLS)
    async fn admin_node_get(&self, workspace: &str, path: &str) -> Result<Option<Value>>;

    /// Get a node by ID (admin context - bypasses RLS)
    async fn admin_node_get_by_id(&self, workspace: &str, id: &str) -> Result<Option<Value>>;

    /// Create a new node (admin context - bypasses permission checks)
    async fn admin_node_create(
        &self,
        workspace: &str,
        parent_path: &str,
        data: Value,
    ) -> Result<Value>;

    /// Update a node (admin context - bypasses permission checks)
    async fn admin_node_update(&self, workspace: &str, path: &str, data: Value) -> Result<Value>;

    /// Delete a node (admin context - bypasses permission checks)
    async fn admin_node_delete(&self, workspace: &str, path: &str) -> Result<()>;

    /// Update a property (admin context - bypasses permission checks)
    async fn admin_node_update_property(
        &self,
        workspace: &str,
        node_path: &str,
        property_path: &str,
        value: Value,
    ) -> Result<()>;

    /// Query nodes (admin context - bypasses RLS)
    async fn admin_node_query(&self, workspace: &str, query: Value) -> Result<Vec<Value>>;

    /// Get children of a node (admin context - bypasses RLS)
    async fn admin_node_get_children(
        &self,
        workspace: &str,
        parent_path: &str,
        limit: Option<u32>,
    ) -> Result<Vec<Value>>;

    // ========== Admin SQL Operations ==========

    /// Execute a SQL query (admin context - bypasses RLS)
    async fn admin_sql_query(&self, sql: &str, params: Vec<Value>) -> Result<Value>;

    /// Execute a SQL statement (admin context - bypasses permission checks)
    async fn admin_sql_execute(&self, sql: &str, params: Vec<Value>) -> Result<i64>;

    // ========== Lock / Inventory Operations ==========

    /// Try once to acquire a lease-lock on `key`. Returns the lock guard
    /// (`{ key, token, expires_at_ms }`) on success or `None` if held.
    async fn lock_acquire(
        &self,
        key: &str,
        ttl_ms: i64,
        owner: Option<String>,
    ) -> Result<Option<Value>>;

    /// Release a lease-lock held with the given fencing `token`. Returns whether
    /// the lock was actually released (false if already gone / not owner).
    async fn lock_release(&self, key: &str, token: i64) -> Result<bool>;

    /// Renew (extend) a lease-lock held with `token`. Returns false if the lease
    /// was already lost.
    async fn lock_renew(&self, key: &str, token: i64, ttl_ms: i64) -> Result<bool>;

    /// Atomically claim `n` units from inventory `pool` (seeded to `capacity` on
    /// first touch). Returns `{ remaining }` on success or `None` if sold out.
    async fn inventory_claim(&self, pool: &str, n: i64, capacity: i64) -> Result<Option<Value>>;

    /// Return `n` units to inventory `pool`. Returns the new remaining count.
    async fn inventory_release(&self, pool: &str, n: i64) -> Result<i64>;

    // ========== Secret Operations ==========

    /// Read a secret's plaintext.
    ///
    /// `spec` is EITHER a bare name (`"stripe_key"`) or a full reference
    /// (`"secret://node/01H8XY/api_key@1"`). The reference form is what the
    /// primary use case hands the caller: a node property in an
    /// `encrypted: true` field stores the reference, node reads never resolve
    /// it, so the function passes that exact string straight back here. Parsing
    /// goes through [`raisin_models::secret_ref`], never a hand-rolled split —
    /// a name may itself contain `@`.
    ///
    /// A version pinned in the reference is honoured. Passing an explicit
    /// `version` argument AS WELL is an error, not a precedence question: two
    /// stated versions cannot both be satisfied, and silently picking either
    /// could return a value an old node revision never held. Every call is
    /// gated by the
    /// function's [`SecretPolicy`](crate::types::SecretPolicy) on the PARSED
    /// name, BEFORE the store is touched, so the two spellings of one secret
    /// cannot get different answers. A denial is a hard error naming the secret
    /// — never an empty result a caller could mistake for "no such secret".
    ///
    /// The returned plaintext is the caller's to handle; it is never logged.
    async fn secret_get(&self, spec: &str, version: Option<i64>) -> Result<String>;

    /// Resolve a property value that MAY be a `secret://` reference.
    ///
    /// Returns the plaintext when `value` is a reference, and `value` unchanged
    /// when it is not — so a connector config field that is a literal on one
    /// deployment and a vaulted reference on another is consumed the same way.
    /// A reference that fails to resolve is an ERROR, never the input echoed
    /// back: that fallback would send the literal text `secret://stripe_key` to
    /// a provider as a credential.
    async fn secret_resolve(&self, value: &str) -> Result<String>;

    /// Append a new version of `name` and return `{ name, version }`.
    ///
    /// `name` accepts the reference spelling too, but a PINNED reference is
    /// refused: a write always appends, so `secret://k@1` cannot mean what it
    /// says. Never overwrites — a prior version stays readable through a pinned
    /// reference forever.
    async fn secret_put(&self, name: &str, value: &str) -> Result<Value>;

    /// Metadata for the newest version of every secret this function may see.
    ///
    /// Returns [`SecretMetadata`](raisin_rocksdb::secret_store::SecretMetadata)
    /// shapes — name, version, timestamps, tombstone flag — and NEVER ciphertext
    /// or plaintext. Filtered to names the policy allows, so a function cannot
    /// enumerate credentials it may not read.
    async fn secret_list(&self) -> Result<Vec<Value>>;

    /// Append a new version of `name`, stamped as a rotation. Returns
    /// `{ name, version }`. Anything holding a pinned `secret://name@N` keeps
    /// resolving to the old version.
    async fn secret_rotate(&self, name: &str, value: &str) -> Result<Value>;

    /// Append a tombstone for `name`. Prior versions stay readable through a
    /// pinned reference (MVCC time-travel reads depend on it). Returns
    /// `{ name, version }` — the tombstone's ordinal.
    async fn secret_delete(&self, name: &str) -> Result<Value>;

    // ========== Integration / Mount Operations ==========

    /// Trigger a "sync now" for a virtual mount (connector).
    ///
    /// Enqueues a one-shot `VirtualMountSync` job for `mount_id`, deduped per
    /// mount so a webhook storm collapses into a single in-flight sync. `mode`
    /// defaults to `"delta"` (pass `"full"` for a full re-sync).
    ///
    /// Returns `{ "job_id": String|null, "status": "queued"|"already_running" }`;
    /// `job_id` is `null` when an in-flight sync deduped the request.
    async fn integrations_sync_now(&self, mount_id: &str, mode: Option<&str>) -> Result<Value>;

    // ========== IMAP Operations (native protocol binding) ==========

    /// Fetch messages newer than a UID cursor from an IMAP mailbox.
    ///
    /// `conn` is `{ host, port, tls, username, password }` (password may be an
    /// app password or an OAuth2 access token — never logged). `since_uid` is
    /// the last-seen UID; only messages with a strictly greater UID are
    /// returned. `opts` may carry `{ mailbox?: string, limit?: number }`
    /// (mailbox defaults to `INBOX`, limit defaults to 200 and is hard-capped).
    ///
    /// Returns `{ messages: [...], highestUid, uidvalidity }`. The connection's
    /// host:port MUST be authorized by the function's network policy; an
    /// unauthorized host is refused before any socket is opened.
    async fn imap_fetch_since(
        &self,
        conn: Value,
        since_uid: i64,
        opts: Option<Value>,
    ) -> Result<Value>;

    /// List all mailboxes (folders) on an IMAP account. See
    /// [`imap_fetch_since`](Self::imap_fetch_since) for `conn` and policy rules.
    async fn imap_list_mailboxes(&self, conn: Value) -> Result<Value>;

    /// Fetch one full message by UID (headers, text, html, flags). `opts` may
    /// carry `{ mailbox?: string }`. See [`imap_fetch_since`](Self::imap_fetch_since)
    /// for `conn` and policy rules.
    async fn imap_fetch_message(&self, conn: Value, uid: i64, opts: Option<Value>)
        -> Result<Value>;

    // ========== Crypto Operations (native primitives) ==========

    /// Verify an RS256/ES256-signed JWT/OIDC token against a JWKS.
    ///
    /// `opts` is `{ jwks_url?, issuer?, audience?, algorithms? }`. The JWKS is
    /// fetched from `jwks_url` (host authorized against the function's network
    /// policy BEFORE any socket is opened, then briefly cached); the signature,
    /// `exp`/`nbf`, and — when supplied — `iss`/`aud` are checked.
    ///
    /// Returns `{ valid: bool, claims?: object, error?: string }`. An invalid
    /// token is reported as `{ valid: false, error }` (not a hard error); a hard
    /// error is reserved for policy denial and an unreachable JWKS endpoint. The
    /// token and decoded claims are never logged.
    ///
    /// This is a general primitive (any connector may use it to authenticate a
    /// signed push/webhook); it is not provider-specific.
    async fn crypto_verify_jwt(&self, token: &str, opts: Value) -> Result<Value>;
}
