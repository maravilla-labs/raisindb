package com.raisindb.client.operations;

import com.raisindb.client.protocol.RequestContext;
import com.raisindb.client.protocol.RequestType;

import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.concurrent.CompletableFuture;

/**
 * Represents a workspace within a database.
 */
public class Workspace {

    private final Database database;
    private final String name;
    private final String branch;
    private final String revision;
    private NodeOperations nodeOperations;
    private EventSubscriptions eventSubscriptions;

    public Workspace(Database database, String name) {
        this(database, name, "main", null);
    }

    public Workspace(Database database, String name, String branch) {
        this(database, name, branch, null);
    }

    private Workspace(Database database, String name, String branch, String revision) {
        this.database = database;
        this.name = name;
        this.branch = branch;
        this.revision = revision;
    }

    /**
     * Get the node operations interface.
     *
     * @return NodeOperations instance
     */
    public NodeOperations nodes() {
        if (nodeOperations == null) {
            nodeOperations = new NodeOperations(this);
        }
        return nodeOperations;
    }

    /**
     * Get the event subscriptions interface.
     *
     * @return EventSubscriptions instance
     */
    public EventSubscriptions events() {
        if (eventSubscriptions == null) {
            eventSubscriptions = new EventSubscriptions(this);
        }
        return eventSubscriptions;
    }

    /**
     * Get workspace information.
     *
     * @return CompletableFuture with workspace info
     */
    public CompletableFuture<Object> getInfo() {
        RequestContext context = getContext();
        Map<String, String> payload = new HashMap<>();
        payload.put("name", name);

        return database.getClient().getRequestTracker()
                .sendRequest(RequestType.WORKSPACE_GET, context, payload);
    }

    /**
     * Update workspace configuration.
     *
     * @param description           New description
     * @param allowedNodeTypes      List of allowed node types
     * @param allowedRootNodeTypes  List of allowed root node types
     * @return CompletableFuture with updated workspace info
     */
    public CompletableFuture<Object> update(
            String description,
            List<String> allowedNodeTypes,
            List<String> allowedRootNodeTypes
    ) {
        RequestContext context = getContext();
        Map<String, Object> payload = new HashMap<>();
        payload.put("name", name);

        if (description != null) {
            payload.put("description", description);
        }
        if (allowedNodeTypes != null) {
            payload.put("allowed_node_types", allowedNodeTypes);
        }
        if (allowedRootNodeTypes != null) {
            payload.put("allowed_root_node_types", allowedRootNodeTypes);
        }

        return database.getClient().getRequestTracker()
                .sendRequest(RequestType.WORKSPACE_UPDATE, context, payload);
    }

    /**
     * Create a new workspace instance on a different branch.
     *
     * @param branch Branch name
     * @return New Workspace instance
     */
    public Workspace onBranch(String branch) {
        return new Workspace(database, name, branch, this.revision);
    }

    /**
     * Create a new workspace instance scoped to a specific revision/commit.
     *
     * @param revision Revision/commit ID
     * @return New Workspace instance with revision context
     */
    public Workspace atRevision(String revision) {
        return new Workspace(database, name, this.branch, revision);
    }

    /**
     * Get request context for this workspace.
     */
    RequestContext getContext() {
        RequestContext context = database.getContext(name, branch);
        // Override with workspace-specific revision if set
        if (this.revision != null) {
            context.setRevision(this.revision);
        }
        return context;
    }

    public String getName() {
        return name;
    }

    public String getBranch() {
        return branch;
    }

    Database getDatabase() {
        return database;
    }
}
