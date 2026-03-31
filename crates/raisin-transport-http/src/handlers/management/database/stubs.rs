// SPDX-License-Identifier: BSL-1.1

//! Stub implementations for non-rocksdb feature builds.
//!
//! When the `storage-rocksdb` feature is disabled, these stubs return
//! `NOT_IMPLEMENTED` for all database management endpoints.

use axum::http::StatusCode;

#[cfg(not(feature = "storage-rocksdb"))]
pub async fn verify_fulltext_index() -> (StatusCode, &'static str) {
    (StatusCode::NOT_IMPLEMENTED, "RocksDB feature not enabled")
}

#[cfg(not(feature = "storage-rocksdb"))]
pub async fn rebuild_fulltext_index() -> (StatusCode, &'static str) {
    (StatusCode::NOT_IMPLEMENTED, "RocksDB feature not enabled")
}

#[cfg(not(feature = "storage-rocksdb"))]
pub async fn optimize_fulltext_index() -> (StatusCode, &'static str) {
    (StatusCode::NOT_IMPLEMENTED, "RocksDB feature not enabled")
}

#[cfg(not(feature = "storage-rocksdb"))]
pub async fn purge_fulltext_index() -> (StatusCode, &'static str) {
    (StatusCode::NOT_IMPLEMENTED, "RocksDB feature not enabled")
}

#[cfg(not(feature = "storage-rocksdb"))]
pub async fn get_fulltext_health() -> (StatusCode, &'static str) {
    (StatusCode::NOT_IMPLEMENTED, "RocksDB feature not enabled")
}

#[cfg(not(feature = "storage-rocksdb"))]
pub async fn verify_vector_index() -> (StatusCode, &'static str) {
    (StatusCode::NOT_IMPLEMENTED, "RocksDB feature not enabled")
}

#[cfg(not(feature = "storage-rocksdb"))]
pub async fn rebuild_vector_index() -> (StatusCode, &'static str) {
    (StatusCode::NOT_IMPLEMENTED, "RocksDB feature not enabled")
}

#[cfg(not(feature = "storage-rocksdb"))]
pub async fn optimize_vector_index() -> (StatusCode, &'static str) {
    (StatusCode::NOT_IMPLEMENTED, "RocksDB feature not enabled")
}

#[cfg(not(feature = "storage-rocksdb"))]
pub async fn restore_vector_index() -> (StatusCode, &'static str) {
    (StatusCode::NOT_IMPLEMENTED, "RocksDB feature not enabled")
}

#[cfg(not(feature = "storage-rocksdb"))]
pub async fn get_vector_health() -> (StatusCode, &'static str) {
    (StatusCode::NOT_IMPLEMENTED, "RocksDB feature not enabled")
}

#[cfg(not(feature = "storage-rocksdb"))]
pub async fn regenerate_vector_embeddings() -> (StatusCode, &'static str) {
    (StatusCode::NOT_IMPLEMENTED, "RocksDB feature not enabled")
}

#[cfg(not(feature = "storage-rocksdb"))]
pub async fn reindex_start() -> (StatusCode, &'static str) {
    (StatusCode::NOT_IMPLEMENTED, "RocksDB feature not enabled")
}

#[cfg(not(feature = "storage-rocksdb"))]
pub async fn verify_relation_integrity() -> (StatusCode, &'static str) {
    (StatusCode::NOT_IMPLEMENTED, "RocksDB feature not enabled")
}

#[cfg(not(feature = "storage-rocksdb"))]
pub async fn repair_relation_integrity() -> (StatusCode, &'static str) {
    (StatusCode::NOT_IMPLEMENTED, "RocksDB feature not enabled")
}
