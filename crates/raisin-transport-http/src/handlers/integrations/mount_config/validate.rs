// SPDX-License-Identifier: BSL-1.1

//! Validation that no single field can do on its own: how two `sync_config`
//! keys relate, and whether the connector can honour what was asked for.
//!
//! Every rule here REFUSES rather than accepting-and-ignoring. That is the whole
//! point of the endpoint: a setting that is stored, reads back as chosen, and
//! can never have an effect is the failure mode that cost a production drive
//! mount a day of "uploads succeed, bytes never leave".
//!
//! A pair rule fires only when the request TOUCHED one of the keys in it. A
//! mount that already holds an odd combination — one made before this endpoint
//! existed — must still be editable in every unrelated respect, or the first
//! thing this endpoint would do is lock the operator out of fixing it.

use serde_json::{Map, Value};

/// The connector's cached capability blob (`raisin:Integration.capabilities`),
/// absent when the connector has never been probed.
pub(crate) type Caps<'a> = Option<&'a Value>;

fn flag(caps: Caps<'_>, key: &str) -> Option<bool> {
    caps?.get(key)?.as_bool()
}

fn truthy(merged: &Map<String, Value>, key: &str) -> bool {
    merged.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn present(merged: &Map<String, Value>, key: &str) -> bool {
    merged.get(key).is_some_and(|v| !v.is_null())
}

/// Check the merged `sync_config` a patch would produce.
///
/// `touched` is the set of keys the request actually named.
pub(crate) fn check(
    merged: &Map<String, Value>,
    touched: &[String],
    caps: Caps<'_>,
) -> Result<(), String> {
    let named = |k: &str| touched.iter().any(|t| t == k);

    // ---- capability gate -------------------------------------------------
    //
    // Only `webhook` is gated, and only on `supports_push`. A `webhook` mount is
    // never polled (`check::is_due` returns early for it), so a connector that
    // cannot push leaves it with no path to a sync at all: it goes quiet, its
    // status stays whatever it last was, and nothing in the console says why.
    // `hybrid` is NOT gated — it polls as well, so on a push-less connector it
    // degrades to `poll` rather than to silence, and refusing it would refuse a
    // configuration that works.
    if named("mode") && merged.get("mode").and_then(Value::as_str) == Some("webhook") {
        match flag(caps, "supports_push") {
            Some(true) => {}
            Some(false) => {
                return Err(
                    "this connector does not declare `supports_push`, and a `webhook` mount is \
                     never polled — it would have no way to sync at all. Use `hybrid` (polls and \
                     subscribes when it can) or `poll`"
                        .to_string(),
                )
            }
            // Conservative-unknown, the same rule the write fieldset holds: an
            // unprobed connector is treated as incapable, never as permissive.
            None => {
                return Err(
                    "this connector's capabilities have not been probed, so it is not known \
                     whether it can push. Run Test connection (or one sync) first; until then a \
                     `webhook` mount could go permanently silent"
                        .to_string(),
                )
            }
        }
    }

    // ---- cached BYTES: cache_content ⟷ content_ttl_seconds ---------------
    //
    // `content_ttl_seconds` is how long a fetched copy outlives its last use. It
    // is read ONLY inside `if sync_config.cache_content` (the sync run's
    // eviction pass), so on a mount that holds no bytes it is a number that
    // governs nothing.
    if (named("cache_content") || named("content_ttl_seconds"))
        && present(merged, "content_ttl_seconds")
        && !truthy(merged, "cache_content")
    {
        return Err(
            "`content_ttl_seconds` governs how long CACHED BYTES are kept, and this mount does \
             not cache them. Set `cache_content: true` in the same request, or clear the TTL \
             with `content_ttl_seconds: null`"
                .to_string(),
        );
    }

    // ---- expiring NODES: ephemeral ⟷ ttl_seconds -------------------------
    //
    // The mailbox pattern, and a different subject from the bytes above — one
    // deletes NODES, the other deletes cached FILES, and conflating them would
    // mean enabling a cache started deleting a tenant's documents.
    if (named("ephemeral") || named("ttl_seconds"))
        && present(merged, "ttl_seconds")
        && !truthy(merged, "ephemeral")
    {
        return Err(
            "`ttl_seconds` expires synced NODES and is read only on an `ephemeral` mount. Set \
             `ephemeral: true` in the same request, or clear it with `ttl_seconds: null`"
                .to_string(),
        );
    }
    if (named("ephemeral") || named("ttl_seconds"))
        && truthy(merged, "ephemeral")
        && !present(merged, "ttl_seconds")
    {
        // The run's cleanup is `if ephemeral { if let Some(ttl) = ttl_seconds }`,
        // so an ephemeral mount with no TTL expires nothing — a mount that reads
        // as self-cleaning and grows forever.
        return Err(
            "`ephemeral` deletes synced nodes older than `ttl_seconds`, and with no TTL it \
             deletes nothing. Set `ttl_seconds` in the same request, or leave `ephemeral: false`"
                .to_string(),
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn merged(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    fn keys(k: &[&str]) -> Vec<String> {
        k.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn webhook_needs_push_and_unknown_counts_as_cannot() {
        let m = merged(json!({ "mode": "webhook" }));
        assert!(check(
            &m,
            &keys(&["mode"]),
            Some(&json!({ "supports_push": true }))
        )
        .is_ok());
        assert!(check(
            &m,
            &keys(&["mode"]),
            Some(&json!({ "supports_push": false }))
        )
        .is_err());
        // Never probed: refuse. A webhook mount on a connector that turns out
        // not to push is silent, and silence is what this endpoint exists to
        // stop being a configuration outcome.
        assert!(check(&m, &keys(&["mode"]), None).is_err());
    }

    /// `hybrid` still polls, so it must NOT be gated — refusing it would refuse
    /// a configuration that works on every connector.
    #[test]
    fn hybrid_is_not_gated_on_push() {
        let m = merged(json!({ "mode": "hybrid" }));
        assert!(check(
            &m,
            &keys(&["mode"]),
            Some(&json!({ "supports_push": false }))
        )
        .is_ok());
    }

    #[test]
    fn a_content_ttl_without_a_cache_is_refused() {
        let m = merged(json!({ "content_ttl_seconds": 3600 }));
        assert!(check(&m, &keys(&["content_ttl_seconds"]), None).is_err());
        let m = merged(json!({ "content_ttl_seconds": 3600, "cache_content": true }));
        assert!(check(&m, &keys(&["content_ttl_seconds"]), None).is_ok());
    }

    /// Turning the cache OFF while a TTL is stored is caught too: the same
    /// merged state, reached from the other side.
    #[test]
    fn clearing_the_cache_flag_catches_the_orphaned_ttl() {
        let m = merged(json!({ "content_ttl_seconds": 3600, "cache_content": false }));
        assert!(check(&m, &keys(&["cache_content"]), None).is_err());
    }

    #[test]
    fn ephemeral_and_its_ttl_must_agree_in_both_directions() {
        let m = merged(json!({ "ephemeral": true }));
        assert!(check(&m, &keys(&["ephemeral"]), None).is_err());
        let m = merged(json!({ "ttl_seconds": 86400 }));
        assert!(check(&m, &keys(&["ttl_seconds"]), None).is_err());
        let m = merged(json!({ "ephemeral": true, "ttl_seconds": 86400 }));
        assert!(check(&m, &keys(&["ephemeral", "ttl_seconds"]), None).is_ok());
    }

    /// A pre-existing odd combination must not block an unrelated edit, or the
    /// first thing this endpoint does is prevent anyone from fixing it.
    #[test]
    fn an_untouched_pair_is_left_alone() {
        let m = merged(json!({ "content_ttl_seconds": 3600, "path_template": "{name}" }));
        assert!(check(&m, &keys(&["path_template"]), None).is_ok());
    }
}
