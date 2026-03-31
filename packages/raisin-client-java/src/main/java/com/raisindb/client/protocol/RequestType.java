package com.raisindb.client.protocol;

import com.fasterxml.jackson.annotation.JsonValue;

/**
 * Types of requests supported by the WebSocket protocol.
 */
public enum RequestType {
    // Authentication
    AUTHENTICATE("authenticate"),
    REFRESH_TOKEN("refresh_token"),

    // Node operations
    NODE_CREATE("node_create"),
    NODE_UPDATE("node_update"),
    NODE_DELETE("node_delete"),
    NODE_GET("node_get"),
    NODE_QUERY("node_query"),
    NODE_QUERY_BY_PATH("node_query_by_path"),
    NODE_QUERY_BY_PROPERTY("node_query_by_property"),

    // Tree operations
    NODE_LIST_CHILDREN("node_list_children"),
    NODE_GET_TREE("node_get_tree"),
    NODE_GET_TREE_FLAT("node_get_tree_flat"),

    // Node manipulation operations
    NODE_MOVE("node_move"),
    NODE_RENAME("node_rename"),
    NODE_COPY("node_copy"),
    NODE_COPY_TREE("node_copy_tree"),
    NODE_REORDER("node_reorder"),
    NODE_MOVE_CHILD_BEFORE("node_move_child_before"),
    NODE_MOVE_CHILD_AFTER("node_move_child_after"),

    // Property path operations
    PROPERTY_GET("property_get"),
    PROPERTY_UPDATE("property_update"),

    // Relationship operations
    RELATION_ADD("relation_add"),
    RELATION_REMOVE("relation_remove"),
    RELATIONS_GET("relations_get"),

    // SQL queries
    SQL_QUERY("sql_query"),

    // Workspace operations
    WORKSPACE_CREATE("workspace_create"),
    WORKSPACE_GET("workspace_get"),
    WORKSPACE_LIST("workspace_list"),
    WORKSPACE_DELETE("workspace_delete"),
    WORKSPACE_UPDATE("workspace_update"),

    // Branch operations
    BRANCH_CREATE("branch_create"),
    BRANCH_GET("branch_get"),
    BRANCH_LIST("branch_list"),
    BRANCH_DELETE("branch_delete"),
    BRANCH_GET_HEAD("branch_get_head"),
    BRANCH_UPDATE_HEAD("branch_update_head"),
    BRANCH_MERGE("branch_merge"),
    BRANCH_COMPARE("branch_compare"),

    // Tag operations
    TAG_CREATE("tag_create"),
    TAG_GET("tag_get"),
    TAG_LIST("tag_list"),
    TAG_DELETE("tag_delete"),

    // NodeType operations
    NODE_TYPE_CREATE("node_type_create"),
    NODE_TYPE_GET("node_type_get"),
    NODE_TYPE_LIST("node_type_list"),
    NODE_TYPE_UPDATE("node_type_update"),
    NODE_TYPE_DELETE("node_type_delete"),
    NODE_TYPE_PUBLISH("node_type_publish"),
    NODE_TYPE_UNPUBLISH("node_type_unpublish"),
    NODE_TYPE_VALIDATE("node_type_validate"),
    NODE_TYPE_GET_RESOLVED("node_type_get_resolved"),

    // Archetype operations
    ARCHETYPE_CREATE("archetype_create"),
    ARCHETYPE_GET("archetype_get"),
    ARCHETYPE_LIST("archetype_list"),
    ARCHETYPE_UPDATE("archetype_update"),
    ARCHETYPE_DELETE("archetype_delete"),
    ARCHETYPE_PUBLISH("archetype_publish"),
    ARCHETYPE_UNPUBLISH("archetype_unpublish"),

    // ElementType operations
    ELEMENT_TYPE_CREATE("element_type_create"),
    ELEMENT_TYPE_GET("element_type_get"),
    ELEMENT_TYPE_LIST("element_type_list"),
    ELEMENT_TYPE_UPDATE("element_type_update"),
    ELEMENT_TYPE_DELETE("element_type_delete"),
    ELEMENT_TYPE_PUBLISH("element_type_publish"),
    ELEMENT_TYPE_UNPUBLISH("element_type_unpublish"),

    // Event subscriptions
    SUBSCRIBE("subscribe"),
    UNSUBSCRIBE("unsubscribe"),

    // Repository management
    REPOSITORY_CREATE("repository_create"),
    REPOSITORY_GET("repository_get"),
    REPOSITORY_LIST("repository_list"),
    REPOSITORY_UPDATE("repository_update"),
    REPOSITORY_DELETE("repository_delete");

    private final String value;

    RequestType(String value) {
        this.value = value;
    }

    @JsonValue
    public String getValue() {
        return value;
    }

    @Override
    public String toString() {
        return value;
    }
}
