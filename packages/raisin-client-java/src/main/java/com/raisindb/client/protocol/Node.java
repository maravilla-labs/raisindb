package com.raisindb.client.protocol;

import com.fasterxml.jackson.annotation.JsonInclude;
import com.fasterxml.jackson.annotation.JsonProperty;
import java.time.Instant;
import java.util.*;

/**
 * A content node in RaisinDB's hierarchical structure.
 *
 * Nodes are the primary content entities, organized in a tree structure within workspaces.
 * Each node has a type (NodeType) that defines its schema, allowed children, and behavior.
 *
 * <p>Example usage:</p>
 * <pre>{@code
 * Node node = Node.builder()
 *     .nodeType("Page")
 *     .name("My Page")
 *     .path("/content/my-page")
 *     .property("title", "Welcome")
 *     .property("published", true)
 *     .build();
 * }</pre>
 */
@JsonInclude(JsonInclude.Include.NON_NULL)
public class Node {

    /** Unique identifier for this node */
    @JsonProperty("id")
    private String id;

    /** Display name of the node (used in URLs and hierarchical paths) */
    @JsonProperty("name")
    private String name;

    /** Full path to this node in the tree (e.g., "/content/my-page") */
    @JsonProperty("path")
    private String path;

    /** The NodeType name that defines this node's schema */
    @JsonProperty("node_type")
    private String nodeType;

    /** Optional archetype for specialized rendering */
    @JsonProperty("archetype")
    private String archetype;

    /** Key-value map of properties, validated against the NodeType schema */
    @JsonProperty("properties")
    private Map<String, Object> properties;

    /** Ordered list of child node IDs */
    @JsonProperty("children")
    private List<String> children;

    /**
     * Fractional index for ordering among siblings.
     * Base62 string that's lexicographically sortable (e.g., "a", "b", "b5", "c")
     */
    @JsonProperty("order_key")
    private String orderKey;

    /** Whether this node has children (computed field) */
    @JsonProperty("has_children")
    private Boolean hasChildren;

    /**
     * Name of the parent node (not the full path, just the name).
     * Example: If path is "/content/docs/page1", parent should be "docs"
     */
    @JsonProperty("parent")
    private String parent;

    /** Version number for this node (incremented on updates) */
    @JsonProperty("version")
    private Integer version = 1;

    /** Timestamp when this node was created */
    @JsonProperty("created_at")
    private Instant createdAt;

    /** Timestamp when this node was last updated */
    @JsonProperty("updated_at")
    private Instant updatedAt;

    /** Timestamp when this node was published (null if unpublished) */
    @JsonProperty("published_at")
    private Instant publishedAt;

    /** User ID who published this node */
    @JsonProperty("published_by")
    private String publishedBy;

    /** User ID who last updated this node */
    @JsonProperty("updated_by")
    private String updatedBy;

    /** User ID who created this node */
    @JsonProperty("created_by")
    private String createdBy;

    /** Translations for multi-language support */
    @JsonProperty("translations")
    private Map<String, Object> translations;

    /** Tenant ID for multi-tenant deployments */
    @JsonProperty("tenant_id")
    private String tenantId;

    /** Workspace this node belongs to */
    @JsonProperty("workspace")
    private String workspace;

    /** Owner user ID (for access control) */
    @JsonProperty("owner_id")
    private String ownerId;

    /** Relations to other nodes */
    @JsonProperty("relations")
    private List<RelationRef> relations;

    // Constructors
    public Node() {
        this.properties = new HashMap<>();
        this.children = new ArrayList<>();
        this.relations = new ArrayList<>();
    }

    // Builder pattern
    public static Builder builder() {
        return new Builder();
    }

    public static class Builder {
        private final Node node = new Node();

        public Builder id(String id) {
            node.id = id;
            return this;
        }

        public Builder name(String name) {
            node.name = name;
            return this;
        }

        public Builder path(String path) {
            node.path = path;
            return this;
        }

        public Builder nodeType(String nodeType) {
            node.nodeType = nodeType;
            return this;
        }

        public Builder archetype(String archetype) {
            node.archetype = archetype;
            return this;
        }

        public Builder property(String key, Object value) {
            if (node.properties == null) {
                node.properties = new HashMap<>();
            }
            node.properties.put(key, value);
            return this;
        }

        public Builder properties(Map<String, Object> properties) {
            node.properties = properties;
            return this;
        }

        public Builder child(String childId) {
            if (node.children == null) {
                node.children = new ArrayList<>();
            }
            node.children.add(childId);
            return this;
        }

        public Builder children(List<String> children) {
            node.children = children;
            return this;
        }

        public Builder orderKey(String orderKey) {
            node.orderKey = orderKey;
            return this;
        }

        public Builder parent(String parent) {
            node.parent = parent;
            return this;
        }

        public Builder version(Integer version) {
            node.version = version;
            return this;
        }

        public Builder workspace(String workspace) {
            node.workspace = workspace;
            return this;
        }

        public Builder tenantId(String tenantId) {
            node.tenantId = tenantId;
            return this;
        }

        public Builder relation(RelationRef relation) {
            if (node.relations == null) {
                node.relations = new ArrayList<>();
            }
            node.relations.add(relation);
            return this;
        }

        public Builder relations(List<RelationRef> relations) {
            node.relations = relations;
            return this;
        }

        public Node build() {
            // Ensure collections are initialized
            if (node.properties == null) node.properties = new HashMap<>();
            if (node.children == null) node.children = new ArrayList<>();
            if (node.relations == null) node.relations = new ArrayList<>();
            return node;
        }
    }

