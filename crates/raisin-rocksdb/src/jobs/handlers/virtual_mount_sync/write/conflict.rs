//! What a REFUSED push means: the mount's conflict policy, and the pluggable
//! resolver behind it.
//!
//! A conflict is the provider saying "this object changed since you last read
//! it". The engine cannot decide what that means — whether the local edit is the
//! important one, whether the two can be combined, or whether a human has to
//! look — so the mount decides, and for anything beyond the three fixed answers
//! a package-shipped function decides.
//!
//! ## Why `remote_wins` is the default
//!
//! It is what the engine did before this module existed: abandon the intent, let
//! the next delta bring the remote value in and overwrite the local one. It
//! loses the local edit, which is the *recoverable* half of the trade — the
//! remote object is untouched and the user can redo the edit. `local_wins` is
//! the other half: it re-sends the push with no concurrency base at all, which
//! **overwrites whatever the other writer did**, unrecoverably. That is why it
//! is never a default at any layer, exactly as `purge` is never a default in
//! [`super::policy`].
//!
//! ## The resolver
//!
//! Invoked through the SAME plumbing as the mapper (`ctx.invoker.invoke`), so
//! there is no second executor, no second workspace and no second sandbox to
//! reason about. Like `mapping_function` — and unlike `adapter_function` — it
//! has **no integration-level fallback**: a conflict resolver is a statement
//! about one mount's data, and inheriting one from a connector shared by twenty
//! mounts would apply a merge rule to nineteen mounts that never asked for it.
//!
//! Every unclear answer PARKS. A throw, an unrecognized `resolution`, a
//! `merged` with nothing mergeable — all of them leave the edit pending and say
//! so, because the alternative is a silent drop of a user's edit at the one
//! moment the engine already knows something is wrong.

use serde_json::{json, Map, Value};

use super::super::{AdapterError, MountConfig, SyncCtx};

/// What this mount does when the provider refuses a push.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ConflictPolicy {
    /// Abandon the intent; the next delta overwrites the local value. The
    /// default, and today's behaviour.
    RemoteWins,
    /// Re-send the push with NO concurrency base, overwriting the other writer.
    LocalWins,
    /// Park the intent and put the mount into `writeback_status: "conflict"`.
    Error,
    /// Ask this function node what to do.
    Resolver(String),
}

impl ConflictPolicy {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::RemoteWins => "remote_wins",
            Self::LocalWins => "local_wins",
            Self::Error => "error",
            Self::Resolver(_) => "resolver_function",
        }
    }
}

/// What one conflict resolved to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Resolution {
    /// Re-send the original payload with `etag: null`.
    ForceLocal,
    /// Abandon. Nothing is stamped, so the node stays diverged until the next
    /// delta brings the remote value in.
    Abandon,
    /// Push these node-shaped field values, and land them locally too.
    Merged(Map<String, Value>),
    /// Leave the edit pending and surface the reason.
    Park(String),
}

/// Resolve the effective conflict policy for one mount.
///
/// Errors are REFUSALS, never fallbacks — the same rule as
/// [`super::policy::resolve_delete_policy`]. A typo that silently resolved to
/// `remote_wins` would drop edits on a mount whose operator had explicitly asked
/// for them to be kept, and the operator would have no way to tell.
pub(crate) fn resolve_conflict_policy(mount: &MountConfig) -> Result<ConflictPolicy, String> {
    let resolver = mount
        .resolver_function
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let raw = mount
        .write_config
        .conflict
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let Some(raw) = raw else {
        // A `resolver_function` with no `conflict` value is not ambiguous: there
        // is no other reason to point a mount at one. Requiring both would be a
        // configuration that reads as complete, behaves as `remote_wins`, and
        // says nothing — which is the failure this whole module exists to stop.
        return Ok(match resolver {
            Some(path) => ConflictPolicy::Resolver(path.to_string()),
            None => ConflictPolicy::RemoteWins,
        });
    };

    match raw.to_ascii_lowercase().as_str() {
        "remote_wins" => Ok(ConflictPolicy::RemoteWins),
        "local_wins" => Ok(ConflictPolicy::LocalWins),
        "error" => Ok(ConflictPolicy::Error),
        "resolver_function" => match resolver {
            Some(path) => Ok(ConflictPolicy::Resolver(path.to_string())),
            // Not a fallback to `remote_wins`: an operator who typed
            // `resolver_function` has a merge rule in mind, and running without
            // it would discard exactly the edits the rule was written to keep.
            None => Err(
                "write_config.conflict is 'resolver_function' but the mount sets no \
                 resolver_function path"
                    .to_string(),
            ),
        },
        _ => Err(format!(
            "write_config.conflict '{raw}' is not recognized; expected 'remote_wins', \
             'local_wins', 'error' or 'resolver_function'"
        )),
    }
}

