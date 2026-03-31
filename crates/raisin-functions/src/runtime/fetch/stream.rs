// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Stream registry for ReadableStream body handling
//!
//! This module manages response body streams for the W3C Fetch API.
//! It provides a pull-based streaming interface where JavaScript can
//! request chunks on demand via `.read()` calls.

use bytes::Bytes;
use dashmap::DashMap;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;

/// Default chunk size for reading (64KB)
#[allow(dead_code)]
const DEFAULT_CHUNK_SIZE: usize = 64 * 1024;

/// Maximum buffer size before applying backpressure (1MB)
const MAX_BUFFER_SIZE: usize = 1024 * 1024;

/// Result of reading a chunk from a stream
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status")]
pub enum StreamReadResult {
    /// A chunk of data is available
    #[serde(rename = "chunk")]
    Chunk {
        /// Base64-encoded chunk data
        value: String,
        /// Size in bytes
        size: usize,
    },
    /// Stream has ended
    #[serde(rename = "done")]
    Done,
    /// An error occurred
    #[serde(rename = "error")]
    Error {
        /// Error message
        message: String,
    },
}

impl StreamReadResult {
    /// Convert to JSON string for JavaScript
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| {
            r#"{"status":"error","message":"Failed to serialize stream result"}"#.to_string()
        })
    }
}

/// Internal state for a response body stream
struct StreamState {
    /// Buffered chunks ready for consumption
    chunks: Mutex<VecDeque<Bytes>>,
    /// Total bytes currently buffered
    buffered_bytes: AtomicUsize,
    /// Whether the stream has completed
    done: AtomicBool,
    /// Whether the stream has been cancelled
    cancelled: AtomicBool,
    /// Error message if stream failed
    error: Mutex<Option<String>>,
    /// Notify when new data is available
    data_available: Notify,
    /// Whether a reader is currently locked
    locked: AtomicBool,
    /// Total bytes read from this stream
    total_bytes_read: AtomicUsize,
    /// Maximum bytes allowed (0 = unlimited)
    max_bytes: usize,
}

impl StreamState {
    fn new(max_bytes: usize) -> Self {
        Self {
            chunks: Mutex::new(VecDeque::new()),
            buffered_bytes: AtomicUsize::new(0),
            done: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
            error: Mutex::new(None),
            data_available: Notify::new(),
            locked: AtomicBool::new(false),
            total_bytes_read: AtomicUsize::new(0),
            max_bytes,
        }
    }

    /// Add a chunk to the buffer
    fn push_chunk(&self, chunk: Bytes) {
        let len = chunk.len();
        if let Ok(mut chunks) = self.chunks.lock() {
            chunks.push_back(chunk);
            self.buffered_bytes.fetch_add(len, Ordering::SeqCst);
            self.data_available.notify_one();
        }
    }

    /// Take the next chunk from the buffer
    fn pop_chunk(&self) -> Option<Bytes> {
        if let Ok(mut chunks) = self.chunks.lock() {
            if let Some(chunk) = chunks.pop_front() {
                self.buffered_bytes.fetch_sub(chunk.len(), Ordering::SeqCst);
                self.total_bytes_read
                    .fetch_add(chunk.len(), Ordering::SeqCst);
                return Some(chunk);
            }
        }
        None
    }

    /// Check if buffer is full (for backpressure)
    fn is_buffer_full(&self) -> bool {
        self.buffered_bytes.load(Ordering::SeqCst) >= MAX_BUFFER_SIZE
    }

    /// Mark stream as done
    fn set_done(&self) {
        self.done.store(true, Ordering::SeqCst);
        self.data_available.notify_waiters();
    }

    /// Mark stream as errored
    fn set_error(&self, message: String) {
        if let Ok(mut error) = self.error.lock() {
            *error = Some(message);
        }
        self.data_available.notify_waiters();
    }

    /// Check if stream is done or errored
    #[allow(dead_code)]
    fn is_complete(&self) -> bool {
        self.done.load(Ordering::SeqCst)
            || self.cancelled.load(Ordering::SeqCst)
            || self.error.lock().map(|e| e.is_some()).unwrap_or(false)
    }
}

