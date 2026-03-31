//! Index transfer for Tantivy and HNSW (Phase 6).
//!
//! Transfers fulltext (Tantivy) and vector (HNSW) indexes from the source
//! peer with chunked streaming, CRC32 verification, and per-index ingestion.

use super::types::{CatchUpSession, IndexTransferResult, PeerStatus};
use super::CatchUpCoordinator;
use crate::{IndexFileInfo, ReplicationMessage};
use raisin_error::{Error, Result};
use std::path::PathBuf;
use tokio::net::TcpStream;
use tracing::{info, warn};

impl CatchUpCoordinator {
    /// Phase 6: Transfer indexes (Tantivy and HNSW)
    ///
    /// Transfers fulltext and vector indexes from the source peer to enable
    /// complete search functionality after catch-up.
    pub(super) async fn transfer_indexes(
        &self,
        source_peer: &PeerStatus,
        session: &CatchUpSession,
    ) -> Result<IndexTransferResult> {
        info!(
            session_id = %session.session_id,
            "Phase 6: Transferring Tantivy and HNSW indexes"
        );

        let mut tantivy_file_count = 0;
        let mut hnsw_index_count = 0;

        // Get connection to source peer
        let mut connections = self.peer_connections.write().await;
        let stream = connections
            .get_mut(&source_peer.node_id)
            .ok_or_else(|| Error::Backend("Source peer connection lost".to_string()))?;

        // Step 1: Request list of available Tantivy indexes
        info!("Requesting Tantivy index list from source peer");
        let tantivy_indexes = self.request_tantivy_index_list(stream).await?;

        info!(
            num_tantivy_indexes = tantivy_indexes.len(),
            "Received Tantivy index list"
        );

        // Step 2: Transfer each Tantivy index
        if let Some(ref tantivy_receiver) = self.tantivy_receiver {
            for (tenant_id, repo_id, branch) in &tantivy_indexes {
                info!(
                    tenant_id = %tenant_id,
                    repo_id = %repo_id,
                    branch = %branch,
                    "Transferring Tantivy index"
                );

                match self
                    .transfer_single_tantivy_index(
                        stream,
                        tantivy_receiver.as_ref(),
                        tenant_id,
                        repo_id,
                        branch,
                    )
                    .await
                {
                    Ok(file_count) => {
                        tantivy_file_count += file_count;
                        info!(
                            tenant_id = %tenant_id,
                            repo_id = %repo_id,
                            branch = %branch,
                            files_transferred = file_count,
                            "Tantivy index transfer complete"
                        );
                    }
                    Err(e) => {
                        warn!(
                            tenant_id = %tenant_id,
                            repo_id = %repo_id,
                            branch = %branch,
                            error = %e,
                            "Failed to transfer Tantivy index, continuing with next"
                        );
                        // Continue with next index rather than failing entire catch-up
                    }
                }
            }
        } else {
            warn!("No Tantivy receiver configured, skipping Tantivy index transfer");
        }

        // Step 3: Request list of available HNSW indexes
        info!("Requesting HNSW index list from source peer");
        let hnsw_indexes = self.request_hnsw_index_list(stream).await?;

        info!(
            num_hnsw_indexes = hnsw_indexes.len(),
            "Received HNSW index list"
        );

        // Step 4: Transfer each HNSW index
        if let Some(ref hnsw_receiver) = self.hnsw_receiver {
            for (tenant_id, repo_id, branch) in &hnsw_indexes {
                info!(
                    tenant_id = %tenant_id,
                    repo_id = %repo_id,
                    branch = %branch,
                    "Transferring HNSW index"
                );

                match self
                    .transfer_single_hnsw_index(
                        stream,
                        hnsw_receiver.as_ref(),
                        tenant_id,
                        repo_id,
                        branch,
                    )
                    .await
                {
                    Ok(()) => {
                        hnsw_index_count += 1;
                        info!(
                            tenant_id = %tenant_id,
                            repo_id = %repo_id,
                            branch = %branch,
                            "HNSW index transfer complete"
                        );
                    }
                    Err(e) => {
                        warn!(
                            tenant_id = %tenant_id,
                            repo_id = %repo_id,
                            branch = %branch,
                            error = %e,
                            "Failed to transfer HNSW index, continuing with next"
                        );
                        // Continue with next index rather than failing entire catch-up
                    }
                }
            }
        } else {
            warn!("No HNSW receiver configured, skipping HNSW index transfer");
        }

        info!(
            tantivy_files = tantivy_file_count,
            hnsw_indexes = hnsw_index_count,
            "Index transfer phase complete"
        );

        Ok(IndexTransferResult {
            tantivy_files: tantivy_file_count,
            hnsw_indexes: hnsw_index_count,
        })
    }

    /// Request list of available Tantivy indexes from source peer
    async fn request_tantivy_index_list(
        &self,
        stream: &mut TcpStream,
    ) -> Result<Vec<(String, String, String)>> {
        // Send request for Tantivy index list
        let request = ReplicationMessage::RequestTantivyIndexList;
        Self::send_message(stream, &request).await?;

        // Receive response
        let response = Self::receive_message(stream).await?;

        match response {
            ReplicationMessage::TantivyIndexList { indexes } => Ok(indexes),
            _ => Err(Error::Backend(
                "Unexpected response to RequestTantivyIndexList".to_string(),
            )),
        }
    }

    /// Request list of available HNSW indexes from source peer
    async fn request_hnsw_index_list(
        &self,
        stream: &mut TcpStream,
    ) -> Result<Vec<(String, String, String)>> {
        // Send request for HNSW index list
        let request = ReplicationMessage::RequestHnswIndexList;
        Self::send_message(stream, &request).await?;

        // Receive response
        let response = Self::receive_message(stream).await?;

        match response {
            ReplicationMessage::HnswIndexList { indexes } => Ok(indexes),
            _ => Err(Error::Backend(
                "Unexpected response to RequestHnswIndexList".to_string(),
            )),
        }
    }
}
