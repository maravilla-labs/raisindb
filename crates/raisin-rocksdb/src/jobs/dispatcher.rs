// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! Job dispatcher for channel-based work distribution
//!
//! This module provides a push-based job distribution system with category-aware
//! routing. Jobs are dispatched to per-category priority queues, ensuring that
//! realtime, background, and system jobs are isolated in separate pools.

use async_channel::{bounded, Receiver, Sender};
use raisin_storage::jobs::{JobCategory, JobId, JobPriority};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use tracing::{debug, warn};

/// Queue capacity per priority level
const HIGH_QUEUE_CAPACITY: usize = 10_000;
const NORMAL_QUEUE_CAPACITY: usize = 50_000;
const LOW_QUEUE_CAPACITY: usize = 100_000;

/// Channels for a single category
struct CategoryChannels {
    high_tx: Sender<JobId>,
    normal_tx: Sender<JobId>,
    low_tx: Sender<JobId>,
    dispatched_high: AtomicU64,
    dispatched_normal: AtomicU64,
    dispatched_low: AtomicU64,
}

/// Job dispatcher that routes jobs to category-specific priority queues
///
/// Each category (Realtime, Background, System) gets its own set of
/// High/Normal/Low priority channels, preventing cross-category starvation.
pub struct JobDispatcher {
    categories: HashMap<JobCategory, CategoryChannels>,
}

/// Job receiver for workers in a specific category pool
///
/// Workers use this to receive jobs with priority ordering:
/// High > Normal > Low. Each worker gets its own receiver clone.
#[derive(Clone)]
pub struct JobReceiver {
    high_rx: Receiver<JobId>,
    normal_rx: Receiver<JobId>,
    low_rx: Receiver<JobId>,
}

/// Statistics about a single category's queue state
#[derive(Debug, Clone, Default)]
pub struct CategoryQueueStats {
    pub high_queue_len: usize,
    pub normal_queue_len: usize,
    pub low_queue_len: usize,
    pub total_high_dispatched: u64,
    pub total_normal_dispatched: u64,
    pub total_low_dispatched: u64,
}

/// Statistics about dispatcher queue state (aggregate across all categories)
#[derive(Debug, Clone)]
pub struct DispatcherStats {
    /// Number of jobs in high priority queue (aggregate)
    pub high_queue_len: usize,
    /// Number of jobs in normal priority queue (aggregate)
    pub normal_queue_len: usize,
    /// Number of jobs in low priority queue (aggregate)
    pub low_queue_len: usize,
    /// Total jobs dispatched to high priority (aggregate)
    pub total_high_dispatched: u64,
    /// Total jobs dispatched to normal priority (aggregate)
    pub total_normal_dispatched: u64,
    /// Total jobs dispatched to low priority (aggregate)
    pub total_low_dispatched: u64,
    /// Per-category stats
    pub category_stats: HashMap<JobCategory, CategoryQueueStats>,
}

impl JobDispatcher {
    /// Create a new dispatcher with per-category receivers
    ///
    /// Returns the dispatcher and a map of receivers (one per category).
    pub fn new() -> (Self, HashMap<JobCategory, JobReceiver>) {
        let mut categories = HashMap::new();
        let mut receivers = HashMap::new();

        for category in [JobCategory::Realtime, JobCategory::Background, JobCategory::System] {
            let (high_tx, high_rx) = bounded(HIGH_QUEUE_CAPACITY);
            let (normal_tx, normal_rx) = bounded(NORMAL_QUEUE_CAPACITY);
            let (low_tx, low_rx) = bounded(LOW_QUEUE_CAPACITY);

            categories.insert(
                category,
                CategoryChannels {
                    high_tx,
                    normal_tx,
                    low_tx,
                    dispatched_high: AtomicU64::new(0),
                    dispatched_normal: AtomicU64::new(0),
                    dispatched_low: AtomicU64::new(0),
                },
            );

            receivers.insert(
                category,
                JobReceiver {
                    high_rx,
                    normal_rx,
                    low_rx,
                },
            );
        }

        (Self { categories }, receivers)
    }

    /// Dispatch a job to the appropriate category and priority queue
    pub async fn dispatch_categorized(
        &self,
        job_id: JobId,
        priority: JobPriority,
        category: JobCategory,
    ) {
        let channels = match self.categories.get(&category) {
            Some(ch) => ch,
            None => {
                warn!(
                    job_id = %job_id,
                    category = %category,
                    "No channels for category, falling back to Realtime"
                );
                self.categories.get(&JobCategory::Realtime).unwrap()
            }
        };

        let (sender, counter) = match priority {
            JobPriority::High => (&channels.high_tx, &channels.dispatched_high),
            JobPriority::Normal => (&channels.normal_tx, &channels.dispatched_normal),
            JobPriority::Low => (&channels.low_tx, &channels.dispatched_low),
        };

        match sender.send(job_id.clone()).await {
            Ok(()) => {
                counter.fetch_add(1, Ordering::Relaxed);
                debug!(
                    job_id = %job_id,
                    priority = %priority,
                    category = %category,
                    "Job dispatched to queue"
                );
            }
            Err(e) => {
                warn!(
                    job_id = %job_id,
                    error = %e,
                    "Failed to dispatch job - channel closed"
                );
            }
        }
    }

