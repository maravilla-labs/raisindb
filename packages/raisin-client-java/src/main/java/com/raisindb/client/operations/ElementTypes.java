package com.raisindb.client.operations;

import com.raisindb.client.protocol.RequestContext;
import com.raisindb.client.protocol.RequestType;

import java.util.*;
import java.util.concurrent.CompletableFuture;

/**
 * ElementType management operations.
 */
public class ElementTypes {

    private final Database database;

    public ElementTypes(Database database) {
        this.database = database;
    }

    /**
     * Create a new ElementType.
     *
     * @param name        ElementType name
     * @param elementType ElementType definition
     * @return CompletableFuture with created ElementType
     */
    public CompletableFuture<Object> create(String name, Map<String, Object> elementType) {
        RequestContext context = database.getContext(null, null);
        Map<String, Object> payload = new HashMap<>();
        payload.put("name", name);
        payload.put("element_type", elementType);

        return database.getClient().getRequestTracker()
                .sendRequest(RequestType.ELEMENT_TYPE_CREATE, context, payload);
    }

    /**
     * Get an ElementType by name.
     *
     * @param name ElementType name
     * @return CompletableFuture with ElementType
     */
    public CompletableFuture<Object> get(String name) {
        RequestContext context = database.getContext(null, null);
        Map<String, Object> payload = new HashMap<>();
        payload.put("name", name);

        return database.getClient().getRequestTracker()
                .sendRequest(RequestType.ELEMENT_TYPE_GET, context, payload);
    }

    /**
     * List all ElementTypes.
     *
     * @param publishedOnly If true, list only published ElementTypes
     * @return CompletableFuture with list of ElementTypes
     */
    public CompletableFuture<List<Object>> list(boolean publishedOnly) {
        RequestContext context = database.getContext(null, null);
        Map<String, Object> payload = new HashMap<>();
        payload.put("published_only", publishedOnly);

        return database.getClient().getRequestTracker()
                .sendRequest(RequestType.ELEMENT_TYPE_LIST, context, payload)
                .thenApply(result -> {
                    if (result instanceof List) {
                        return (List<Object>) result;
                    }
                    return Collections.emptyList();
                });
    }

    /**
     * List all ElementTypes (published and unpublished).
     *
     * @return CompletableFuture with list of ElementTypes
     */
    public CompletableFuture<List<Object>> list() {
        return list(false);
    }

    /**
     * Update an ElementType.
     *
     * @param name        ElementType name
     * @param elementType Updated ElementType definition
     * @return CompletableFuture with updated ElementType
     */
    public CompletableFuture<Object> update(String name, Map<String, Object> elementType) {
        RequestContext context = database.getContext(null, null);
        Map<String, Object> payload = new HashMap<>();
        payload.put("name", name);
        payload.put("element_type", elementType);

        return database.getClient().getRequestTracker()
                .sendRequest(RequestType.ELEMENT_TYPE_UPDATE, context, payload);
    }

    /**
     * Delete an ElementType.
     *
     * @param name ElementType name
     * @return CompletableFuture with result
     */
    public CompletableFuture<Object> delete(String name) {
        RequestContext context = database.getContext(null, null);
        Map<String, Object> payload = new HashMap<>();
        payload.put("name", name);

        return database.getClient().getRequestTracker()
                .sendRequest(RequestType.ELEMENT_TYPE_DELETE, context, payload);
    }

    /**
     * Publish an ElementType.
     *
     * @param name ElementType name
     * @return CompletableFuture with result
     */
    public CompletableFuture<Object> publish(String name) {
        RequestContext context = database.getContext(null, null);
        Map<String, Object> payload = new HashMap<>();
        payload.put("name", name);

        return database.getClient().getRequestTracker()
                .sendRequest(RequestType.ELEMENT_TYPE_PUBLISH, context, payload);
    }

    /**
     * Unpublish an ElementType.
     *
     * @param name ElementType name
     * @return CompletableFuture with result
     */
    public CompletableFuture<Object> unpublish(String name) {
        RequestContext context = database.getContext(null, null);
        Map<String, Object> payload = new HashMap<>();
        payload.put("name", name);

        return database.getClient().getRequestTracker()
                .sendRequest(RequestType.ELEMENT_TYPE_UNPUBLISH, context, payload);
    }
}