/// Decide what to do about ONE refused push.
///
/// `fields` is the mount's EFFECTIVE ALLOW-LIST — what this mount may write at
/// all — never the diverged subset that happened to go on the wire this attempt.
/// `watched` is the node's current values and `last_pushed` what the provider
/// last agreed to; together they are the `field_diff` a resolver reasons over.
///
/// The distinction was a real defect. This used to receive the diverged subset,
/// so a resolver answering "keep the local subject AND restore the remote
/// category" had the category half dropped as "outside this mount's write
/// allow-list" when it was squarely inside it — and in the degenerate case every
/// merged key was dropped and the intent parked with a reason that blamed the
/// mount's configuration for the engine's own narrowing.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn resolve_one(
    ctx: &SyncCtx<'_>,
    policy: &ConflictPolicy,
    node: &Value,
    fields: &[String],
    watched: &Map<String, Value>,
    last_pushed: Option<&Map<String, Value>>,
    base_etag: Option<&str>,
    message: &str,
) -> Resolution {
    match policy {
        ConflictPolicy::RemoteWins => Resolution::Abandon,
        ConflictPolicy::LocalWins => Resolution::ForceLocal,
        ConflictPolicy::Error => Resolution::Park(format!(
            "conflict policy is 'error': {message}. The edit stays pending; nothing was \
             written at the provider."
        )),
        ConflictPolicy::Resolver(path) => {
            let input = json!({
                "operation": "resolve_conflict",
                "local": node,
                // The engine has NO adapter `get` operation, so the remote
                // object is genuinely not in scope here — see
                // `docs/reference/virtual-node-adapters.md`. Passing `null`
                // rather than inventing one: a resolver that needs the remote
                // side must read it from `conflict`, which carries the
                // provider's own message verbatim.
                "remote": Value::Null,
                "conflict": message,
                "base_etag": base_etag,
                "field_diff": field_diff(fields, watched, last_pushed),
                "mount": ctx.mount_snapshot,
            });
            match ctx.invoker.invoke(&ctx.scope, path, input).await {
                Ok(out) => interpret(ctx, path, fields, out),
                // A throw is `Transient` at the adapter boundary, and a
                // transient failure of the thing that decides what happens to a
                // user's edit is not a licence to pick one. Park.
                Err(e) => Resolution::Park(format!(
                    "conflict resolver '{path}' failed ({e}); the edit stays pending"
                )),
            }
        }
    }
}

/// Turn a resolver's answer into a [`Resolution`].
fn interpret(ctx: &SyncCtx<'_>, path: &str, fields: &[String], out: Value) -> Resolution {
    let resolution = out
        .get("resolution")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    match resolution.as_str() {
        "local_wins" => Resolution::ForceLocal,
        "remote_wins" => Resolution::Abandon,
        "park" => Resolution::Park(format!(
            "conflict resolver '{path}' parked this edit{}",
            out.get("reason")
                .and_then(|v| v.as_str())
                .map(|r| format!(": {r}"))
                .unwrap_or_default()
        )),
        "merged" => merged_fields(ctx, path, fields, &out),
        // Includes an absent `resolution`, a non-string one, and a value from a
        // newer resolver than this engine. Parking is the only answer that
        // cannot be wrong: the edit survives and the mismatch is visible.
        other => Resolution::Park(format!(
            "conflict resolver '{path}' returned an unrecognized resolution '{other}'; \
             expected 'local_wins', 'remote_wins', 'merged' or 'park'"
        )),
    }
}

/// The merged values, narrowed to what this mount may actually push.
///
/// A resolver runs privileged, so the allow-list is enforced HERE rather than
/// trusted: `fields` is already the intersection of the mount's declared
/// `mutable_fields` and what the adapter accepts, and a merge that could write
/// outside it would be a way to push any property through a mount configured to
/// push one.
fn merged_fields(ctx: &SyncCtx<'_>, path: &str, fields: &[String], out: &Value) -> Resolution {
    // `fields` first, `node.properties` as the fallback shape — a resolver that
    // finds it easier to hand back a whole node should not have to restate the
    // diff.
    let source = out
        .get("fields")
        .and_then(|v| v.as_object())
        .or_else(|| {
            out.get("node")
                .and_then(|n| n.get("properties"))
                .and_then(|v| v.as_object())
        })
        .cloned()
        .unwrap_or_default();

    let mut merged = Map::new();
    let mut dropped = Vec::new();
    for (key, value) in source {
        if fields.contains(&key) {
            merged.insert(key, value);
        } else {
            dropped.push(key);
        }
    }
    if !dropped.is_empty() {
        tracing::warn!(
            mount_id = %ctx.scope.mount_id,
            resolver = %path,
            dropped = ?dropped,
            "conflict resolver returned merged fields outside this mount's write \
             allow-list; they were dropped. Add them to write_config.mutable_fields \
             (and to the adapter's declared mutable_fields) if they are meant to push."
        );
    }
    if merged.is_empty() {
        // Not an abandon and not a force: a `merged` with nothing to merge is a
        // resolver bug, and guessing which of the two it meant is how an edit
        // gets lost quietly.
        return Resolution::Park(format!(
            "conflict resolver '{path}' answered 'merged' with no field this mount may \
             push; the edit stays pending"
        ));
    }
    Resolution::Merged(merged)
}

/// `{ field: { local, pushed } }` for every field in the push allow-list.
///
/// Both sides, always, including when one is absent — a resolver's most common
/// question is "did the user change this or has it simply never been pushed?",
/// and omitting the null half makes those two indistinguishable.
fn field_diff(
    fields: &[String],
    watched: &Map<String, Value>,
    last_pushed: Option<&Map<String, Value>>,
) -> Value {
    let mut out = Map::new();
    for field in fields {
        out.insert(
            field.clone(),
            json!({
                "local": watched.get(field).cloned().unwrap_or(Value::Null),
                "pushed": last_pushed
                    .and_then(|p| p.get(field))
                    .cloned()
                    .unwrap_or(Value::Null),
            }),
        );
    }
    Value::Object(out)
}

/// Wrap an adapter failure that is definitely NOT a conflict, so a caller
/// resolving one can pass it straight back.
pub(crate) fn is_conflict(e: &AdapterError) -> bool {
    matches!(e, AdapterError::Conflict(_))
}
