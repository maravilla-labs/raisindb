//! Read/write of `CompoundIndexState` records in `cf::INDEX_STATUS`.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use raisin_error::{Error, Result};
use raisin_models::nodes::properties::schema::CompoundIndexDefinition;
use raisin_storage::compound::{
    CompoundAvailability, CompoundBuildPhase, CompoundIndexState, CompoundStateSource,
};
use rocksdb::{WriteBatch, DB};

use crate::{cf, cf_handle};

/// Key: `compound_index\0{tenant}\0{repo}\0{branch}\0{workspace}\0{index_name}`.
///
/// The index NAME is the whole discriminator, deliberately — that is exactly
/// what the entry keyspace is scoped by (`…cidx\0{index_name}\0…`), which
/// carries no node type. Keying this record by node type instead would let two
/// NodeTypes declaring the same index name each record "ready" for one shared,
/// interleaved keyspace.
///
/// `{index_name}` must be null-free; names come from YAML/SQL identifiers, and
/// `warn_on_suspect_compound_indexes` is where a hostile one would be caught.
pub fn compound_state_key(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    index_name: &str,
) -> Vec<u8> {
    format!("compound_index\0{tenant_id}\0{repo_id}\0{branch}\0{workspace}\0{index_name}")
        .into_bytes()
}

/// Uncached read, for callers that hold a `&DB` and not a store.
pub fn read_state(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    index_name: &str,
) -> Result<Option<CompoundIndexState>> {
    let cf = cf_handle(db, cf::INDEX_STATUS)?;
    let key = compound_state_key(tenant_id, repo_id, branch, workspace, index_name);
    match db
        .get_cf(cf, &key)
        .map_err(|e| Error::storage(format!("Failed to read compound index state: {}", e)))?
    {
        Some(bytes) => Ok(rmp_serde::from_slice::<CompoundIndexState>(&bytes).ok()),
        None => Ok(None),
    }
}

/// Cached reader/writer for compound index build state.
///
/// The cache is per-instance and write-through-invalidated, matching
/// `SpatialStateStore`. A caller consulting availability in a hot loop should
/// hold the handle rather than re-fetching it from `Storage::compound_state()`.
#[derive(Clone)]
pub struct CompoundStateStore {
    db: Arc<DB>,
    cache: Arc<RwLock<HashMap<Vec<u8>, Option<Arc<CompoundIndexState>>>>>,
}

impl CompoundStateStore {
    pub fn new(db: Arc<DB>) -> Self {
        Self {
            db,
            cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Cache-first read. A corrupt record is reported and treated as absent —
    /// which is the fail-closed answer, not a silent success.
    pub fn get(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        index_name: &str,
    ) -> Result<Option<Arc<CompoundIndexState>>> {
        let key = compound_state_key(tenant_id, repo_id, branch, workspace, index_name);

        if let Ok(cache) = self.cache.read() {
            if let Some(hit) = cache.get(&key) {
                return Ok(hit.clone());
            }
        }

        let cf = cf_handle(&self.db, cf::INDEX_STATUS)?;
        let loaded =
            match self.db.get_cf(cf, &key).map_err(|e| {
                Error::storage(format!("Failed to read compound index state: {}", e))
            })? {
                Some(bytes) => match rmp_serde::from_slice::<CompoundIndexState>(&bytes) {
                    Ok(state) => Some(Arc::new(state)),
                    Err(e) => {
                        tracing::warn!(
                            index_name,
                            error = %e,
                            "Corrupt compound index state record; treating the index as not built"
                        );
                        None
                    }
                },
                None => None,
            };

        if let Ok(mut cache) = self.cache.write() {
            // Negative entries are cached too: "no record" is the common case
            // on a database that predates this record, and re-reading RocksDB
            // for it on every statement is the overhead this cache exists to
            // avoid.
            cache.insert(key, loaded.clone());
        }
        Ok(loaded)
    }

    /// Stage a record into an existing batch, invalidating the cache entry.
    pub fn put_to_batch(
        &self,
        batch: &mut WriteBatch,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        state: &CompoundIndexState,
    ) -> Result<()> {
        let cf = cf_handle(&self.db, cf::INDEX_STATUS)?;
        let key = compound_state_key(tenant_id, repo_id, branch, workspace, &state.index_name);
        let bytes = rmp_serde::to_vec(state).map_err(|e| {
            Error::storage(format!("Failed to serialize compound index state: {}", e))
        })?;
        batch.put_cf(cf, &key, bytes);
        self.invalidate(&key);
        Ok(())
    }

    /// Write one record immediately.
    pub fn put(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        state: &CompoundIndexState,
    ) -> Result<()> {
        let mut batch = WriteBatch::default();
        self.put_to_batch(&mut batch, tenant_id, repo_id, branch, workspace, state)?;
        self.db
            .write(batch)
            .map_err(|e| Error::storage(format!("Failed to write compound index state: {}", e)))
    }

    /// Flip an existing record to `NotBuilt`. No-op when there is no record —
    /// absent already reads as `NotBuilt`.
    pub fn mark_not_built(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        index_name: &str,
    ) -> Result<()> {
        if let Some(existing) = self.get(tenant_id, repo_id, branch, workspace, index_name)? {
            let mut state = (*existing).clone();
            state.phase = CompoundBuildPhase::NotBuilt;
            self.put(tenant_id, repo_id, branch, workspace, &state)?;
        }
        Ok(())
    }

    /// Every recorded index in a workspace. Used by the boot sweep to tell
    /// "declared and recorded" from "declared but never built".
    pub fn list_for_workspace(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
    ) -> Result<Vec<CompoundIndexState>> {
        let cf = cf_handle(&self.db, cf::INDEX_STATUS)?;
        let prefix =
            format!("compound_index\0{tenant_id}\0{repo_id}\0{branch}\0{workspace}\0").into_bytes();

        let mut out = Vec::new();
        for item in self.db.prefix_iterator_cf(cf, &prefix) {
            let (key, value) =
                item.map_err(|e| Error::storage(format!("Failed to list compound state: {}", e)))?;
            if !key.starts_with(&prefix) {
                break;
            }
            if let Ok(state) = rmp_serde::from_slice::<CompoundIndexState>(&value) {
                out.push(state);
            }
        }
        Ok(out)
    }

    fn invalidate(&self, key: &[u8]) {
        if let Ok(mut cache) = self.cache.write() {
            cache.remove(key);
        }
    }
}

impl CompoundStateSource for CompoundStateStore {
    fn compound_availability(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        definition: &CompoundIndexDefinition,
    ) -> CompoundAvailability {
        match self.get(tenant_id, repo_id, branch, workspace, &definition.name) {
            Ok(Some(state)) => state.availability_for(definition),
            // No record: fail CLOSED. The declaration existing is not evidence
            // that the entries do.
            Ok(None) => CompoundAvailability::NotBuilt,
            Err(e) => CompoundAvailability::Unusable(format!(
                "could not read compound index state: {}",
                e
            )),
        }
    }
}
