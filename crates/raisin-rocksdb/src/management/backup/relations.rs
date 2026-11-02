//! Graph relations in a repository backup (`relations.jsonl`).
//!
//! Relations live in `RELATION_INDEX`, keyed by node id, not in the node
//! records, so `nodes.jsonl` alone restored a repository without a single
//! edge. Backups now carry them in `relations.jsonl`, one JSON object per
//! line:
//!
//! ```json
//! {"v":1,"branch":"main","source_id":"..","source_workspace":"..",
//!  "source_node_type":"..","target_id":"..","target_workspace":"..",
//!  "target_node_type":"..","relation_type":"FRIENDS_WITH","weight":1.0}
//! ```
//!
//! Only relations live at each branch HEAD are exported (no tombstones, no
//! history). Node ids are preserved by a restore, so the endpoints are written
//! back unchanged. A backup without the file (older releases) restores with no
//! relations, as before. Lines with a newer `v` than this reader knows are
//! refused rather than half-understood.

use crate::repositories::put_full_relation_to_batch;
use crate::RocksDBStorage;
use raisin_error::{Error, Result};
use raisin_models::nodes::FullRelation;
use raisin_storage::{BranchScope, RelationRepository, Storage};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, Write};
use std::path::Path;

/// File name of the relations section of a repository backup.
pub(super) const RELATIONS_FILE: &str = "relations.jsonl";

/// Version of one `relations.jsonl` line.
pub(super) const RELATION_LINE_VERSION: u32 = 1;

/// One exported relation: the branch it lives on plus the full record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(super) struct BackupRelation {
    /// Line format version.
    pub v: u32,
    /// Branch the relation was live on.
    pub branch: String,
    /// The relation itself (both endpoints, type, weight).
    #[serde(flatten)]
    pub relation: FullRelation,
}

/// Every relation live at each branch HEAD of a repository.
pub(super) async fn export_relations(
    storage: &RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
) -> Result<Vec<BackupRelation>> {
    let mut out = Vec::new();
    for branch in super::export::export_branches(storage, tenant_id, repo_id).await? {
        let rows = storage
            .relations()
            .scan_relations_global(
                BranchScope::new(tenant_id, repo_id, &branch.name),
                None,
                Some(&branch.head),
            )
            .await?;
        for (_, _, _, _, relation) in rows {
            out.push(BackupRelation {
                v: RELATION_LINE_VERSION,
                branch: branch.name.clone(),
                relation,
            });
        }
    }
    Ok(out)
}

/// Write `relations` to `file` as JSON Lines.
pub(super) fn write_relations(file: &Path, relations: &[BackupRelation]) -> Result<()> {
    let mut writer = std::io::BufWriter::new(
        std::fs::File::create(file)
            .map_err(|e| Error::storage(format!("Failed to create relations file: {}", e)))?,
    );
    for rel in relations {
        serde_json::to_writer(&mut writer, rel)
            .map_err(|e| Error::storage(format!("Failed to write relation: {}", e)))?;
        writeln!(&mut writer)
            .map_err(|e| Error::storage(format!("Failed to write newline: {}", e)))?;
    }
    writer
        .flush()
        .map_err(|e| Error::storage(format!("Failed to flush relations file: {}", e)))
}

/// Read `relations.jsonl`, refusing a line format newer than this reader.
pub(super) fn read_relations(file: &Path) -> Result<Vec<BackupRelation>> {
    let handle = std::fs::File::open(file)
        .map_err(|e| Error::storage(format!("Failed to open relations file: {}", e)))?;
    let mut out = Vec::new();
    for (n, line) in std::io::BufReader::new(handle).lines().enumerate() {
        let line = line.map_err(|e| Error::storage(format!("Failed to read line: {}", e)))?;
        if line.trim().is_empty() {
            continue;
        }
        let rel: BackupRelation = serde_json::from_str(&line).map_err(|e| {
            Error::storage(format!("Failed to parse relation on line {}: {}", n + 1, e))
        })?;
        if rel.v > RELATION_LINE_VERSION {
            return Err(Error::Validation(format!(
                "restore refused: relations.jsonl line {} has format v{}, this release reads up \
                 to v{}",
                n + 1,
                rel.v,
                RELATION_LINE_VERSION
            )));
        }
        out.push(rel);
    }
    Ok(out)
}

/// Restore relations onto their branches, at the revision the node restore
/// uses (`HLC(0,0)`, visible at any HEAD). Returns how many were written.
pub(super) fn import_relations(
    storage: &RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
    relations: &[BackupRelation],
) -> Result<usize> {
    let revision = raisin_hlc::HLC::new(0, 0);
    let mut batch = rocksdb::WriteBatch::default();
    for (i, rel) in relations.iter().enumerate() {
        put_full_relation_to_batch(
            storage.db(),
            &mut batch,
            tenant_id,
            repo_id,
            &rel.branch,
            &revision,
            &rel.relation,
        )?;
        if (i + 1) % 1000 == 0 {
            storage
                .db()
                .write(std::mem::take(&mut batch))
                .map_err(|e| Error::storage(format!("Relation batch write failed: {}", e)))?;
        }
    }
    if !batch.is_empty() {
        storage
            .db()
            .write(batch)
            .map_err(|e| Error::storage(format!("Relation batch write failed: {}", e)))?;
    }
    Ok(relations.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> BackupRelation {
        BackupRelation {
            v: RELATION_LINE_VERSION,
            branch: "main".into(),
            relation: FullRelation::new(
                "a".into(),
                "raisin:access_control".into(),
                "raisin:User".into(),
                "b".into(),
                "raisin:access_control".into(),
                "raisin:User".into(),
                "FRIENDS_WITH".into(),
                Some(0.5),
            ),
        }
    }

    #[test]
    fn lines_round_trip_and_stay_flat() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(RELATIONS_FILE);
        write_relations(&file, &[sample()]).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(
            text.contains("\"relation_type\":\"FRIENDS_WITH\""),
            "{text}"
        );
        assert!(text.contains("\"v\":1"), "{text}");
        assert_eq!(read_relations(&file).unwrap(), vec![sample()]);
    }

    #[test]
    fn a_newer_line_format_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(RELATIONS_FILE);
        let mut rel = sample();
        rel.v = RELATION_LINE_VERSION + 1;
        write_relations(&file, &[rel]).unwrap();
        assert!(read_relations(&file).is_err());
    }
}
