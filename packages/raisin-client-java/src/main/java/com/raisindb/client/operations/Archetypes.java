package com.raisindb.client.operations;

import com.raisindb.client.protocol.RequestContext;
import com.raisindb.client.protocol.RequestType;

import java.util.*;
import java.util.concurrent.CompletableFuture;

/**
 * Archetype management operations.
 */
public class Archetypes {

    private final Database database;

    public Archetypes(Database database) {
        this.database = database;
    }

    /**
     * Create a new Archetype.
     *
     * @param name      Archetype name
     * @param archetype Archetype definition
     * @return CompletableFuture with created Archetype
     */
    public CompletableFuture<Object> create(String name, Map<String, Object> archetype) {
        RequestContext context = database.getContext(null, null);
        Map<String, Object> payload = new HashMap<>();
        payload.put("name", name);
        payload.put("archetype", archetype);

        return database.getClient().getRequestTracker()
                .sendRequest(RequestType.ARCHETYPE_CREATE, context, payload);
    }

    /**
     * Get an Archetype by name.
     *
     * @param name Archetype name
     * @return CompletableFuture with Archetype
     */
    public CompletableFuture<Object> get(String name) {
        RequestContext context = database.getContext(null, null);
        Map<String, Object> payload = new HashMap<>();
        payload.put("name", name);

        return database.getClient().getRequestTracker()
                .sendRequest(RequestType.ARCHETYPE_GET, context, payload);
    }

    /**
     * List all Archetypes.
     *
     * @param publishedOnly If true, list only published Archetypes
     * @return CompletableFuture with list of Archetypes
     */
    public CompletableFuture<List<Object>> list(boolean publishedOnly) {
        RequestContext context = database.getContext(null, null);
        Map<String, Object> payload = new HashMap<>();
        payload.put("published_only", publishedOnly);

        return database.getClient().getRequestTracker()
                .sendRequest(RequestType.ARCHETYPE_LIST, context, payload)
                .thenApply(result -> {
                    if (result instanceof List) {
                        return (List<Object>) result;
                    }
                    return Collections.emptyList();
                });
    }

    /**
     * List all Archetypes (published and unpublished).
     *
     * @return CompletableFuture with list of Archetypes
     */
    public CompletableFuture<List<Object>> list() {
        return list(false);
    }

    /**
     * Update an Archetype.
     *
     * @param name      Archetype name
     * @param archetype Updated Archetype definition
     * @return CompletableFuture with updated Archetype
     */
    public CompletableFuture<Object> update(String name, Map<String, Object> archetype) {
        RequestContext context = database.getContext(null, null);
        Map<String, Object> payload = new HashMap<>();
        payload.put("name", name);
        payload.put("archetype", archetype);

        return database.getClient().getRequestTracker()
                .sendRequest(RequestType.ARCHETYPE_UPDATE, context, payload);
    }

    /**
     * Delete an Archetype.
     *
     * @param name Archetype name
     * @return CompletableFuture with result
     */
    public CompletableFuture<Object> delete(String name) {
        RequestContext context = database.getContext(null, null);
        Map<String, Object> payload = new HashMap<>();
        payload.put("name", name);

        return database.getClient().getRequestTracker()
                .sendRequest(RequestType.ARCHETYPE_DELETE, context, payload);
    }

    /**
     * Publish an Archetype.
     *
     * @param name Archetype name
     * @return CompletableFuture with result
     */
    public CompletableFuture<Object> publish(String name) {
        RequestContext context = database.getContext(null, null);
        Map<String, Object> payload = new HashMap<>();
        payload.put("name", name);

        return database.getClient().getRequestTracker()
                .sendRequest(RequestType.ARCHETYPE_PUBLISH, context, payload);
    }

    /**
     * Unpublish an Archetype.
     *
     * @param name Archetype name
     * @return CompletableFuture with result
     */
    public CompletableFuture<Object> unpublish(String name) {
        RequestContext context = database.getContext(null, null);
        Map<String, Object> payload = new HashMap<>();
        payload.put("name", name);

        return database.getClient().getRequestTracker()
                .sendRequest(RequestType.ARCHETYPE_UNPUBLISH, context, payload);
    }
}