    /// Dispatch a job using default Realtime category (backward compatibility)
    pub async fn dispatch(&self, job_id: JobId, priority: JobPriority) {
        self.dispatch_categorized(job_id, priority, JobCategory::Realtime)
            .await;
    }

    /// Try to dispatch a job without blocking
    pub fn try_dispatch(&self, job_id: JobId, priority: JobPriority) -> bool {
        self.try_dispatch_categorized(job_id, priority, JobCategory::Realtime)
    }

    /// Try to dispatch a job to a specific category without blocking
    pub fn try_dispatch_categorized(
        &self,
        job_id: JobId,
        priority: JobPriority,
        category: JobCategory,
    ) -> bool {
        let channels = match self.categories.get(&category) {
            Some(ch) => ch,
            None => return false,
        };

        let (sender, counter) = match priority {
            JobPriority::High => (&channels.high_tx, &channels.dispatched_high),
            JobPriority::Normal => (&channels.normal_tx, &channels.dispatched_normal),
            JobPriority::Low => (&channels.low_tx, &channels.dispatched_low),
        };

        match sender.try_send(job_id.clone()) {
            Ok(()) => {
                counter.fetch_add(1, Ordering::Relaxed);
                debug!(
                    job_id = %job_id,
                    priority = %priority,
                    category = %category,
                    "Job dispatched to queue (non-blocking)"
                );
                true
            }
            Err(async_channel::TrySendError::Full(_)) => {
                warn!(
                    job_id = %job_id,
                    priority = %priority,
                    category = %category,
                    "Queue full - job not dispatched"
                );
                false
            }
            Err(async_channel::TrySendError::Closed(_)) => {
                warn!(
                    job_id = %job_id,
                    "Channel closed - job not dispatched"
                );
                false
            }
        }
    }

    /// Get current queue statistics (aggregate + per-category)
    pub fn stats(&self) -> DispatcherStats {
        let mut total_high = 0usize;
        let mut total_normal = 0usize;
        let mut total_low = 0usize;
        let mut total_high_dispatched = 0u64;
        let mut total_normal_dispatched = 0u64;
        let mut total_low_dispatched = 0u64;
        let mut category_stats = HashMap::new();

        for (&cat, ch) in &self.categories {
            let cat_stats = CategoryQueueStats {
                high_queue_len: ch.high_tx.len(),
                normal_queue_len: ch.normal_tx.len(),
                low_queue_len: ch.low_tx.len(),
                total_high_dispatched: ch.dispatched_high.load(Ordering::Relaxed),
                total_normal_dispatched: ch.dispatched_normal.load(Ordering::Relaxed),
                total_low_dispatched: ch.dispatched_low.load(Ordering::Relaxed),
            };
            total_high += cat_stats.high_queue_len;
            total_normal += cat_stats.normal_queue_len;
            total_low += cat_stats.low_queue_len;
            total_high_dispatched += cat_stats.total_high_dispatched;
            total_normal_dispatched += cat_stats.total_normal_dispatched;
            total_low_dispatched += cat_stats.total_low_dispatched;
            category_stats.insert(cat, cat_stats);
        }

        DispatcherStats {
            high_queue_len: total_high,
            normal_queue_len: total_normal,
            low_queue_len: total_low,
            total_high_dispatched,
            total_normal_dispatched,
            total_low_dispatched,
            category_stats,
        }
    }

    /// Close all channels, signaling workers to shut down
    pub fn close(&self) {
        for ch in self.categories.values() {
            ch.high_tx.close();
            ch.normal_tx.close();
            ch.low_tx.close();
        }
    }
}

impl JobReceiver {
    /// Receive the next job, prioritizing high > normal > low
    pub async fn recv(&self) -> Option<JobId> {
        // First, try non-blocking receives in priority order
        if let Ok(job_id) = self.high_rx.try_recv() {
            return Some(job_id);
        }
        if let Ok(job_id) = self.normal_rx.try_recv() {
            return Some(job_id);
        }
        if let Ok(job_id) = self.low_rx.try_recv() {
            return Some(job_id);
        }

        // All queues empty - wait on any queue with priority bias
        tokio::select! {
            biased;

            result = self.high_rx.recv() => {
                result.ok()
            }
            result = self.normal_rx.recv() => {
                result.ok()
            }
            result = self.low_rx.recv() => {
                result.ok()
            }
        }
    }

