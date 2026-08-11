//! Event handler for invalidating the encrypted-fields gate on schema changes.
//!
//! Deliberately mirrors `SchemaStatsEventHandler`, which does the same job for
//! the schema stats cache. The difference is what a stale entry costs: a stale
//! statistic is a worse query plan, while a stale `false` here writes a secret
//! to disk in PLAINTEXT — into the node blob, the property index and every
//! replica, with no later fix able to recover it. See the module docs on
//! `raisin_core::services::encrypted_fields`.
//!
//! The checkpoint-ingest half of invalidation (`derived_cache_registry`) is
//! already wired inside `encrypted_fields::global()`; this handler covers the
//! ordinary `Event::Schema` half.

use raisin_core::services::encrypted_fields;
use raisin_events::{Event, EventHandler};
use std::future::Future;
use std::pin::Pin;

/// Drops the encrypted-fields answers for a branch when a NodeType, Archetype
/// or ElementType in it is created, updated or deleted.
pub struct EncryptedFieldsEventHandler;

impl EventHandler for EncryptedFieldsEventHandler {
    fn name(&self) -> &str {
        "encrypted_fields_invalidator"
    }

    fn handle<'a>(
        &'a self,
        event: &'a Event,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            if let Event::Schema(schema_event) = event {
                encrypted_fields::global().invalidate_scope(
                    &schema_event.tenant_id,
                    &schema_event.repository_id,
                    &schema_event.branch,
                );
                tracing::debug!(
                    tenant = %schema_event.tenant_id,
                    repo = %schema_event.repository_id,
                    branch = %schema_event.branch,
                    kind = ?schema_event.kind,
                    "Encrypted-fields gate invalidated"
                );
            }
            Ok(())
        })
    }
}
