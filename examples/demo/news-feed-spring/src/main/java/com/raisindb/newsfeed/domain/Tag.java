package com.raisindb.newsfeed.domain;

import java.util.ArrayList;
import java.util.List;

/**
 * Tag entity representing a news:Tag node.
 */
public class Tag {

    private String id;
    private String path;
    private String name;
    private String nodeType;
    private TagProperties properties;
    private List<Tag> children = new ArrayList<>();

    public Tag() {
    }

    public String getId() {
        return id;
    }

    public void setId(String id) {
        this.id = id;
    }

    public String getPath() {
        return path;
    }

    public void setPath(String path) {
        this.path = path;
    }

    public String getName() {
        return name;
    }

    public void setName(String name) {
        this.name = name;
    }

    public String getNodeType() {
        return nodeType;
    }

    public void setNodeType(String nodeType) {
        this.nodeType = nodeType;
    }

    public TagProperties getProperties() {
        return properties;
    }

    public void setProperties(TagProperties properties) {
        this.properties = properties;
    }

    public List<Tag> getChildren() {
        return children;
    }

    public void setChildren(List<Tag> children) {
        this.children = children;
    }

    /**
     * Get parent path.
     * e.g., /superbigshit/tags/tech-stack/rust -> /superbigshit/tags/tech-stack
     */
    public String getParentPath() {
        if (path == null || path.isEmpty()) {
            return "";
        }
        int lastSlash = path.lastIndexOf('/');
        return lastSlash > 0 ? path.substring(0, lastSlash) : "";
    }

    /**
     * Get display label (from properties or name fallback).
     */
    public String getDisplayLabel() {
        if (properties != null && properties.getLabel() != null) {
            return properties.getLabel();
        }
        return name;
    }
}
