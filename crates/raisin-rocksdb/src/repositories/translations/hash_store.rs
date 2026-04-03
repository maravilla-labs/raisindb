//! Hash record storage for translation staleness detection.
//!
//! This module provides storage operations for TranslationHashRecord,
//! which tracks the original content hash at the time of translation.

use raisin_error::Result;
use raisin_models::translations::{JsonPointer, LocaleCode, TranslationHashRecord};
use rocksdb::DB;
use std::collections::HashMap;
use std::sync::Arc;

use crate::error_ext::ResultExt;

use super::keys;

/// Store a hash record for a translation field.
pub(super) async fn store_hash_record(
    db: &Arc<DB>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    locale: &LocaleCode,
    pointer: &JsonPointer,
    record: &TranslationHashRecord,
) -> Result<()> {
    let cf = crate::cf_handle(db, crate::cf::TRANSLATION_HASHES)?;

    let key = keys::translation_hash_key(
        tenant_id,
        repo_id,
        branch,
        workspace,
        node_id,
        locale.as_str(),
        pointer.as_str(),
    );

    let value = serde_json::to_vec(record).map_err(|e| {
        raisin_error::Error::internal(format!("Failed to serialize hash record: {}", e))
    })?;

    db.put_cf(&cf, key, value).rocksdb_err()?;

    Ok(())
}

/// Store multiple hash records in a batch.
pub(super) async fn store_hash_records_batch(
    db: &Arc<DB>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    locale: &LocaleCode,
    records: &HashMap<JsonPointer, TranslationHashRecord>,
) -> Result<()> {
    if records.is_empty() {
        return Ok(());
    }

    let cf = crate::cf_handle(db, crate::cf::TRANSLATION_HASHES)?;
    let mut batch = rocksdb::WriteBatch::default();

    for (pointer, record) in records {
        let key = keys::translation_hash_key(
            tenant_id,
            repo_id,
            branch,
            workspace,
            node_id,
            locale.as_str(),
            pointer.as_str(),
        );

        let value = serde_json::to_vec(record).map_err(|e| {
            raisin_error::Error::internal(format!("Failed to serialize hash record: {}", e))
        })?;

        batch.put_cf(&cf, key, value);
    }

    db.write(batch).rocksdb_err()?;

    Ok(())
}

/// Get all hash records for a node/locale combination.
pub(super) async fn get_hash_records(
    db: &Arc<DB>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    locale: &LocaleCode,
) -> Result<HashMap<JsonPointer, TranslationHashRecord>> {
    let cf = crate::cf_handle(db, crate::cf::TRANSLATION_HASHES)?;

    let prefix = keys::translation_hash_prefix(
        tenant_id,
        repo_id,
        branch,
        workspace,
        node_id,
        locale.as_str(),
    );

    let mut records = HashMap::new();
    let iter = db.prefix_iterator_cf(&cf, &prefix);

    for item in iter {
        let (key, value) = item.rocksdb_err()?;

        // Check if key is still within our prefix
        if !key.starts_with(&prefix) {
            break;
        }

        // Extract pointer from key
        // Key format: {prefix}{pointer}
        if let Some(pointer_bytes) = key.strip_prefix(prefix.as_slice()) {
            if let Ok(pointer_str) = std::str::from_utf8(pointer_bytes) {
                let pointer = JsonPointer::new(pointer_str);

                // Deserialize the hash record
                match serde_json::from_slice::<TranslationHashRecord>(&value) {
                    Ok(record) => {
                        records.insert(pointer, record);
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Failed to deserialize hash record for {}/{}: {}",
                            node_id,
                            pointer_str,
                            e
                        );
                    }
                }
            }
        }
    }

    Ok(records)
}

