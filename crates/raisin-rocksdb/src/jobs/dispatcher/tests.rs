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

//! Dispatcher tests: routing, per-category isolation, and shutdown.
//!
//! Fair-share behaviour across tenants is covered by `jobs::fair::tests`.

use super::*;

/// Priority ordering, for one tenant. Across tenants it is the round
/// robin that decides, which is what `fair::tests` covers.
#[tokio::test]
async fn test_dispatcher_priority_ordering() {
    let (dispatcher, receivers) = JobDispatcher::new();
    let receiver = receivers.get(&JobCategory::Realtime).unwrap().clone();

    let low_job = JobId::new();
    let normal_job = JobId::new();
    let high_job = JobId::new();

    dispatcher
        .dispatch_categorized(
            low_job.clone(),
            JobPriority::Low,
            JobCategory::Realtime,
            "t1",
        )
        .await;
    dispatcher
        .dispatch_categorized(
            normal_job.clone(),
            JobPriority::Normal,
            JobCategory::Realtime,
            "t1",
        )
        .await;
    dispatcher
        .dispatch_categorized(
            high_job.clone(),
            JobPriority::High,
            JobCategory::Realtime,
            "t1",
        )
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
        .dispatch_categorized(
            rt_job.clone(),
            JobPriority::High,
            JobCategory::Realtime,
            "t1",
        )
        .await;
    dispatcher
        .dispatch_categorized(
            bg_job.clone(),
            JobPriority::Normal,
            JobCategory::Background,
            "t1",
        )
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
        .dispatch_categorized(JobId::new(), JobPriority::High, JobCategory::Realtime, "t1")
        .await;
    dispatcher
        .dispatch_categorized(
            JobId::new(),
            JobPriority::Normal,
            JobCategory::Background,
            "t1",
        )
        .await;
    dispatcher
        .dispatch_categorized(JobId::new(), JobPriority::Low, JobCategory::System, "t1")
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
    dispatcher
        .dispatch(job.clone(), JobPriority::High, "t1")
        .await;

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
