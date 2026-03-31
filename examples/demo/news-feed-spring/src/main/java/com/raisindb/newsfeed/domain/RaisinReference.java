package com.raisindb.newsfeed.domain;

import com.fasterxml.jackson.annotation.JsonProperty;

/**
 * RaisinDB Reference type - points to another node.
 * References are stored in properties as JSON objects.
 */
public class RaisinReference {

    @JsonProperty("raisin:ref")
    private String ref;

    @JsonProperty("raisin:workspace")
    private String workspace;

    @JsonProperty("raisin:path")
    private String path;

    public RaisinReference() {
    }

    public RaisinReference(String ref, String workspace, String path) {
        this.ref = ref;
        this.workspace = workspace;
        this.path = path;
    }

    public String getRef() {
        return ref;
    }

    public void setRef(String ref) {
        this.ref = ref;
    }

    public String getWorkspace() {
        return workspace;
    }

    public void setWorkspace(String workspace) {
        this.workspace = workspace;
    }

    public String getPath() {
        return path;
    }

    public void setPath(String path) {
        this.path = path;
    }

    /**
     * Extract tag name from reference path.
     * e.g., /superbigshit/tags/tech-stack/rust -> rust
     */
    public String getTagName() {
        if (path == null || path.isEmpty()) {
            return "";
        }
        String[] parts = path.split("/");
        return parts.length > 0 ? parts[parts.length - 1] : "";
    }
}
