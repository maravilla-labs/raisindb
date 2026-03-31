// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! In-memory event bus implementation

use std::sync::{Arc, RwLock};

use crate::{Event, EventBus, EventHandler};

/// Maximum number of concurrent event handlers across all subscribers.
/// This provides backpressure to prevent the system from being overwhelmed
/// when events are published faster than handlers can process them.
const MAX_CONCURRENT_EVENT_HANDLERS: usize = 200;

/// Simple in-memory event bus with backpressure
///
/// Events are dispatched to all subscribers in the order they were registered.
/// Handlers are called asynchronously but event publishing is non-blocking.
///
/// Backpressure is provided via a semaphore that limits concurrent event handlers.
/// When the limit is reached, new handlers wait for a permit before executing.
#[derive(Clone)]
pub struct InMemoryEventBus {
    subscribers: Arc<RwLock<Vec<Arc<dyn EventHandler>>>>,
    /// Semaphore to limit concurrent event handler executions
    handler_semaphore: Arc<tokio::sync::Semaphore>,
}

impl Default for InMemoryEventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemoryEventBus {
    /// Create a new empty event bus with default concurrency limit
    pub fn new() -> Self {
        Self::with_concurrency_limit(MAX_CONCURRENT_EVENT_HANDLERS)
    }

    /// Create a new event bus with custom concurrency limit
    pub fn with_concurrency_limit(limit: usize) -> Self {
        Self {
            subscribers: Arc::new(RwLock::new(Vec::new())),
            handler_semaphore: Arc::new(tokio::sync::Semaphore::new(limit)),
        }
    }

    /// Get current subscriber count (useful for testing)
    pub fn subscriber_count(&self) -> usize {
        self.subscribers.read().unwrap().len()
    }

    /// Get the number of available permits (handlers that can run immediately)
    pub fn available_permits(&self) -> usize {
        self.handler_semaphore.available_permits()
    }
}

impl EventBus for InMemoryEventBus {
    fn publish(&self, event: Event) {
        let subscribers = self.subscribers.read().unwrap();

        let available = self.handler_semaphore.available_permits();
        tracing::debug!(
            "📤 EventBus publishing event - type: {:?}, subscriber_count: {}, available_permits: {}",
            match &event {
                Event::Node(e) => format!("Node({:?})", e.kind),
                Event::Repository(e) => format!("Repository({:?})", e.kind),
                Event::Workspace(e) => format!("Workspace({:?})", e.kind),
                Event::Replication(e) => format!("Replication({:?})", e.kind),
                Event::Schema(e) => format!("Schema({:?})", e.kind),
            },
            subscribers.len(),
            available
        );

        // Warn if we're running low on permits (backpressure building up)
        if available < 20 {
            tracing::warn!(
                "⚠️ EventBus backpressure: only {} permits available (limit: {})",
                available,
                MAX_CONCURRENT_EVENT_HANDLERS
            );
        }

        // Spawn tasks for each handler, with semaphore-based backpressure
        for handler in subscribers.iter() {
            let handler_name = handler.name().to_string();
            let handler = handler.clone();
            let event = event.clone();
            let semaphore = self.handler_semaphore.clone();

            tokio::spawn(async move {
                // Acquire permit before executing handler - provides backpressure
                // If no permits available, this waits (doesn't drop events)
                let _permit = match semaphore.acquire().await {
                    Ok(p) => p,
                    Err(_) => {
                        tracing::error!(
                            "❌ Semaphore closed, dropping event for handler: {}",
                            handler_name
                        );
                        return;
                    }
                };

                tracing::trace!("🎯 Dispatching event to handler: {}", handler_name);
                if let Err(e) = handler.handle(&event).await {
                    // Log error but don't fail event publishing
                    tracing::error!(
                        "❌ Event handler '{}' failed for event {:?}: {}",
                        handler_name,
                        event,
                        e
                    );
                }
                // Permit is automatically released when _permit drops
            });
        }
    }

    fn subscribe(&self, handler: Arc<dyn EventHandler>) {
        let handler_name = handler.name().to_string();
        let mut subscribers = self.subscribers.write().unwrap();
        subscribers.push(handler);
        let count = subscribers.len();
        tracing::info!(
            "✅ EventBus registered subscriber '{}' - total subscribers: {}",
            handler_name,
            count
        );
    }

    fn clear_subscribers(&self) {
        let mut subscribers = self.subscribers.write().unwrap();
        subscribers.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NodeEvent, NodeEventKind};
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::time::{sleep, Duration};

    struct CountingHandler {
        count: Arc<AtomicUsize>,
    }

    impl EventHandler for CountingHandler {
        fn handle<'a>(
            &'a self,
            _event: &'a Event,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + 'a>>
        {
            let count = self.count.clone();
            Box::pin(async move {
                count.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        }

        fn name(&self) -> &str {
            "CountingHandler"
        }
    }

    #[tokio::test]
    async fn test_event_bus_subscribe_and_publish() {
        let bus = InMemoryEventBus::new();
        let count = Arc::new(AtomicUsize::new(0));

        let handler = Arc::new(CountingHandler {
            count: count.clone(),
        });

        bus.subscribe(handler);

        let node_event = NodeEvent {
            tenant_id: "test".to_string(),
            repository_id: "repo1".to_string(),
            branch: "main".to_string(),
            node_id: "node1".to_string(),
            node_type: Some("test:Type".to_string()),
            kind: NodeEventKind::Created,
            path: None,
            metadata: None,
            workspace_id: todo!(),
            revision: todo!(),
        };
        let event = Event::Node(node_event);

        bus.publish(event);

        // Give async handlers time to complete
        sleep(Duration::from_millis(50)).await;

        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_multiple_subscribers() {
        let bus = InMemoryEventBus::new();
        let count1 = Arc::new(AtomicUsize::new(0));
        let count2 = Arc::new(AtomicUsize::new(0));

        bus.subscribe(Arc::new(CountingHandler {
            count: count1.clone(),
        }));
        bus.subscribe(Arc::new(CountingHandler {
            count: count2.clone(),
        }));

        assert_eq!(bus.subscriber_count(), 2);

        let node_event = NodeEvent {
            tenant_id: "test".to_string(),
            repository_id: "repo1".to_string(),
            branch: "main".to_string(),
            node_id: "node1".to_string(),
            node_type: Some("test:Type".to_string()),
            kind: NodeEventKind::Updated,
            path: None,
            metadata: None,
            workspace_id: todo!(),
            revision: todo!(),
        };
        let event = Event::Node(node_event);

        bus.publish(event);

        // Give async handlers time to complete
        sleep(Duration::from_millis(50)).await;

        assert_eq!(count1.load(Ordering::SeqCst), 1);
        assert_eq!(count2.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_clear_subscribers() {
        let bus = InMemoryEventBus::new();
        let count = Arc::new(AtomicUsize::new(0));

        bus.subscribe(Arc::new(CountingHandler {
            count: count.clone(),
        }));

        assert_eq!(bus.subscriber_count(), 1);

        bus.clear_subscribers();

        assert_eq!(bus.subscriber_count(), 0);
    }
}
