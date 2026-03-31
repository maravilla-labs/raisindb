//! Tests for checkpoint server.

use super::*;
use rocksdb::Options;
use tempfile::TempDir;

#[tokio::test]
async fn test_checkpoint_server_creation() {
    let temp_db = TempDir::new().unwrap();
    let temp_checkpoints = TempDir::new().unwrap();

    let mut opts = Options::default();
    opts.create_if_missing(true);

    let db = Arc::new(DB::open(&opts, temp_db.path()).unwrap());

    let _server = CheckpointServer::new(
        db,
        temp_checkpoints.path().to_path_buf(),
        "node1".to_string(),
        None, // No Tantivy manager for test
        None, // No HNSW manager for test
    );

    // Server created successfully
}
