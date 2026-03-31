//! HNSW vector index transfer.
//!
//! Handles transfer of HNSW vector indexes from the source peer,
//! including verification, staging, and ingestion.

use super::CatchUpCoordinator;
use crate::ReplicationMessage;
use raisin_error::{Error, Result};
use tokio::net::TcpStream;
use tracing::info;

impl CatchUpCoordinator {
    /// Transfer a single HNSW index (entire file, typically small)
    pub(super) async fn transfer_single_hnsw_index(
        &self,
        stream: &mut TcpStream,
        receiver: &dyn crate::HnswIndexReceiver,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
    ) -> Result<()> {
        // Request HNSW index
        let request = ReplicationMessage::RequestHnswIndex {
            tenant_id: tenant_id.to_string(),
            repo_id: repo_id.to_string(),
            branch: branch.to_string(),
        };
        Self::send_message(stream, &request).await?;

        // Receive index data
        let response = Self::receive_message(stream).await?;

        match response {
            ReplicationMessage::HnswIndexData {
                tenant_id: _,
                repo_id: _,
                branch: _,
                data,
                crc32,
            } => {
                info!(size_mb = data.len() / 1_048_576, "HNSW index data received");

                // Receive and verify
                let staging_path = receiver
                    .receive_index(tenant_id, repo_id, branch, data, crc32)
                    .await
                    .map_err(|e| Error::Backend(format!("Failed to receive HNSW index: {}", e)))?;

                // Ingest
                receiver
                    .ingest_index(&staging_path, tenant_id, repo_id, branch)
                    .await
                    .map_err(|e| Error::Backend(format!("Failed to ingest HNSW index: {}", e)))?;

                // Send acknowledgment
                let ack = ReplicationMessage::HnswIndexAck {
                    status: crate::TransferStatus::Success,
                };
                Self::send_message(stream, &ack).await?;

                Ok(())
            }
            _ => Err(Error::Backend(
                "Unexpected response to RequestHnswIndex".to_string(),
            )),
        }
    }
}
