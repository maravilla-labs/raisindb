//! Event handler for invalidating the schema stats cache on schema changes.
//!
//! Listens to `Event::Schema` events and invalidates the corresponding
//! branch-scoped entry in the `SharedSchemaStatsCache` so that the next
//! query planning round re-computes up-to-date statistics.

use raisin_core::SharedSchemaStatsCache;
use raisin_events::{Event, EventHandler};
use std::future::Future;
use std::pin::Pin;

/// Invalidates schema stats cache entries when NodeType or Archetype
/// definitions are created, updated, or deleted.
pub struct SchemaStatsEventHandler {
    cache: SharedSchemaStatsCache,
}

impl SchemaStatsEventHandler {
    pub fn new(cache: SharedSchemaStatsCache) -> Self {
        Self { cache }
    }
}

impl EventHandler for SchemaStatsEventHandler {
    fn name(&self) -> &str {
        "schema_stats_invalidator"
    }

    fn handle<'a>(
        &'a self,
        event: &'a Event,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            if let Event::Schema(schema_event) = event {
                let scope_key = format!(
                    "{}:{}:{}",
                    schema_event.tenant_id, schema_event.repository_id, schema_event.branch
                );
                self.cache.invalidate(&scope_key);
                tracing::debug!(
                    scope = %scope_key,
                    kind = ?schema_event.kind,
                    "Schema stats cache invalidated"
                );
            }
            Ok(())
        })
    }
}
