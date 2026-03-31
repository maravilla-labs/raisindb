package com.raisindb.client.operations;

import com.raisindb.client.protocol.RequestContext;
import com.raisindb.client.protocol.RequestType;

import java.util.*;
import java.util.concurrent.CompletableFuture;

/**
 * Tag management operations.
 */
public class Tags {

    private final Database database;

    public Tags(Database database) {
        this.database = database;
    }

    /**
     * Create a new tag.
     *
     * @param name     Tag name
     * @param revision Revision number this tag points to
     * @param message  Optional annotation message
     * @return CompletableFuture with created tag
     */
    public CompletableFuture<Object> create(String name, String revision, String message) {
        RequestContext context = database.getContext(null, null);
        Map<String, Object> payload = new HashMap<>();
        payload.put("name", name);
        payload.put("revision", revision);
        if (message != null) {
            payload.put("message", message);
        }

        return database.getClient().getRequestTracker()
                .sendRequest(RequestType.TAG_CREATE, context, payload);
    }

    /**
     * Create a new tag without a message.
     *
     * @param name     Tag name
     * @param revision Revision number this tag points to
     * @return CompletableFuture with created tag
     */
    public CompletableFuture<Object> create(String name, String revision) {
        return create(name, revision, null);
    }

    /**
     * Get a tag by name.
     *
     * @param name Tag name
     * @return CompletableFuture with tag
     */
    public CompletableFuture<Object> get(String name) {
        RequestContext context = database.getContext(null, null);
        Map<String, Object> payload = new HashMap<>();
        payload.put("name", name);

        return database.getClient().getRequestTracker()
                .sendRequest(RequestType.TAG_GET, context, payload);
    }

    /**
     * List all tags.
     *
     * @return CompletableFuture with list of tags
     */
    public CompletableFuture<List<Object>> list() {
        RequestContext context = database.getContext(null, null);
        Map<String, Object> payload = new HashMap<>();

        return database.getClient().getRequestTracker()
                .sendRequest(RequestType.TAG_LIST, context, payload)
                .thenApply(result -> {
                    if (result instanceof List) {
                        return (List<Object>) result;
                    }
                    return Collections.emptyList();
                });
    }

    /**
     * Delete a tag.
     *
     * @param name Tag name
     * @return CompletableFuture with result
     */
    public CompletableFuture<Object> delete(String name) {
        RequestContext context = database.getContext(null, null);
        Map<String, Object> payload = new HashMap<>();
        payload.put("name", name);

        return database.getClient().getRequestTracker()
                .sendRequest(RequestType.TAG_DELETE, context, payload);
    }
}
