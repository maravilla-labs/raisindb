package com.raisindb.client.operations;

import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.datatype.jsr310.JavaTimeModule;
import com.raisindb.client.protocol.Node;
import com.raisindb.client.protocol.RequestContext;
import com.raisindb.client.protocol.RequestType;

import java.util.*;
import java.util.concurrent.CompletableFuture;
import java.util.stream.Collectors;

/**
 * Node CRUD operations within a workspace.
 */
public class NodeOperations {

    private final Workspace workspace;
    private final ObjectMapper objectMapper;

    public NodeOperations(Workspace workspace) {
        this.workspace = workspace;
        this.objectMapper = new ObjectMapper();
        this.objectMapper.registerModule(new JavaTimeModule());
    }

    /**
     * Create a new node.
     *
     * @param nodeType   Type of node to create
     * @param path       Path for the node
     * @param properties Node properties
     * @param content    Node content
     * @return CompletableFuture with created Node
     */
    public CompletableFuture<Node> create(
            String nodeType,
            String path,
            Map<String, Object> properties,
            Object content
    ) {
        RequestContext context = workspace.getContext();

        Map<String, Object> payload = new HashMap<>();
        payload.put("node_type", nodeType);
        payload.put("path", path);
        payload.put("properties", properties != null ? properties : Collections.emptyMap());
        if (content != null) {
            payload.put("content", content);
        }

        return workspace.getDatabase().getClient().getRequestTracker()
                .sendRequest(RequestType.NODE_CREATE, context, payload)
                .thenApply(result -> objectMapper.convertValue(result, Node.class));
    }

    /**
     * Get a node by ID.
     *
     * @param nodeId Node ID
     * @return CompletableFuture with Node
     */
    public CompletableFuture<Node> get(String nodeId) {
        RequestContext context = workspace.getContext();

        Map<String, String> payload = new HashMap<>();
        payload.put("node_id", nodeId);

        return workspace.getDatabase().getClient().getRequestTracker()
                .sendRequest(RequestType.NODE_GET, context, payload)
                .thenApply(result -> objectMapper.convertValue(result, Node.class));
    }

    /**
     * Update an existing node.
     *
     * @param nodeId     Node ID
     * @param properties Properties to update
     * @param content    Content to update
     * @return CompletableFuture with updated Node
     */
    public CompletableFuture<Node> update(
            String nodeId,
            Map<String, Object> properties,
            Object content
    ) {
        RequestContext context = workspace.getContext();

        Map<String, Object> payload = new HashMap<>();
        payload.put("node_id", nodeId);
        payload.put("properties", properties != null ? properties : Collections.emptyMap());
        if (content != null) {
            payload.put("content", content);
        }

        return workspace.getDatabase().getClient().getRequestTracker()
                .sendRequest(RequestType.NODE_UPDATE, context, payload)
                .thenApply(result -> objectMapper.convertValue(result, Node.class));
    }

    /**
     * Delete a node.
     *
     * @param nodeId Node ID
     * @return CompletableFuture that completes when deletion succeeds
     */
    public CompletableFuture<Void> delete(String nodeId) {
        RequestContext context = workspace.getContext();

        Map<String, String> payload = new HashMap<>();
        payload.put("node_id", nodeId);

        return workspace.getDatabase().getClient().getRequestTracker()
                .sendRequest(RequestType.NODE_DELETE, context, payload)
                .thenApply(result -> null);
    }

    /**
     * Query nodes by path pattern.
     *
     * @param path Path pattern (supports wildcards)
     * @return CompletableFuture with list of Nodes
     */
    public CompletableFuture<List<Node>> queryByPath(String path) {
        RequestContext context = workspace.getContext();

        Map<String, String> payload = new HashMap<>();
        payload.put("path", path);

        return workspace.getDatabase().getClient().getRequestTracker()
                .sendRequest(RequestType.NODE_QUERY_BY_PATH, context, payload)
                .thenApply(result -> {
                    if (result instanceof List) {
                        return ((List<?>) result).stream()
                                .map(item -> objectMapper.convertValue(item, Node.class))
                                .collect(Collectors.toList());
                    }
                    return Collections.emptyList();
                });
    }

    /**
     * Query nodes by a specific property.
     *
     * @param propertyName  Property name
     * @param propertyValue Property value
     * @return CompletableFuture with list of Nodes
     */
    public CompletableFuture<List<Node>> queryByProperty(String propertyName, Object propertyValue) {
        RequestContext context = workspace.getContext();

        Map<String, Object> payload = new HashMap<>();
        payload.put("property_name", propertyName);
        payload.put("property_value", propertyValue);

        return workspace.getDatabase().getClient().getRequestTracker()
                .sendRequest(RequestType.NODE_QUERY_BY_PROPERTY, context, payload)
                .thenApply(result -> {
                    if (result instanceof List) {
                        return ((List<?>) result).stream()
                                .map(item -> objectMapper.convertValue(item, Node.class))
                                .collect(Collectors.toList());
                    }
                    return Collections.emptyList();
                });
    }

    // ========================================================================
    // Tree Operations
    // ========================================================================

