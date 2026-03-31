package com.raisindb.client.operations;

import com.raisindb.client.protocol.RequestContext;
import com.raisindb.client.protocol.RequestType;

import java.util.*;
import java.util.concurrent.CompletableFuture;

/**
 * NodeType management operations.
 */
public class NodeTypes {

    private final Database database;

    public NodeTypes(Database database) {
        this.database = database;
    }

    /**
     * Create a new NodeType.
     *
     * @param name     NodeType name
     * @param nodeType NodeType definition
     * @return CompletableFuture with created NodeType
     */
    public CompletableFuture<Object> create(String name, Map<String, Object> nodeType) {
        RequestContext context = database.getContext(null, null);
        Map<String, Object> payload = new HashMap<>();
        payload.put("name", name);
        payload.put("node_type", nodeType);

        return database.getClient().getRequestTracker()
                .sendRequest(RequestType.NODE_TYPE_CREATE, context, payload);
    }

    /**
     * Get a NodeType by name.
     *
     * @param name NodeType name
     * @return CompletableFuture with NodeType
     */
    public CompletableFuture<Object> get(String name) {
        RequestContext context = database.getContext(null, null);
        Map<String, Object> payload = new HashMap<>();
        payload.put("name", name);

        return database.getClient().getRequestTracker()
                .sendRequest(RequestType.NODE_TYPE_GET, context, payload);
    }

    /**
     * List all NodeTypes.
     *
     * @param publishedOnly If true, list only published NodeTypes
     * @return CompletableFuture with list of NodeTypes
     */
    public CompletableFuture<List<Object>> list(boolean publishedOnly) {
        RequestContext context = database.getContext(null, null);
        Map<String, Object> payload = new HashMap<>();
        payload.put("published_only", publishedOnly);

        return database.getClient().getRequestTracker()
                .sendRequest(RequestType.NODE_TYPE_LIST, context, payload)
                .thenApply(result -> {
                    if (result instanceof List) {
                        return (List<Object>) result;
                    }
                    return Collections.emptyList();
                });
    }

    /**
     * List all NodeTypes (published and unpublished).
     *
     * @return CompletableFuture with list of NodeTypes
     */
    public CompletableFuture<List<Object>> list() {
        return list(false);
    }

    /**
     * Update a NodeType.
     *
     * @param name     NodeType name
     * @param nodeType Updated NodeType definition
     * @return CompletableFuture with updated NodeType
     */
    public CompletableFuture<Object> update(String name, Map<String, Object> nodeType) {
        RequestContext context = database.getContext(null, null);
        Map<String, Object> payload = new HashMap<>();
        payload.put("name", name);
        payload.put("node_type", nodeType);

        return database.getClient().getRequestTracker()
                .sendRequest(RequestType.NODE_TYPE_UPDATE, context, payload);
    }

    /**
     * Delete a NodeType.
     *
     * @param name NodeType name
     * @return CompletableFuture with result
     */
    public CompletableFuture<Object> delete(String name) {
        RequestContext context = database.getContext(null, null);
        Map<String, Object> payload = new HashMap<>();
        payload.put("name", name);

        return database.getClient().getRequestTracker()
                .sendRequest(RequestType.NODE_TYPE_DELETE, context, payload);
    }

    /**
     * Publish a NodeType.
     *
     * @param name NodeType name
     * @return CompletableFuture with result
     */
    public CompletableFuture<Object> publish(String name) {
        RequestContext context = database.getContext(null, null);
        Map<String, Object> payload = new HashMap<>();
        payload.put("name", name);

        return database.getClient().getRequestTracker()
                .sendRequest(RequestType.NODE_TYPE_PUBLISH, context, payload);
    }

    /**
     * Unpublish a NodeType.
     *
     * @param name NodeType name
     * @return CompletableFuture with result
     */
    public CompletableFuture<Object> unpublish(String name) {
        RequestContext context = database.getContext(null, null);
        Map<String, Object> payload = new HashMap<>();
        payload.put("name", name);

        return database.getClient().getRequestTracker()
                .sendRequest(RequestType.NODE_TYPE_UNPUBLISH, context, payload);
    }

    /**
     * Validate a node against its NodeType.
     *
     * @param node Node to validate
     * @return CompletableFuture with validation result
     */
    public CompletableFuture<Object> validate(Map<String, Object> node) {
        RequestContext context = database.getContext(null, null);
        Map<String, Object> payload = new HashMap<>();
        payload.put("node", node);

        return database.getClient().getRequestTracker()
                .sendRequest(RequestType.NODE_TYPE_VALIDATE, context, payload);
    }

    /**
     * Get resolved NodeType with full inheritance applied.
     *
     * @param name NodeType name
     * @return CompletableFuture with resolved NodeType
     */
    public CompletableFuture<Object> getResolved(String name) {
        RequestContext context = database.getContext(null, null);
        Map<String, Object> payload = new HashMap<>();
        payload.put("name", name);

        return database.getClient().getRequestTracker()
                .sendRequest(RequestType.NODE_TYPE_GET_RESOLVED, context, payload);
    }
}
