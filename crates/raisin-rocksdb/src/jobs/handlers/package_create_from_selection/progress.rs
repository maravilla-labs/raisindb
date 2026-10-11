//! Why a build stops, and the per-chunk progress hook.
//!
//! [`Progress`] owns the counters and the record mirror while the archive is
//! written, and is the [`ChunkHook`](super::archive::ChunkHook) the archive
//! writer calls after every binary chunk: it counts, honours a cancel within
//! one chunk, re-checks free space every [`SPACE_RECHECK_BYTES`], and keeps
//! the record's heartbeat moving during a single multi-gigabyte file.

use super::archive;
use super::estimate;
use super::mirror::{Mirror, RecordSignal, WsCount};
use raisin_binary::DynBinaryStorage;
use raisin_error::{Error, Result};
use std::sync::Arc;
use std::time::Instant;
use tokio_util::sync::CancellationToken;

/// Free space is re-checked every this many bytes written.
pub(super) const SPACE_RECHECK_BYTES: u64 = 64 * 1024 * 1024;

/// Why a build stopped early.
#[derive(Debug)]
pub(super) enum Stop {
    Cancelled,
    /// A refusal with a code from the build-record vocabulary.
    Refused {
        code: &'static str,
        message: String,
        detail: Option<serde_json::Value>,
    },
    Failed(Error),
}

impl From<Error> for Stop {
    fn from(e: Error) -> Self {
        Stop::Failed(e)
    }
}

pub(super) fn io(e: std::io::Error) -> Stop {
    Stop::Failed(Error::storage(format!("Failed to write the archive: {e}")))
}

pub(super) fn blob(e: anyhow::Error) -> Stop {
    Stop::Failed(Error::storage(format!("Blob store: {e}")))
}

pub(super) fn insufficient(
    short: raisin_binary::space::InsufficientStorage,
    written: u64,
    total: u64,
) -> Stop {
    Stop::Refused {
        code: "insufficient_storage",
        message: format!(
            "The server ran out of storage space: need {} bytes free (safety floor included), \
             have {}",
            short.need, short.have
        ),
        detail: Some(serde_json::json!({
            "need": short.need,
            "have": short.have,
            "written": written,
            "total": total,
        })),
    }
}

/// Stop if the record asked for a cancel or vanished, or the token fired.
pub(super) fn check(
    signal: RecordSignal,
    cancel: &CancellationToken,
) -> std::result::Result<(), Stop> {
    match signal {
        RecordSignal::CancelRequested => {
            cancel.cancel();
            Err(Stop::Cancelled)
        }
        RecordSignal::Missing => {
            cancel.cancel();
            Err(Stop::Cancelled)
        }
        RecordSignal::Continue if cancel.is_cancelled() => Err(Stop::Cancelled),
        RecordSignal::Continue => Ok(()),
    }
}

/// Counters, cancel, the in-flight free-space check and the record mirror.
pub(super) struct Progress<'m> {
    pub(super) mirror: &'m mut Mirror,
    store: Arc<dyn DynBinaryStorage>,
    floor: Option<u64>,
    cancel: CancellationToken,
    pub(super) counters: super::mirror::Counters,
    bytes_at_last_space_check: u64,
    started: Instant,
    estimate_bytes: u64,
    /// Why the last hook call failed, so the error maps back to a [`Stop`].
    pending_stop: Option<Stop>,
}

impl<'m> Progress<'m> {
    pub(super) fn new(
        mirror: &'m mut Mirror,
        store: Arc<dyn DynBinaryStorage>,
        floor: Option<u64>,
        cancel: CancellationToken,
        estimate_bytes: u64,
    ) -> Self {
        Self {
            mirror,
            store,
            floor,
            cancel,
            counters: Default::default(),
            bytes_at_last_space_check: 0,
            started: Instant::now(),
            estimate_bytes,
            pending_stop: None,
        }
    }

    pub(super) fn check_cancel(&self) -> std::result::Result<(), Stop> {
        if self.cancel.is_cancelled() {
            Err(Stop::Cancelled)
        } else {
            Ok(())
        }
    }

    /// Map an error out of the archive writer back to why it stopped.
    pub(super) fn stop_for(&mut self, e: Error) -> Stop {
        if let Some(stop) = self.pending_stop.take() {
            return stop;
        }
        if self.cancel.is_cancelled() {
            return Stop::Cancelled;
        }
        Stop::Failed(e)
    }

    pub(super) fn node_done(&mut self, ws: &str, w: &archive::Written) {
        let yaml_bytes = w.line.bytes;
        let bin_bytes = w.line.bin.as_ref().map(|b| b.bytes).unwrap_or(0);
        // Binary bytes were counted chunk by chunk already.
        self.counters.bytes_written += yaml_bytes;
        self.counters.items_written += 1;
        self.counters.files += w.files;
        let e = self
            .counters
            .per_ws
            .entry(ws.to_string())
            .or_insert(WsCount::default());
        e.items += 1;
        e.bytes += yaml_bytes + bin_bytes;
        self.update_rate();
    }

    pub(super) fn update_rate(&mut self) {
        let secs = self.started.elapsed().as_secs_f64();
        if secs > 0.5 {
            self.counters.bytes_per_s = (self.counters.bytes_written as f64 / secs) as u64;
        }
    }

    /// Publish the counters to the mirror, revising the estimate upward once
    /// the build overtakes it (E25), so a progress bar never sits at 100 %
    /// while work continues.
    pub(super) fn revise_estimate(&mut self) {
        self.mirror.state.counters = self.counters.clone();
        self.mirror.state.mark_dirty();
        let Some(est) = self.mirror.state.estimate.as_mut() else {
            return;
        };
        let mut revised = false;
        if self.counters.items_written > est.items {
            est.items = self.counters.items_written + self.counters.items_written / 10;
            revised = true;
        }
        if self.counters.bytes_written > est.bytes {
            est.bytes = self.counters.bytes_written + self.counters.bytes_written / 10;
            revised = true;
        }
        if revised {
            est.revised_at = Some(chrono::Utc::now());
        }
    }

    /// Every [`SPACE_RECHECK_BYTES`], make sure the floor still holds.
    pub(super) async fn recheck_space(&mut self) -> std::result::Result<(), Stop> {
        if self.counters.bytes_written - self.bytes_at_last_space_check < SPACE_RECHECK_BYTES {
            return Ok(());
        }
        self.bytes_at_last_space_check = self.counters.bytes_written;
        let space = self.store.available_space().await.map_err(blob)?;
        if let Err(short) = estimate::room_for(space, 0, self.floor) {
            return Err(insufficient(
                short,
                self.counters.bytes_written,
                self.estimate_bytes,
            ));
        }
        Ok(())
    }
}

impl archive::ChunkHook for Progress<'_> {
    fn chunk(&mut self, n: u64) -> raisin_binary::BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            self.counters.bytes_written += n;
            self.update_rate();
            let outcome = async {
                self.check_cancel()?;
                self.recheck_space().await?;
                self.revise_estimate();
                let signal = self.mirror.tick().await;
                check(signal, &self.cancel)
            }
            .await;
            match outcome {
                Ok(()) => Ok(()),
                Err(stop) => {
                    self.pending_stop = Some(stop);
                    Err(Error::InvalidState("package build stopped".to_string()))
                }
            }
        })
    }
}
