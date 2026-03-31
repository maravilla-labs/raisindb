package com.raisindb.client.operations;

import com.raisindb.client.protocol.RequestContext;
import com.raisindb.client.protocol.RequestType;

import java.util.*;
import java.util.concurrent.CompletableFuture;

/**
 * Branch management operations.
 */
public class Branches {

    private final Database database;

    public Branches(Database database) {
        this.database = database;
    }

    /**
     * Create a new branch.
     *
     * @param name         Branch name
     * @param fromRevision Optional revision to branch from
     * @param fromBranch   Optional branch name to branch from
     * @return CompletableFuture with created branch
     */
    public CompletableFuture<Object> create(String name, String fromRevision, String fromBranch) {
        RequestContext context = database.getContext(null, null);
        Map<String, Object> payload = new HashMap<>();
        payload.put("name", name);
        if (fromRevision != null) {
            payload.put("from_revision", fromRevision);
        }
        if (fromBranch != null) {
            payload.put("from_branch", fromBranch);
        }

        return database.getClient().getRequestTracker()
                .sendRequest(RequestType.BRANCH_CREATE, context, payload);
    }

    /**
     * Create a new branch from HEAD.
     *
     * @param name Branch name
     * @return CompletableFuture with created branch
     */
    public CompletableFuture<Object> create(String name) {
        return create(name, null, null);
    }

    /**
     * Get a branch by name.
     *
     * @param name Branch name
     * @return CompletableFuture with branch
     */
    public CompletableFuture<Object> get(String name) {
        RequestContext context = database.getContext(null, null);
        Map<String, Object> payload = new HashMap<>();
        payload.put("name", name);

        return database.getClient().getRequestTracker()
                .sendRequest(RequestType.BRANCH_GET, context, payload);
    }

    /**
     * List all branches.
     *
     * @return CompletableFuture with list of branches
     */
    public CompletableFuture<List<Object>> list() {
        RequestContext context = database.getContext(null, null);
        Map<String, Object> payload = new HashMap<>();

        return database.getClient().getRequestTracker()
                .sendRequest(RequestType.BRANCH_LIST, context, payload)
                .thenApply(result -> {
                    if (result instanceof List) {
                        return (List<Object>) result;
                    }
                    return Collections.emptyList();
                });
    }

    /**
     * Delete a branch.
     *
     * @param name Branch name
     * @return CompletableFuture with result
     */
    public CompletableFuture<Object> delete(String name) {
        RequestContext context = database.getContext(null, null);
        Map<String, Object> payload = new HashMap<>();
        payload.put("name", name);

        return database.getClient().getRequestTracker()
                .sendRequest(RequestType.BRANCH_DELETE, context, payload);
    }

    /**
     * Get the HEAD revision of a branch.
     *
     * @param name Branch name
     * @return CompletableFuture with HEAD revision
     */
    public CompletableFuture<Object> getHead(String name) {
        RequestContext context = database.getContext(null, null);
        Map<String, Object> payload = new HashMap<>();
        payload.put("name", name);

        return database.getClient().getRequestTracker()
                .sendRequest(RequestType.BRANCH_GET_HEAD, context, payload);
    }

    /**
     * Update the HEAD revision of a branch.
     *
     * @param name     Branch name
     * @param revision New HEAD revision
     * @return CompletableFuture with result
     */
    public CompletableFuture<Object> updateHead(String name, String revision) {
        RequestContext context = database.getContext(null, null);
        Map<String, Object> payload = new HashMap<>();
        payload.put("name", name);
        payload.put("revision", revision);

        return database.getClient().getRequestTracker()
                .sendRequest(RequestType.BRANCH_UPDATE_HEAD, context, payload);
    }

    /**
     * Merge a source branch into a target branch.
     *
     * @param sourceBranch Source branch name
     * @param targetBranch Target branch name
     * @param strategy     Merge strategy (optional, e.g., "fast_forward" or "three_way")
     * @param message      Merge commit message (optional)
     * @return CompletableFuture with merge result
     */
    public CompletableFuture<Object> merge(String sourceBranch, String targetBranch, String strategy, String message) {
        RequestContext context = database.getContext(null, null);
        Map<String, Object> payload = new HashMap<>();
        payload.put("source_branch", sourceBranch);
        payload.put("target_branch", targetBranch);
        if (strategy != null) {
            payload.put("strategy", strategy);
        }
        if (message != null) {
            payload.put("message", message);
        }

        return database.getClient().getRequestTracker()
                .sendRequest(RequestType.BRANCH_MERGE, context, payload);
    }

    /**
     * Merge a source branch into a target branch with default strategy.
     *
     * @param sourceBranch Source branch name
     * @param targetBranch Target branch name
     * @return CompletableFuture with merge result
     */
    public CompletableFuture<Object> merge(String sourceBranch, String targetBranch) {
        return merge(sourceBranch, targetBranch, null, null);
    }

    /**
     * Compare two branches to calculate divergence.
     *
     * @param branch     Branch name
     * @param baseBranch Base branch name to compare against
     * @return CompletableFuture with divergence information
     */
    public CompletableFuture<Object> compare(String branch, String baseBranch) {
        RequestContext context = database.getContext(null, null);
        Map<String, Object> payload = new HashMap<>();
        payload.put("branch", branch);
        payload.put("base_branch", baseBranch);

        return database.getClient().getRequestTracker()
                .sendRequest(RequestType.BRANCH_COMPARE, context, payload);
    }
}