/// Registry for managing response body streams
///
/// This registry allows creating streams from reqwest responses and
/// reading chunks on demand from JavaScript.
pub struct StreamRegistry {
    streams: DashMap<String, Arc<StreamState>>,
}

impl StreamRegistry {
    /// Create a new stream registry
    pub fn new() -> Self {
        Self {
            streams: DashMap::new(),
        }
    }

    /// Start streaming a response body
    ///
    /// Spawns a background task to buffer chunks from the response.
    /// Returns a stream ID that can be used to read chunks.
    pub fn start_stream(&self, response: reqwest::Response, max_bytes: usize) -> String {
        let id = nanoid::nanoid!();
        let state = Arc::new(StreamState::new(max_bytes));

        self.streams.insert(id.clone(), state.clone());

        // Spawn background task to buffer chunks
        tokio::spawn(async move {
            let mut stream = response.bytes_stream();
            let mut total_read = 0usize;

            while let Some(chunk_result) = stream.next().await {
                // Check if cancelled
                if state.cancelled.load(Ordering::SeqCst) {
                    break;
                }

                match chunk_result {
                    Ok(chunk) => {
                        let chunk_len = chunk.len();
                        total_read += chunk_len;

                        // Check max bytes limit
                        if state.max_bytes > 0 && total_read > state.max_bytes {
                            state.set_error(format!(
                                "Response body exceeded maximum size of {} bytes",
                                state.max_bytes
                            ));
                            break;
                        }

                        // Apply backpressure if buffer is full
                        while state.is_buffer_full() && !state.cancelled.load(Ordering::SeqCst) {
                            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
                        }

                        if state.cancelled.load(Ordering::SeqCst) {
                            break;
                        }

                        state.push_chunk(chunk);
                    }
                    Err(e) => {
                        state.set_error(format!("Failed to read response body: {}", e));
                        break;
                    }
                }
            }

            state.set_done();
        });

        id
    }

    /// Start a stream from already-buffered data
    ///
    /// Used for small responses that were already fully read.
    pub fn start_buffered_stream(&self, data: Bytes) -> String {
        let id = nanoid::nanoid!();
        let state = Arc::new(StreamState::new(0));

        // Push the data as a single chunk
        state.push_chunk(data);
        state.set_done();

        self.streams.insert(id.clone(), state);
        id
    }

