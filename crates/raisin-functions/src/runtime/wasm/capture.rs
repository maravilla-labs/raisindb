// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Guest stdout/stderr capture.
//!
//! `println!` from a guest is the wasm equivalent of `console.log`, so it has
//! to reach `ExecutionResult.logs` — but it must never be able to hurt the
//! host. This stream is therefore **bounded and lossy**: once the cap is
//! reached, further bytes are dropped and the stream stays writable.
//!
//! Neither of wasmtime-wasi's own pipes does that. `MemoryOutputPipe` TRAPS
//! the guest on an oversized write and reports a zero write permit when full,
//! which makes a chatty guest spin against a never-ready stream; the
//! `AsyncWriteStream` wrapper defers writes to a background task, so a line
//! printed just before the call returned may not be in the buffer when we read
//! it. Losing the tail of a very verbose log is the acceptable failure here.

use std::sync::{Arc, Mutex};

use bytes::Bytes;
use wasmtime_wasi::cli::{IsTerminal, StdoutStream};
use wasmtime_wasi::p2::{OutputStream, Pollable, StreamResult};

/// One bounded capture buffer, shared by every clone of the stream.
#[derive(Clone)]
pub struct CaptureStream {
    buffer: Arc<Mutex<Vec<u8>>>,
    capacity: usize,
}

impl CaptureStream {
    /// A capture buffer holding at most `capacity` bytes.
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: Arc::new(Mutex::new(Vec::new())),
            capacity,
        }
    }

    /// Everything captured so far, lossily decoded (guest output is not
    /// guaranteed to be UTF-8, and a decoding failure must not lose the logs).
    pub fn contents(&self) -> String {
        match self.buffer.lock() {
            Ok(buffer) => String::from_utf8_lossy(&buffer).into_owned(),
            Err(_) => String::new(),
        }
    }

    fn append(&self, bytes: &[u8]) {
        let Ok(mut buffer) = self.buffer.lock() else {
            return;
        };
        let room = self.capacity.saturating_sub(buffer.len());
        if room == 0 {
            return;
        }
        buffer.extend_from_slice(&bytes[..bytes.len().min(room)]);
    }
}

#[async_trait::async_trait]
impl Pollable for CaptureStream {
    async fn ready(&mut self) {}
}

impl OutputStream for CaptureStream {
    fn write(&mut self, bytes: Bytes) -> StreamResult<()> {
        self.append(&bytes);
        Ok(())
    }

    fn flush(&mut self) -> StreamResult<()> {
        Ok(())
    }

    fn check_write(&mut self) -> StreamResult<usize> {
        // Always writable. Reporting the remaining room instead would make a
        // guest that filled the buffer wait on a stream that never becomes
        // ready — a hang, where dropping bytes is merely lossy.
        Ok(4096)
    }
}

impl IsTerminal for CaptureStream {
    fn is_terminal(&self) -> bool {
        false
    }
}

impl StdoutStream for CaptureStream {
    fn async_stream(&self) -> Box<dyn tokio::io::AsyncWrite + Send + Sync> {
        Box::new(AsyncCapture(self.clone()))
    }

    fn p2_stream(&self) -> Box<dyn OutputStream> {
        Box::new(self.clone())
    }
}

/// `tokio::io::AsyncWrite` face of the same buffer, for the p1/p3 code paths
/// that ask for one. Writes are immediate; nothing is ever pending.
struct AsyncCapture(CaptureStream);

impl tokio::io::AsyncWrite for AsyncCapture {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        self.0.append(buf);
        std::task::Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_is_bounded_and_lossy_not_fatal() {
        let mut stream = CaptureStream::new(8);
        stream.write(Bytes::from_static(b"hello")).unwrap();
        stream.write(Bytes::from_static(b" world")).unwrap();
        // Truncated, but neither write failed: a full buffer must not become a
        // guest trap.
        assert_eq!(stream.contents(), "hello wo");
        assert!(stream.check_write().unwrap() > 0);
    }
}