    /**
     * List children of a parent node.
     *
     * @param parentPath Path of the parent node (e.g., "/blog" or "/" for root)
     * @return CompletableFuture with list of child Nodes in order
     */
    public CompletableFuture<List<Node>> listChildren(String parentPath) {
        RequestContext context = workspace.getContext();

        Map<String, String> payload = new HashMap<>();
        payload.put("parent_path", parentPath);

        return workspace.getDatabase().getClient().getRequestTracker()
                .sendRequest(RequestType.NODE_LIST_CHILDREN, context, payload)
                .thenApply(result -> {
                    if (result instanceof List) {
                        return ((List<?>) result).stream()
                                .map(item -> objectMapper.convertValue(item, Node.class))
                                .collect(Collectors.toList());
                    }
                    return Collections.emptyList();
                });
    }

    /**
     * Get hierarchical tree structure starting from a root node.
     *
     * @param rootPath Path of the root node
     * @param maxDepth Maximum depth to traverse (null for unlimited)
     * @return CompletableFuture with tree structure
     */
    public CompletableFuture<Node> getTree(String rootPath, Integer maxDepth) {
        RequestContext context = workspace.getContext();

        Map<String, Object> payload = new HashMap<>();
        payload.put("root_path", rootPath);
        if (maxDepth != null) {
            payload.put("max_depth", maxDepth);
        }

        return workspace.getDatabase().getClient().getRequestTracker()
                .sendRequest(RequestType.NODE_GET_TREE, context, payload)
                .thenApply(result -> objectMapper.convertValue(result, Node.class));
    }

    /**
     * Get flat list of all nodes in a tree.
     *
     * @param rootPath Path of the root node
     * @param maxDepth Maximum depth to traverse (null for unlimited)
     * @return CompletableFuture with flat list of Nodes
     */
    public CompletableFuture<List<Node>> getTreeFlat(String rootPath, Integer maxDepth) {
        RequestContext context = workspace.getContext();

        Map<String, Object> payload = new HashMap<>();
        payload.put("root_path", rootPath);
        if (maxDepth != null) {
            payload.put("max_depth", maxDepth);
        }

        return workspace.getDatabase().getClient().getRequestTracker()
                .sendRequest(RequestType.NODE_GET_TREE_FLAT, context, payload)
                .thenApply(result -> {
                    if (result instanceof List) {
                        return ((List<?>) result).stream()
                                .map(item -> objectMapper.convertValue(item, Node.class))
                                .collect(Collectors.toList());
                    }
                    return Collections.emptyList();
                });
    }

    // ========================================================================
    // Node Manipulation Operations
    // ========================================================================

    /**
     * Move a node to a new parent.
     *
     * @param fromPath Source node path
     * @param toParentPath Destination parent path
     * @return CompletableFuture with moved Node
     */
    public CompletableFuture<Node> move(String fromPath, String toParentPath) {
        RequestContext context = workspace.getContext();

        Map<String, String> payload = new HashMap<>();
        payload.put("from_path", fromPath);
        payload.put("to_parent_path", toParentPath);

        return workspace.getDatabase().getClient().getRequestTracker()
                .sendRequest(RequestType.NODE_MOVE, context, payload)
                .thenApply(result -> objectMapper.convertValue(result, Node.class));
    }

    /**
     * Rename a node.
     *
     * @param nodePath Node path
     * @param newName New name for the node
     * @return CompletableFuture with renamed Node
     */
    public CompletableFuture<Node> rename(String nodePath, String newName) {
        RequestContext context = workspace.getContext();

        Map<String, String> payload = new HashMap<>();
        payload.put("node_path", nodePath);
        payload.put("new_name", newName);

        return workspace.getDatabase().getClient().getRequestTracker()
                .sendRequest(RequestType.NODE_RENAME, context, payload)
                .thenApply(result -> objectMapper.convertValue(result, Node.class));
    }

    /**
     * Copy a node to a new parent (shallow copy).
     *
     * @param fromPath Source node path
     * @param toParentPath Destination parent path
     * @param newName New name for the copied node (null to keep original name)
     * @return CompletableFuture with copied Node
     */
    public CompletableFuture<Node> copy(String fromPath, String toParentPath, String newName) {
        RequestContext context = workspace.getContext();

        Map<String, Object> payload = new HashMap<>();
        payload.put("from_path", fromPath);
        payload.put("to_parent_path", toParentPath);
        if (newName != null) {
            payload.put("new_name", newName);
        }
        payload.put("deep", false);

        return workspace.getDatabase().getClient().getRequestTracker()
                .sendRequest(RequestType.NODE_COPY, context, payload)
                .thenApply(result -> objectMapper.convertValue(result, Node.class));
    }

    /**
     * Copy a node tree to a new parent (deep copy with all children).
     *
     * @param fromPath Source node path
     * @param toParentPath Destination parent path
     * @param newName New name for the copied node (null to keep original name)
     * @return CompletableFuture with copied Node tree
     */
    public CompletableFuture<Node> copyTree(String fromPath, String toParentPath, String newName) {
        RequestContext context = workspace.getContext();

        Map<String, Object> payload = new HashMap<>();
        payload.put("from_path", fromPath);
        payload.put("to_parent_path", toParentPath);
        if (newName != null) {
            payload.put("new_name", newName);
        }
        payload.put("deep", true);

        return workspace.getDatabase().getClient().getRequestTracker()
                .sendRequest(RequestType.NODE_COPY_TREE, context, payload)
                .thenApply(result -> objectMapper.convertValue(result, Node.class));
    }