    // Helper methods

    /**
     * Get a property value by name.
     *
     * @param key the property name
     * @return the property value, or null if not found
     */
    public Object getProperty(String key) {
        return properties != null ? properties.get(key) : null;
    }

    /**
     * Get a property value as a specific type.
     *
     * @param key the property name
     * @param type the expected type class
     * @param <T> the expected type
     * @return the property value cast to the specified type, or null if not found or type mismatch
     */
    @SuppressWarnings("unchecked")
    public <T> T getProperty(String key, Class<T> type) {
        Object value = getProperty(key);
        if (value != null && type.isInstance(value)) {
            return (T) value;
        }
        return null;
    }

    /**
     * Set a property value.
     *
     * @param key the property name
     * @param value the property value
     * @return this node for chaining
     */
    public Node setProperty(String key, Object value) {
        if (this.properties == null) {
            this.properties = new HashMap<>();
        }
        this.properties.put(key, value);
        return this;
    }

    /**
     * Check if this node has any children.
     *
     * @return true if the node has children
     */
    public boolean hasChildren() {
        return hasChildren != null ? hasChildren : (children != null && !children.isEmpty());
    }

    /**
     * Get the parent path derived from this node's path.
     *
     * @return the full parent path, or null if this is a root node
     */
    public String getParentPath() {
        if (path == null || path.isEmpty() || path.equals("/")) {
            return null;
        }
        int lastSlash = path.lastIndexOf('/');
        if (lastSlash <= 0) {
            return "/";
        }
        return path.substring(0, lastSlash);
    }

    /**
     * Check if this node is published.
     *
     * @return true if the node has been published
     */
    public boolean isPublished() {
        return publishedAt != null;
    }

    /**
     * Add a relation to another node.
     *
     * @param target target node ID
     * @param workspace target workspace
     * @param relationType semantic relationship type
     * @return this node for chaining
     */
    public Node addRelation(String target, String workspace, String relationType) {
        return addRelation(target, workspace, null, relationType, null);
    }

    /**
     * Add a relation to another node with all parameters.
     *
     * @param target target node ID
     * @param workspace target workspace
     * @param targetNodeType target node type
     * @param relationType semantic relationship type
     * @param weight optional weight for graph algorithms
     * @return this node for chaining
     */
    public Node addRelation(String target, String workspace, String targetNodeType,
                           String relationType, Float weight) {
        if (this.relations == null) {
            this.relations = new ArrayList<>();
        }
        this.relations.add(new RelationRef(target, workspace, targetNodeType, relationType, weight));
        return this;
    }

    // Standard getters and setters
    public String getId() { return id; }
    public void setId(String id) { this.id = id; }

    public String getName() { return name; }
    public void setName(String name) { this.name = name; }

    public String getPath() { return path; }
    public void setPath(String path) { this.path = path; }

    public String getNodeType() { return nodeType; }
    public void setNodeType(String nodeType) { this.nodeType = nodeType; }

    public String getArchetype() { return archetype; }
    public void setArchetype(String archetype) { this.archetype = archetype; }

    public Map<String, Object> getProperties() { return properties; }
    public void setProperties(Map<String, Object> properties) { this.properties = properties; }

    public List<String> getChildren() { return children; }
    public void setChildren(List<String> children) { this.children = children; }

    public String getOrderKey() { return orderKey; }
    public void setOrderKey(String orderKey) { this.orderKey = orderKey; }

    public Boolean getHasChildren() { return hasChildren; }
    public void setHasChildren(Boolean hasChildren) { this.hasChildren = hasChildren; }

    public String getParent() { return parent; }
    public void setParent(String parent) { this.parent = parent; }

    public Integer getVersion() { return version; }
    public void setVersion(Integer version) { this.version = version; }

    public Instant getCreatedAt() { return createdAt; }
    public void setCreatedAt(Instant createdAt) { this.createdAt = createdAt; }

    public Instant getUpdatedAt() { return updatedAt; }
    public void setUpdatedAt(Instant updatedAt) { this.updatedAt = updatedAt; }

    public Instant getPublishedAt() { return publishedAt; }
    public void setPublishedAt(Instant publishedAt) { this.publishedAt = publishedAt; }

    public String getPublishedBy() { return publishedBy; }
    public void setPublishedBy(String publishedBy) { this.publishedBy = publishedBy; }

    public String getUpdatedBy() { return updatedBy; }
    public void setUpdatedBy(String updatedBy) { this.updatedBy = updatedBy; }

    public String getCreatedBy() { return createdBy; }
    public void setCreatedBy(String createdBy) { this.createdBy = createdBy; }

    public Map<String, Object> getTranslations() { return translations; }
    public void setTranslations(Map<String, Object> translations) { this.translations = translations; }

    public String getTenantId() { return tenantId; }
    public void setTenantId(String tenantId) { this.tenantId = tenantId; }

    public String getWorkspace() { return workspace; }
    public void setWorkspace(String workspace) { this.workspace = workspace; }

    public String getOwnerId() { return ownerId; }
    public void setOwnerId(String ownerId) { this.ownerId = ownerId; }

    public List<RelationRef> getRelations() { return relations; }
    public void setRelations(List<RelationRef> relations) { this.relations = relations; }

    @Override
    public String toString() {
        return String.format("Node{id='%s', nodeType='%s', path='%s', name='%s'}",
                           id, nodeType, path, name);
    }
}
