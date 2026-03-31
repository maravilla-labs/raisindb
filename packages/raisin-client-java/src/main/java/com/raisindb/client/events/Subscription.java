package com.raisindb.client.events;

import com.raisindb.client.operations.EventSubscriptions;

import java.util.concurrent.CompletableFuture;

/**
 * Represents an active event subscription.
 */
public class Subscription {

    private final String subscriptionId;
    private final EventSubscriptions manager;
    private boolean active;

    public Subscription(String subscriptionId, EventSubscriptions manager) {
        this.subscriptionId = subscriptionId;
        this.manager = manager;
        this.active = true;
    }

    /**
     * Unsubscribe from events.
     *
     * @return CompletableFuture that completes when unsubscription succeeds
     */
    public CompletableFuture<Void> unsubscribe() {
        if (!active) {
            return CompletableFuture.completedFuture(null);
        }

        return manager.unsubscribe(subscriptionId)
                .thenApply(result -> {
                    active = false;
                    return null;
                });
    }

    /**
     * Check if subscription is active.
     */
    public boolean isActive() {
        return active;
    }

    public String getSubscriptionId() {
        return subscriptionId;
    }
}