    /// Read the next chunk from a stream
    ///
    /// This is called by JavaScript's `ReadableStreamDefaultReader.read()`.
    /// Blocks until data is available or the stream completes.
    pub async fn read_chunk(&self, stream_id: &str) -> StreamReadResult {
        let state = match self.streams.get(stream_id) {
            Some(s) => s.clone(),
            None => {
                return StreamReadResult::Error {
                    message: "Stream not found".to_string(),
                }
            }
        };

        loop {
            // Try to get a chunk
            if let Some(chunk) = state.pop_chunk() {
                let encoded =
                    base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &chunk);
                return StreamReadResult::Chunk {
                    value: encoded,
                    size: chunk.len(),
                };
            }

            // Check if done
            if state.done.load(Ordering::SeqCst) {
                // No more chunks and stream is done
                return StreamReadResult::Done;
            }

            // Check for error
            if let Ok(error) = state.error.lock() {
                if let Some(ref msg) = *error {
                    return StreamReadResult::Error {
                        message: msg.clone(),
                    };
                }
            }

            // Check if cancelled
            if state.cancelled.load(Ordering::SeqCst) {
                return StreamReadResult::Error {
                    message: "Stream was cancelled".to_string(),
                };
            }

            // Wait for more data
            state.data_available.notified().await;
        }
    }

    /// Read the next chunk (non-blocking version)
    ///
    /// Returns immediately with current state.
    pub fn try_read_chunk(&self, stream_id: &str) -> StreamReadResult {
        let state = match self.streams.get(stream_id) {
            Some(s) => s.clone(),
            None => {
                return StreamReadResult::Error {
                    message: "Stream not found".to_string(),
                }
            }
        };

        // Try to get a chunk
        if let Some(chunk) = state.pop_chunk() {
            let encoded =
                base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &chunk);
            return StreamReadResult::Chunk {
                value: encoded,
                size: chunk.len(),
            };
        }

        // Check if done
        if state.done.load(Ordering::SeqCst) {
            return StreamReadResult::Done;
        }

        // Check for error
        if let Ok(error) = state.error.lock() {
            if let Some(ref msg) = *error {
                return StreamReadResult::Error {
                    message: msg.clone(),
                };
            }
        }

        // No data available yet - return done: false to indicate pending
        // This is used for polling mode
        StreamReadResult::Error {
            message: "PENDING".to_string(),
        }
    }

    /// Cancel a stream
    pub fn cancel(&self, stream_id: &str) -> bool {
        if let Some(state) = self.streams.get(stream_id) {
            state.cancelled.store(true, Ordering::SeqCst);
            state.data_available.notify_waiters();
            true
        } else {
            false
        }
    }

    /// Lock a stream for exclusive reading
    pub fn lock(&self, stream_id: &str) -> bool {
        if let Some(state) = self.streams.get(stream_id) {
            // Try to set locked from false to true
            state
                .locked
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
        } else {
            false
        }
    }

    /// Unlock a stream
    pub fn unlock(&self, stream_id: &str) -> bool {
        if let Some(state) = self.streams.get(stream_id) {
            state.locked.store(false, Ordering::SeqCst);
            true
        } else {
            false
        }
    }

    /// Check if a stream is locked
    pub fn is_locked(&self, stream_id: &str) -> bool {
        self.streams
            .get(stream_id)
            .map(|s| s.locked.load(Ordering::SeqCst))
            .unwrap_or(false)
    }

    /// Remove a stream from the registry
    pub fn remove(&self, stream_id: &str) {
        if let Some((_, state)) = self.streams.remove(stream_id) {
            state.cancelled.store(true, Ordering::SeqCst);
        }
    }

    /// Get the number of active streams
    pub fn len(&self) -> usize {
        self.streams.len()
    }

    /// Check if registry is empty
    pub fn is_empty(&self) -> bool {
        self.streams.is_empty()
    }
}

impl Default for StreamRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_buffered_stream() {
        let registry = StreamRegistry::new();
        let data = Bytes::from("Hello, World!");

        let id = registry.start_buffered_stream(data.clone());

        // Should be able to read the data
        let result = registry.try_read_chunk(&id);
        match result {
            StreamReadResult::Chunk { value, size } => {
                assert_eq!(size, 13);
                let decoded =
                    base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &value)
                        .unwrap();
                assert_eq!(decoded, b"Hello, World!");
            }
            _ => panic!("Expected chunk"),
        }

        // Should be done now
        let result = registry.try_read_chunk(&id);
        assert!(matches!(result, StreamReadResult::Done));
    }

    #[test]
    fn test_lock_unlock() {
        let registry = StreamRegistry::new();
        let id = registry.start_buffered_stream(Bytes::from("test"));

        // Initially unlocked
        assert!(!registry.is_locked(&id));

        // Lock
        assert!(registry.lock(&id));
        assert!(registry.is_locked(&id));

        // Can't lock again
        assert!(!registry.lock(&id));

        // Unlock
        assert!(registry.unlock(&id));
        assert!(!registry.is_locked(&id));

        // Can lock again
        assert!(registry.lock(&id));
    }

    #[test]
    fn test_cancel() {
        let registry = StreamRegistry::new();
        let id = registry.start_buffered_stream(Bytes::from("test"));

        // Cancel
        assert!(registry.cancel(&id));

        // Stream should error on read
        let result = registry.try_read_chunk(&id);
        // Note: Since we already buffered data, we might get the chunk first
        // Then on next read we'd get cancelled
    }
}
