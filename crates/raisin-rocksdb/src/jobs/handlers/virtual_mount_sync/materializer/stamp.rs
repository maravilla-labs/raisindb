//! Re-stamping the engine-owned metadata of a node that is already there — the
//! write drain's stamp-back after a successful push.

use raisin_error::Result;
use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::transactional::TransactionalContext;

use super::index::{MountScope, VirtualNodeRef};
use super::stage::{IndexMutation, Staged};
use super::write_view::{carried_pushed_state, write_view_of, PUSHED_STATE_PROP};
use super::RocksDbMaterializer;

/// The watched fields present in a mapper's output, or `None` when the mount
/// watches nothing (so no `__pushed_state` property is written at all).
///
/// An absent field is OMITTED rather than stored as null: "the provider did not
/// report this" and "the provider reported null" are different answers, and
/// conflating them would make the first local edit of an unreported field
/// invisible to the converge check.
pub(super) fn watched_subset(
    properties: &serde_json::Map<String, serde_json::Value>,
    watched_fields: &[String],
) -> Option<serde_json::Map<String, serde_json::Value>> {
    if watched_fields.is_empty() {
        return None;
    }
    let mut out = serde_json::Map::new();
    for field in watched_fields {
        if let Some(v) = properties.get(field) {
            out.insert(field.clone(), v.clone());
        }
    }
    Some(out)
}

impl RocksDbMaterializer {
    /// Re-stamp the reserved metadata of an existing node after a push.
    ///
    /// A read-modify-write, and the read is what makes it safe: the node is
    /// loaded as it stands NOW and only `__`-prefixed keys are replaced, so a
    /// user edit that landed between the drain's read and this write survives
    /// intact. It will simply be picked up by the next drain — which is the
    /// correct outcome, and much better than the alternative of re-stating a
    /// snapshot and quietly reverting them.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn stage_stamp(
        &self,
        tx: &dyn TransactionalContext,
        scope: &MountScope,
        pending: &mut Vec<IndexMutation>,
        node_id: &str,
        external_id: &str,
        etag: Option<&str>,
        synced_at: &str,
        pushed_state: Option<&serde_json::Map<String, serde_json::Value>>,
        merged: Option<&serde_json::Map<String, serde_json::Value>>,
        adopt: bool,
    ) -> Result<Staged> {
        let Some(mut node) = tx.get_node(&scope.workspace, node_id).await? else {
            // The node was deleted between the drain reading it and this write.
            // Nothing to stamp and nothing to repair: the push already happened
            // remotely and the next reconcile decides what the absence means.
            tracing::debug!(
                mount_id = %scope.mount_id,
                node_id = %node_id,
                "stamp target no longer exists; skipping"
            );
            return Ok(Staged::Skipped);
        };
        // The merged values FIRST, so the `__pushed_state` written below records
        // the same values the node now holds. A `__` key here would let a
        // resolver forge the engine's own metadata — the etag, the baseline —
        // so reserved keys are dropped exactly as `config/props.rs` drops them
        // from mapper output.
        if let Some(merged) = merged {
            for (key, value) in merged {
                if key.starts_with("__") {
                    tracing::warn!(
                        mount_id = %scope.mount_id,
                        node_id = %node_id,
                        key = %key,
                        "conflict resolver returned a reserved property; dropping it"
                    );
                    continue;
                }
                if let Ok(pv) = serde_json::from_value::<PropertyValue>(value.clone()) {
                    node.properties.insert(key.clone(), pv);
                }
            }
        }
        // Adoption: the engine has just created this node's remote counterpart,
        // so the node stops being ordinary user content and becomes mount-owned.
        //
        // Written from typed arguments and the SCOPE, never from `merged` — the
        // provenance the delete rails read (`__mount_id` + a non-empty
        // `__external_id`) must be the engine's own assertion about a call it
        // made, not something a mapper or a resolver can supply. `mount_id`
        // comes from the running drain's scope for the same reason: an adopt
        // cannot name a mount other than the one doing the adopting.
        if adopt {
            node.properties
                .insert("__virtual".to_string(), PropertyValue::Boolean(true));
            node.properties.insert(
                "__mount_id".to_string(),
                PropertyValue::String(scope.mount_id.clone()),
            );
            node.properties.insert(
                "__external_id".to_string(),
                PropertyValue::String(external_id.to_string()),
            );
        }
        if let Some(etag) = etag {
            node.properties.insert(
                "__etag".to_string(),
                PropertyValue::String(etag.to_string()),
            );
        }
        node.properties.insert(
            "__synced_at".to_string(),
            PropertyValue::String(synced_at.to_string()),
        );
        if let Some(pushed) = pushed_state {
            if let Ok(pv) =
                serde_json::from_value::<PropertyValue>(serde_json::Value::Object(pushed.clone()))
            {
                node.properties.insert(PUSHED_STATE_PROP.to_string(), pv);
            }
        }
        tx.upsert_node(&scope.workspace, &node).await?;

        let write_view = write_view_of(&node, &scope.watched_fields);
        pending.push(IndexMutation::Upsert(VirtualNodeRef {
            id: node.id.clone(),
            path: node.path.clone(),
            external_id: external_id.to_string(),
            is_command: scope
                .command_node_types
                .iter()
                .any(|t| t == &node.node_type),
            etag: etag
                .map(str::to_string)
                .or_else(|| match node.properties.get("__etag") {
                    Some(PropertyValue::String(s)) => Some(s.clone()),
                    _ => None,
                }),
            synced_secs: chrono::DateTime::parse_from_rfc3339(synced_at)
                .ok()
                .map(|d| d.timestamp()),
            pushed_state: carried_pushed_state(&node, &write_view),
            write_view,
        }));
        // NOT `Written`: a stamp is the engine amending its own metadata on a
        // node that already exists, so it is no evidence that this mount can
        // materialize provider items — which is exactly what the batcher's
        // failure budget reads `written` for.
        Ok(Staged::Stamped)
    }
}
