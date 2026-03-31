//! Parallel file transfer orchestrator with flow control

use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::sync::Semaphore;

use super::file_streamer::ReliableFileStreamer;
use super::types::{ChunkAck, FileChunk, FileInfo, StreamError};

/// Parallel file transfer orchestrator with flow control
pub struct ParallelTransferOrchestrator {
    /// Semaphore for limiting concurrent transfers
    semaphore: Arc<Semaphore>,

    /// File streamer
    streamer: Arc<ReliableFileStreamer>,
}

impl ParallelTransferOrchestrator {
    /// Create a new orchestrator with maximum parallel transfers
    pub fn new(max_parallel: usize) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(max_parallel)),
            streamer: Arc::new(ReliableFileStreamer::new()),
        }
    }

    /// Create with custom chunk size
    pub fn with_chunk_size(max_parallel: usize, chunk_size: usize) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(max_parallel)),
            streamer: Arc::new(ReliableFileStreamer::with_chunk_size(chunk_size)),
        }
    }

    /// Transfer multiple files in parallel with flow control
    ///
    /// This method streams multiple files concurrently while respecting
    /// the maximum parallelism limit to prevent network congestion and OOM.
    pub async fn transfer_files(
        &self,
        files: Vec<FileInfo>,
        chunk_tx: mpsc::Sender<FileChunk>,
        ack_rx: mpsc::Receiver<ChunkAck>,
    ) -> Result<Vec<u32>, StreamError> {
        let mut tasks = Vec::new();

        // Create a shared ack receiver using Arc<Mutex<>>
        let ack_rx = Arc::new(tokio::sync::Mutex::new(ack_rx));

        for file_info in files {
            // Acquire semaphore permit (blocks if max_parallel reached)
            let permit = self
                .semaphore
                .clone()
                .acquire_owned()
                .await
                .map_err(|_| StreamError::SemaphoreError)?;

            let streamer = self.streamer.clone();
            let chunk_tx = chunk_tx.clone();
            let ack_rx = ack_rx.clone();

            let task = tokio::spawn(async move {
                // Create per-file channels
                let (file_chunk_tx, mut file_chunk_rx) = mpsc::channel::<FileChunk>(100);
                let (file_ack_tx, file_ack_rx) = mpsc::channel::<ChunkAck>(100);

                // Forward chunks to main channel
                let chunk_forwarder = tokio::spawn({
                    let chunk_tx = chunk_tx.clone();
                    async move {
                        while let Some(chunk) = file_chunk_rx.recv().await {
                            if chunk_tx.send(chunk).await.is_err() {
                                break;
                            }
                        }
                    }
                });

                // Forward acks from main channel to file channel
                let ack_forwarder = tokio::spawn({
                    let ack_rx = ack_rx.clone();
                    async move {
                        let mut ack_rx = ack_rx.lock().await;
                        while let Some(ack) = ack_rx.recv().await {
                            if file_ack_tx.send(ack).await.is_err() {
                                break;
                            }
                        }
                    }
                });

                // Stream file
                let result = streamer
                    .stream_file(&file_info.path, file_chunk_tx, file_ack_rx)
                    .await;

                // Wait for forwarders
                chunk_forwarder.await.ok();
                ack_forwarder.await.ok();

                drop(permit); // Release permit when done
                result
            });

            tasks.push(task);
        }

        // Wait for all transfers to complete
        let mut results = Vec::new();
        for task in tasks {
            let crc32 = task
                .await
                .map_err(|e| StreamError::TaskJoinError(e.to_string()))??;
            results.push(crc32);
        }

        Ok(results)
    }
}