    /**
     * Reorder a node by setting a new order key.
     *
     * @param nodePath Node path
     * @param orderKey New order key (base62-encoded fractional index)
     * @return CompletableFuture with reordered Node
     */
    public CompletableFuture<Node> reorder(String nodePath, String orderKey) {
        RequestContext context = workspace.getContext();

        Map<String, String> payload = new HashMap<>();
        payload.put("node_path", nodePath);
        payload.put("order_key", orderKey);

        return workspace.getDatabase().getClient().getRequestTracker()
                .sendRequest(RequestType.NODE_REORDER, context, payload)
                .thenApply(result -> objectMapper.convertValue(result, Node.class));
    }

    /**
     * Move a child node before a reference sibling.
     *
     * @param parentPath Parent node path
     * @param childPath Child node path to move
     * @param referencePath Reference sibling path to position before
     * @return CompletableFuture with moved Node
     */
    public CompletableFuture<Node> moveChildBefore(String parentPath, String childPath, String referencePath) {
        RequestContext context = workspace.getContext();

        Map<String, String> payload = new HashMap<>();
        payload.put("parent_path", parentPath);
        payload.put("child_path", childPath);
        payload.put("reference_path", referencePath);

        return workspace.getDatabase().getClient().getRequestTracker()
                .sendRequest(RequestType.NODE_MOVE_CHILD_BEFORE, context, payload)
                .thenApply(result -> objectMapper.convertValue(result, Node.class));
    }

    /**
     * Move a child node after a reference sibling.
     *
     * @param parentPath Parent node path
     * @param childPath Child node path to move
     * @param referencePath Reference sibling path to position after
     * @return CompletableFuture with moved Node
     */
    public CompletableFuture<Node> moveChildAfter(String parentPath, String childPath, String referencePath) {
        RequestContext context = workspace.getContext();

        Map<String, String> payload = new HashMap<>();
        payload.put("parent_path", parentPath);
        payload.put("child_path", childPath);
        payload.put("reference_path", referencePath);

        return workspace.getDatabase().getClient().getRequestTracker()
                .sendRequest(RequestType.NODE_MOVE_CHILD_AFTER, context, payload)
                .thenApply(result -> objectMapper.convertValue(result, Node.class));
    }

    // ========================================================================
    // Relationship Operations
    // ========================================================================

    /**
     * Add a relationship between two nodes.
     *
     * @param nodePath Source node path
     * @param relationType Type of relationship
     * @param targetNodePath Target node path
     * @param weight Optional relationship weight
     * @return CompletableFuture with updated Node
     */
    public CompletableFuture<Node> addRelation(String nodePath, String relationType, String targetNodePath, Float weight) {
        RequestContext context = workspace.getContext();

        Map<String, Object> payload = new HashMap<>();
        payload.put("node_path", nodePath);
        payload.put("relation_type", relationType);
        payload.put("target_node_path", targetNodePath);
        if (weight != null) {
            payload.put("weight", weight);
        }

        return workspace.getDatabase().getClient().getRequestTracker()
                .sendRequest(RequestType.RELATION_ADD, context, payload)
                .thenApply(result -> objectMapper.convertValue(result, Node.class));
    }

    /**
     * Remove a relationship between two nodes.
     *
     * @param nodePath Source node path
     * @param relationType Type of relationship
     * @param targetNodePath Target node path
     * @return CompletableFuture with updated Node
     */
    public CompletableFuture<Node> removeRelation(String nodePath, String relationType, String targetNodePath) {
        RequestContext context = workspace.getContext();

        Map<String, String> payload = new HashMap<>();
        payload.put("node_path", nodePath);
        payload.put("relation_type", relationType);
        payload.put("target_node_path", targetNodePath);

        return workspace.getDatabase().getClient().getRequestTracker()
                .sendRequest(RequestType.RELATION_REMOVE, context, payload)
                .thenApply(result -> objectMapper.convertValue(result, Node.class));
    }

    /**
     * Get all relationships for a node.
     *
     * @param nodePath Node path
     * @return CompletableFuture with list of RelationRef objects
     */
    public CompletableFuture<List<RelationRef>> getRelationships(String nodePath) {
        RequestContext context = workspace.getContext();

        Map<String, String> payload = new HashMap<>();
        payload.put("node_path", nodePath);

        return workspace.getDatabase().getClient().getRequestTracker()
                .sendRequest(RequestType.RELATIONS_GET, context, payload)
                .thenApply(result -> {
                    if (result instanceof List) {
                        return ((List<?>) result).stream()
                                .map(item -> objectMapper.convertValue(item, RelationRef.class))
                                .collect(Collectors.toList());
                    }
                    return Collections.emptyList();
                });
    }
}
