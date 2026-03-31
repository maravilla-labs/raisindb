package com.raisindb.client.operations;

import com.raisindb.client.events.Subscription;
import com.raisindb.client.protocol.EventMessage;
import com.raisindb.client.protocol.RequestContext;
import com.raisindb.client.protocol.RequestType;

import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.concurrent.CompletableFuture;
import java.util.function.Consumer;

/**
 * Manages event subscriptions for a workspace.
 */
public class EventSubscriptions {

    private final Workspace workspace;

    public EventSubscriptions(Workspace workspace) {
        this.workspace = workspace;
    }

    /**
     * Subscribe to events with filtering.
     *
     * @param callback   Function to call when events occur
     * @param path       Path pattern filter (supports wildcards: /folder/*, /folder/**)
     * @param eventTypes List of event types to filter
     * @param nodeType   Node type filter
     * @return CompletableFuture with Subscription instance
     */
    public CompletableFuture<Subscription> subscribe(
            Consumer<EventMessage> callback,
            String path,
            List<String> eventTypes,
            String nodeType
    ) {
        RequestContext context = workspace.getContext();

        Map<String, Object> filters = new HashMap<>();
        filters.put("workspace", workspace.getName());
        if (path != null) {
            filters.put("path", path);
        }
        if (eventTypes != null) {
            filters.put("event_types", eventTypes);
        }
        if (nodeType != null) {
            filters.put("node_type", nodeType);
        }

        Map<String, Object> payload = new HashMap<>();
        payload.put("filters", filters);

        return workspace.getDatabase().getClient().getRequestTracker()
                .sendRequest(RequestType.SUBSCRIBE, context, payload)
                .thenApply(result -> {
                    @SuppressWarnings("unchecked")
                    Map<String, Object> response = (Map<String, Object>) result;
                    String subscriptionId = (String) response.get("subscription_id");

                    // Register event handler
                    workspace.getDatabase().getClient()
                            .registerEventHandler(subscriptionId, callback);

                    return new Subscription(subscriptionId, this);
                });
    }

    /**
     * Subscribe to all events on a specific path.
     *
     * @param path     Path pattern (supports wildcards)
     * @param callback Event handler function
     * @return CompletableFuture with Subscription instance
     */
    public CompletableFuture<Subscription> subscribeToPath(String path, Consumer<EventMessage> callback) {
        return subscribe(callback, path, null, null);
    }

    /**
     * Subscribe to a specific event type.
     *
     * @param eventType Event type (e.g., "node:created")
     * @param callback  Event handler function
     * @return CompletableFuture with Subscription instance
     */
    public CompletableFuture<Subscription> subscribeToType(String eventType, Consumer<EventMessage> callback) {
        return subscribe(callback, null, List.of(eventType), null);
    }

    /**
     * Subscribe to events for a specific node type.
     *
     * @param nodeType Node type
     * @param callback Event handler function
     * @return CompletableFuture with Subscription instance
     */
    public CompletableFuture<Subscription> subscribeToNodeType(String nodeType, Consumer<EventMessage> callback) {
        return subscribe(callback, null, null, nodeType);
    }

    /**
     * Unsubscribe from events.
     */
    CompletableFuture<Void> unsubscribe(String subscriptionId) {
        RequestContext context = workspace.getContext();

        Map<String, String> payload = new HashMap<>();
        payload.put("subscription_id", subscriptionId);

        return workspace.getDatabase().getClient().getRequestTracker()
                .sendRequest(RequestType.UNSUBSCRIBE, context, payload)
                .thenApply(result -> {
                    // Unregister event handler
                    workspace.getDatabase().getClient()
                            .unregisterEventHandler(subscriptionId);
                    return null;
                });
    }

    Workspace getWorkspace() {
        return workspace;
    }
}