    /// Try to receive a job without blocking
    pub fn try_recv(&self) -> Option<JobId> {
        if let Ok(job_id) = self.high_rx.try_recv() {
            return Some(job_id);
        }
        if let Ok(job_id) = self.normal_rx.try_recv() {
            return Some(job_id);
        }
        if let Ok(job_id) = self.low_rx.try_recv() {
            return Some(job_id);
        }
        None
    }

    /// Check if all channels are closed
    pub fn is_closed(&self) -> bool {
        self.high_rx.is_closed() && self.normal_rx.is_closed() && self.low_rx.is_closed()
    }

    /// Get the total number of pending jobs across all queues
    pub fn pending_count(&self) -> usize {
        self.high_rx.len() + self.normal_rx.len() + self.low_rx.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_dispatcher_priority_ordering() {
        let (dispatcher, receivers) = JobDispatcher::new();
        let receiver = receivers.get(&JobCategory::Realtime).unwrap().clone();

        let low_job = JobId::new();
        let normal_job = JobId::new();
        let high_job = JobId::new();

        dispatcher
            .dispatch_categorized(low_job.clone(), JobPriority::Low, JobCategory::Realtime)
            .await;
        dispatcher
            .dispatch_categorized(normal_job.clone(), JobPriority::Normal, JobCategory::Realtime)
            .await;
        dispatcher
            .dispatch_categorized(high_job.clone(), JobPriority::High, JobCategory::Realtime)
            .await;

        assert_eq!(receiver.recv().await.unwrap(), high_job);
        assert_eq!(receiver.recv().await.unwrap(), normal_job);
        assert_eq!(receiver.recv().await.unwrap(), low_job);
    }

    #[tokio::test]
    async fn test_category_isolation() {
        let (dispatcher, receivers) = JobDispatcher::new();
        let rt_receiver = receivers.get(&JobCategory::Realtime).unwrap().clone();
        let bg_receiver = receivers.get(&JobCategory::Background).unwrap().clone();

        let rt_job = JobId::new();
        let bg_job = JobId::new();

        dispatcher
            .dispatch_categorized(rt_job.clone(), JobPriority::High, JobCategory::Realtime)
            .await;
        dispatcher
            .dispatch_categorized(bg_job.clone(), JobPriority::Normal, JobCategory::Background)
            .await;

        // Each receiver only gets its own category's jobs
        assert_eq!(rt_receiver.recv().await.unwrap(), rt_job);
        assert_eq!(bg_receiver.recv().await.unwrap(), bg_job);

        // Realtime receiver should NOT receive the background job
        assert!(rt_receiver.try_recv().is_none());
    }

    #[tokio::test]
    async fn test_dispatcher_stats() {
        let (dispatcher, _receivers) = JobDispatcher::new();

        dispatcher
            .dispatch_categorized(JobId::new(), JobPriority::High, JobCategory::Realtime)
            .await;
        dispatcher
            .dispatch_categorized(JobId::new(), JobPriority::Normal, JobCategory::Background)
            .await;
        dispatcher
            .dispatch_categorized(JobId::new(), JobPriority::Low, JobCategory::System)
            .await;

        let stats = dispatcher.stats();
        assert_eq!(stats.high_queue_len, 1);
        assert_eq!(stats.normal_queue_len, 1);
        assert_eq!(stats.low_queue_len, 1);

        // Per-category check
        let rt_stats = stats.category_stats.get(&JobCategory::Realtime).unwrap();
        assert_eq!(rt_stats.high_queue_len, 1);
        assert_eq!(rt_stats.normal_queue_len, 0);

        let bg_stats = stats.category_stats.get(&JobCategory::Background).unwrap();
        assert_eq!(bg_stats.normal_queue_len, 1);
    }

    #[tokio::test]
    async fn test_backward_compat_dispatch() {
        let (dispatcher, receivers) = JobDispatcher::new();
        let rt_receiver = receivers.get(&JobCategory::Realtime).unwrap().clone();

        let job = JobId::new();
        // Old dispatch() should route to Realtime
        dispatcher.dispatch(job.clone(), JobPriority::High).await;

        assert_eq!(rt_receiver.recv().await.unwrap(), job);
    }

    #[tokio::test]
    async fn test_channel_close() {
        let (dispatcher, receivers) = JobDispatcher::new();
        let receiver = receivers.get(&JobCategory::Realtime).unwrap().clone();

        dispatcher.close();

        assert!(receiver.recv().await.is_none());
        assert!(receiver.is_closed());
    }
}
