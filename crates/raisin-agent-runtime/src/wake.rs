// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Waking a queued run.
//!
//! The service calls [`RunWaker::wake`] after every commit that leaves a run
//! `Queued`; `recover` calls it again for every queued run, so a wake lost to a
//! crash is backstopped. `wake` must therefore be idempotent. The lease epoch —
//! not any dedup key — is what guarantees single execution.

use std::sync::Mutex;

use crate::ids::{RunId, RunScope};
use crate::state::WakeReason;

/// Something that schedules a driver for a queued run.
pub trait RunWaker: Send + Sync {
    /// Schedule `run`.
    fn wake(&self, scope: &RunScope, run: &RunId, reason: WakeReason);
}

/// Does nothing.
#[derive(Debug, Default)]
pub struct NoopWaker;

impl RunWaker for NoopWaker {
    fn wake(&self, _: &RunScope, _: &RunId, _: WakeReason) {}
}

/// Records every wake (tests).
#[derive(Debug, Default)]
pub struct RecordingWaker {
    calls: Mutex<Vec<(RunId, WakeReason)>>,
}

impl RecordingWaker {
    /// Every wake so far.
    pub fn calls(&self) -> Vec<(RunId, WakeReason)> {
        self.calls.lock().expect("waker poisoned").clone()
    }

    /// Forget the recorded wakes (simulates a wake lost to a crash).
    pub fn drain(&self) -> Vec<(RunId, WakeReason)> {
        std::mem::take(&mut *self.calls.lock().expect("waker poisoned"))
    }
}

impl RunWaker for RecordingWaker {
    fn wake(&self, _: &RunScope, run: &RunId, reason: WakeReason) {
        self.calls
            .lock()
            .expect("waker poisoned")
            .push((run.clone(), reason));
    }
}
