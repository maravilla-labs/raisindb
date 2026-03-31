package com.raisindb.client.operations;

import com.fasterxml.jackson.databind.ObjectMapper;
import com.raisindb.client.RaisinClient;
import com.raisindb.client.protocol.*;

import java.util.*;
import java.util.concurrent.CompletableFuture;

/**
 * Represents a database (repository) in RaisinDB.
 */
public class Database {

    private final RaisinClient client;
    private final String name;
    private final ObjectMapper objectMapper;
    private final String branch;
    private final String revision;

    public Database(RaisinClient client, String name) {
        this(client, name, null, null);
    }

    private Database(RaisinClient client, String name, String branch, String revision) {
        this.client = client;
        this.name = name;
        this.branch = branch;
        this.revision = revision;
        this.objectMapper = new ObjectMapper();
        this.objectMapper.registerModule(new com.fasterxml.jackson.datatype.jsr310.JavaTimeModule());
    }

    /**
     * Get a workspace interface.
     *
     * @param name Workspace name
     * @return Workspace instance
     */
    public Workspace workspace(String name) {
        return new Workspace(this, name);
    }

    /**
     * Get NodeTypes management interface.
     *
     * @return NodeTypes instance
     */
    public NodeTypes nodeTypes() {
        return new NodeTypes(this);
    }

    /**
     * Get Archetypes management interface.
     *
     * @return Archetypes instance
     */
    public Archetypes archetypes() {
        return new Archetypes(this);
    }

    /**
     * Get ElementTypes management interface.
     *
     * @return ElementTypes instance
     */
    public ElementTypes elementTypes() {
        return new ElementTypes(this);
    }

    /**
     * Get Branches management interface.
     *
     * @return Branches instance
     */
    public Branches branches() {
        return new Branches(this);
    }

    /**
     * Get Tags management interface.
     *
     * @return Tags instance
     */
    public Tags tags() {
        return new Tags(this);
    }

    /**
     * List all workspaces in this database.
     *
     * @return CompletableFuture with list of workspaces
     */
    public CompletableFuture<List<Object>> listWorkspaces() {
        RequestContext context = getContext(null, null);
        return client.getRequestTracker()
                .sendRequest(RequestType.WORKSPACE_LIST, context, Collections.emptyMap())
                .thenApply(result -> {
                    if (result instanceof List) {
                        return (List<Object>) result;
                    }
                    return Collections.emptyList();
                });
    }

    /**
     * Create a new workspace.
     *
     * @param name        Workspace name
     * @param description Optional description
     * @return CompletableFuture with created workspace
     */
    public CompletableFuture<Object> createWorkspace(String name, String description) {
        RequestContext context = getContext(null, null);

        Map<String, Object> payload = new HashMap<>();
        payload.put("name", name);
        if (description != null) {
            payload.put("description", description);
        }

        return client.getRequestTracker()
                .sendRequest(RequestType.WORKSPACE_CREATE, context, payload);
    }

    /**
     * Execute a SQL query with parameter binding.
     *
     * @param query  SQL query string (use ? for parameters)
     * @param params Query parameters
     * @return CompletableFuture with SqlResult
     */
    public CompletableFuture<SqlResult> sql(String query, Object... params) {
        RequestContext context = getContext(null, null);

        Map<String, Object> payload = new HashMap<>();
        payload.put("query", query);
        if (params != null && params.length > 0) {
            payload.put("params", Arrays.asList(params));
        }

        return client.getRequestTracker()
                .sendRequest(RequestType.SQL_QUERY, context, payload)
                .thenApply(result -> objectMapper.convertValue(result, SqlResult.class));
    }

    /**
     * Create a new Database instance scoped to a specific branch.
     *
     * @param branch Branch name
     * @return New Database instance with branch context
     */
    public Database onBranch(String branch) {
        return new Database(client, name, branch, this.revision);
    }

    /**
     * Create a new Database instance scoped to a specific revision/commit.
     *
     * @param revision Revision/commit ID
     * @return New Database instance with revision context
     */
    public Database atRevision(String revision) {
        return new Database(client, name, this.branch, revision);
    }

    /**
     * Create a request context for this database.
     *
     * @param workspace Optional workspace name
     * @param branchOverride Optional branch name override
     * @return RequestContext
     */
    RequestContext getContext(String workspace, String branchOverride) {
        String effectiveBranch = branchOverride != null ? branchOverride : this.branch;
        return RequestContext.builder()
                .tenantId(client.getTenantId())
                .repository(name)
                .workspace(workspace)
                .branch(effectiveBranch)
                .revision(this.revision)
                .build();
    }

    public String getName() {
        return name;
    }

    RaisinClient getClient() {
        return client;
    }
}