/// Delete all hash records for a node/locale combination.
pub(super) async fn delete_hash_records(
    db: &Arc<DB>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    locale: &LocaleCode,
) -> Result<()> {
    let cf = crate::cf_handle(db, crate::cf::TRANSLATION_HASHES)?;

    let prefix = keys::translation_hash_prefix(
        tenant_id,
        repo_id,
        branch,
        workspace,
        node_id,
        locale.as_str(),
    );

    let mut batch = rocksdb::WriteBatch::default();
    let iter = db.prefix_iterator_cf(&cf, &prefix);

    for item in iter {
        let (key, _) = item.rocksdb_err()?;

        // Check if key is still within our prefix
        if !key.starts_with(&prefix) {
            break;
        }

        batch.delete_cf(&cf, &key);
    }

    if batch.len() > 0 {
        db.write(batch).rocksdb_err()?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn create_test_db() -> Arc<DB> {
        let temp_dir = tempfile::tempdir().unwrap();
        let mut opts = rocksdb::Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);

        let cfs = vec![rocksdb::ColumnFamilyDescriptor::new(
            crate::cf::TRANSLATION_HASHES,
            rocksdb::Options::default(),
        )];

        Arc::new(
            DB::open_cf_descriptors(&opts, temp_dir.path(), cfs).expect("Failed to create test DB"),
        )
    }

    #[tokio::test]
    async fn test_store_and_get_hash_record() {
        let db = create_test_db();

        let locale = LocaleCode::parse("de-DE").unwrap();
        let pointer = JsonPointer::new("/title");
        let record = TranslationHashRecord::new("abc123".to_string(), raisin_hlc::HLC::new(42, 0));

        // Store
        store_hash_record(
            &db, "tenant1", "repo1", "main", "ws1", "node1", &locale, &pointer, &record,
        )
        .await
        .unwrap();

        // Get
        let records = get_hash_records(&db, "tenant1", "repo1", "main", "ws1", "node1", &locale)
            .await
            .unwrap();

        assert_eq!(records.len(), 1);
        let retrieved = records.get(&pointer).unwrap();
        assert_eq!(retrieved.original_hash, "abc123");
        assert_eq!(retrieved.original_revision, raisin_hlc::HLC::new(42, 0));
    }

    #[tokio::test]
    async fn test_batch_store() {
        let db = create_test_db();

        let locale = LocaleCode::parse("fr-FR").unwrap();
        let mut records = HashMap::new();
        records.insert(
            JsonPointer::new("/title"),
            TranslationHashRecord::new("hash1".to_string(), raisin_hlc::HLC::new(1, 0)),
        );
        records.insert(
            JsonPointer::new("/description"),
            TranslationHashRecord::new("hash2".to_string(), raisin_hlc::HLC::new(1, 0)),
        );
        records.insert(
            JsonPointer::new("/content"),
            TranslationHashRecord::new("hash3".to_string(), raisin_hlc::HLC::new(1, 0)),
        );

        // Store batch
        store_hash_records_batch(
            &db, "tenant1", "repo1", "main", "ws1", "node1", &locale, &records,
        )
        .await
        .unwrap();

        // Get all
        let retrieved = get_hash_records(&db, "tenant1", "repo1", "main", "ws1", "node1", &locale)
            .await
            .unwrap();

        assert_eq!(retrieved.len(), 3);
        assert_eq!(
            retrieved
                .get(&JsonPointer::new("/title"))
                .unwrap()
                .original_hash,
            "hash1"
        );
        assert_eq!(
            retrieved
                .get(&JsonPointer::new("/description"))
                .unwrap()
                .original_hash,
            "hash2"
        );
    }

    #[tokio::test]
    async fn test_delete_hash_records() {
        let db = create_test_db();

        let locale = LocaleCode::parse("es-ES").unwrap();
        let mut records = HashMap::new();
        records.insert(
            JsonPointer::new("/title"),
            TranslationHashRecord::new("hash1".to_string(), raisin_hlc::HLC::new(1, 0)),
        );
        records.insert(
            JsonPointer::new("/description"),
            TranslationHashRecord::new("hash2".to_string(), raisin_hlc::HLC::new(1, 0)),
        );

        // Store
        store_hash_records_batch(
            &db, "tenant1", "repo1", "main", "ws1", "node1", &locale, &records,
        )
        .await
        .unwrap();

        // Verify stored
        let retrieved = get_hash_records(&db, "tenant1", "repo1", "main", "ws1", "node1", &locale)
            .await
            .unwrap();
        assert_eq!(retrieved.len(), 2);

        // Delete
        delete_hash_records(&db, "tenant1", "repo1", "main", "ws1", "node1", &locale)
            .await
            .unwrap();

        // Verify deleted
        let retrieved = get_hash_records(&db, "tenant1", "repo1", "main", "ws1", "node1", &locale)
            .await
            .unwrap();
        assert_eq!(retrieved.len(), 0);
    }
}
