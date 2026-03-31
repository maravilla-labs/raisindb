//! Archetype repository implementation backed by RocksDB

mod crud;

use crate::repositories::{BranchRepositoryImpl, RevisionRepositoryImpl};
use crate::{cf, cf_handle, keys};
use chrono::Utc;
use nanoid::nanoid;
use raisin_error::{Error as RaisinError, Result};
use raisin_hlc::HLC;
use raisin_models::nodes::types::archetype::Archetype;
use raisin_models::tree::ChangeOperation;
use raisin_storage::{
    ArchetypeChangeInfo, BranchRepository, CommitMetadata, RevisionMeta, RevisionRepository,
};
use rocksdb::{WriteBatch, DB};
use std::sync::Arc;

pub(super) const TOMBSTONE: &[u8] = b"TOMBSTONE";

#[derive(Clone)]
pub struct ArchetypeRepositoryImpl {
    pub(super) db: Arc<DB>,
    pub(super) revision_repo: Arc<RevisionRepositoryImpl>,
    pub(super) branch_repo: Arc<BranchRepositoryImpl>,
    pub(super) operation_capture: Option<Arc<crate::OperationCapture>>,
}

impl ArchetypeRepositoryImpl {
    pub fn new(
        db: Arc<DB>,
        revision_repo: Arc<RevisionRepositoryImpl>,
        branch_repo: Arc<BranchRepositoryImpl>,
    ) -> Self {
        Self {
            db,
            revision_repo,
            branch_repo,
            operation_capture: None,
        }
    }

    pub fn new_with_capture(
        db: Arc<DB>,
        revision_repo: Arc<RevisionRepositoryImpl>,
        branch_repo: Arc<BranchRepositoryImpl>,
        operation_capture: Arc<crate::OperationCapture>,
    ) -> Self {
        Self {
            db,
            revision_repo,
            branch_repo,
            operation_capture: Some(operation_capture),
        }
    }

    pub(super) fn decode_revision(key: &[u8]) -> Result<HLC> {
        if key.len() < 16 {
            return Err(RaisinError::storage(format!(
                "Invalid archetype key: too short ({} bytes, need at least 16 for HLC)",
                key.len()
            )));
        }

        let rev_bytes = &key[key.len() - 16..];

        keys::decode_descending_revision(rev_bytes).map_err(|e| {
            RaisinError::storage(format!("Failed to decode archetype revision: {}", e))
        })
    }

    pub(super) fn deserialize(bytes: &[u8]) -> Result<Archetype> {
        rmp_serde::from_slice::<Archetype>(bytes).map_err(|e| {
            RaisinError::storage(format!("Deserialization error for Archetype: {}", e))
        })
    }

    pub(super) fn extract_name(key: &[u8]) -> Result<String> {
        let parts: Vec<&[u8]> = key.split(|&b| b == 0).collect();
        let name_bytes = parts
            .get(4)
            .ok_or_else(|| RaisinError::storage("Invalid archetype key (missing name)"))?;
        String::from_utf8(name_bytes.to_vec())
            .map_err(|e| RaisinError::storage(format!("Invalid UTF-8 in archetype name: {}", e)))
    }

    pub(super) async fn resolve_head_revision(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
    ) -> Result<Option<HLC>> {
        match self.branch_repo.get_head(tenant_id, repo_id, branch).await {
            Ok(head) => Ok(Some(head)),
            Err(RaisinError::NotFound(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub(super) async fn get_at_or_before(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        name: &str,
        target_revision: &HLC,
    ) -> Result<Option<Archetype>> {
        let cf = cf_handle(&self.db, cf::ARCHETYPES)?;
        let prefix = keys::archetype_name_prefix(tenant_id, repo_id, branch, name);
        let prefix_clone = prefix.clone();
        let iter = self.db.prefix_iterator_cf(cf, prefix);

        for item in iter {
            let (key, value) = item.map_err(|e| RaisinError::storage(e.to_string()))?;
            if !key.starts_with(&prefix_clone) {
                break;
            }

            let revision = Self::decode_revision(&key)?;
            if &revision > target_revision {
                continue;
            }

            if value.as_ref() == TOMBSTONE {
                return Ok(None);
            }

            let archetype = Self::deserialize(&value)?;
            return Ok(Some(archetype));
        }

        Ok(None)
    }

    pub(super) fn determine_parent(parent_head: Option<HLC>) -> Option<HLC> {
        parent_head
    }

    pub(super) fn apply_versioning(
        archetype: Archetype,
        existing: Option<&Archetype>,
    ) -> Archetype {
        let now = Utc::now();
        let mut enriched = archetype.clone();

        if enriched.id.is_empty() {
            enriched.id = nanoid!(16);
        }

        if let Some(previous) = existing {
            let next_version = previous.version.unwrap_or(0) + 1;
            enriched.version = Some(next_version);
            enriched.created_at = previous.created_at;
            enriched.previous_version = Some(previous.id.clone());
        } else {
            enriched.version = Some(1);
            if enriched.created_at.is_none() {
                enriched.created_at = Some(now);
            }
        }

        enriched.updated_at = Some(now);
        enriched
    }

    pub(super) fn build_revision_meta(
        revision: HLC,
        parent: Option<HLC>,
        branch: &str,
        commit: &CommitMetadata,
        operation: ChangeOperation,
        archetype_name: &str,
    ) -> RevisionMeta {
        RevisionMeta {
            revision,
            parent,
            merge_parent: None,
            branch: branch.to_string(),
            timestamp: Utc::now(),
            actor: commit.actor.clone(),
            message: commit.message.clone(),
            is_system: commit.is_system,
            changed_nodes: Vec::new(),
            changed_node_types: Vec::new(),
            changed_archetypes: vec![ArchetypeChangeInfo {
                name: archetype_name.to_string(),
                operation,
            }],
            changed_element_types: Vec::new(),
            operation: None,
        }
    }
}
