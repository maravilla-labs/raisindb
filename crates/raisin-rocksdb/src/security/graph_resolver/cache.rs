//! Cache lookup methods for the graph resolver.
//!
//! Checks both in-memory cache and RocksDB GRAPH_CACHE column family
//! for precomputed reachability data.

use raisin_storage::RelationRepository;

use crate::graph::{CachedValue, GraphCacheValue};
use crate::keys::graph_cache_key;
use crate::{cf, cf_handle};

use super::RocksDBGraphResolver;

impl<R: RelationRepository> RocksDBGraphResolver<'_, R> {
    /// Check RELATES cache for precomputed reachability.
    ///
    /// Returns `Some(true)` if target is reachable according to cache,
    /// `Some(false)` if target is definitely not reachable,
    /// `None` if cache miss (should fallback to BFS).
    pub(super) fn check_cache(
        &self,
        source_id: &str,
        target_id: &str,
        relation_types: &[String],
        max_depth: u32,
    ) -> Option<bool> {
        let db = self.db.as_ref()?;
        let cache_layer = self.cache_layer.as_ref()?;

        // Build config ID for RELATES cache
        // Format: relates-cache-{relation_types}-{max_depth}
        let config_id = format!("relates-cache-{}-{}", relation_types.join("-"), max_depth);

        // First, try in-memory cache
        if let Some(cached_value) = cache_layer.get(&config_id, source_id) {
            return self.check_reachability_set(&cached_value, target_id);
        }

        // Try RocksDB GRAPH_CACHE column family
        let cf = cf_handle(db, cf::GRAPH_CACHE).ok()?;
        let key = graph_cache_key(
            self.tenant_id,
            self.repo_id,
            self.branch,
            &config_id,
            source_id,
        );

        match db.get_cf(cf, &key) {
            Ok(Some(bytes)) => {
                let cached_value: GraphCacheValue = rmp_serde::from_slice(&bytes).ok()?;

                // Check if expired
                if cached_value.is_expired() {
                    return None; // Expired, need BFS
                }

                // Populate in-memory cache for next time
                cache_layer.put(&config_id, source_id, cached_value.clone());

                self.check_reachability_set(&cached_value, target_id)
            }
            _ => None, // Cache miss
        }
    }

    /// Check if target_id is in the reachability set.
    fn check_reachability_set(
        &self,
        cached_value: &GraphCacheValue,
        target_id: &str,
    ) -> Option<bool> {
        if let CachedValue::ReachabilitySet(reachable_ids) = &cached_value.value {
            Some(reachable_ids.contains(&target_id.to_string()))
        } else {
            None // Wrong value type
        }
    }
}
