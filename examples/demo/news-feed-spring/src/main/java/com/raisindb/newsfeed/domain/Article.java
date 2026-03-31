package com.raisindb.newsfeed.domain;

import java.time.OffsetDateTime;

/**
 * Article entity representing a news:Article node.
 */
public class Article {

    private String id;
    private String path;
    private String name;
    private String nodeType;
    private ArticleProperties properties;
    private OffsetDateTime createdAt;
    private OffsetDateTime updatedAt;

    public Article() {
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

    public ArticleProperties getProperties() {
        return properties;
    }

    public void setProperties(ArticleProperties properties) {
        this.properties = properties;
    }

    public OffsetDateTime getCreatedAt() {
        return createdAt;
    }

    public void setCreatedAt(OffsetDateTime createdAt) {
        this.createdAt = createdAt;
    }

    public OffsetDateTime getUpdatedAt() {
        return updatedAt;
    }

    public void setUpdatedAt(OffsetDateTime updatedAt) {
        this.updatedAt = updatedAt;
    }

    /**
     * Get the URL path for this article (removes base path).
     */
    public String getUrlPath() {
        if (path == null) {
            return "/";
        }
        if (path.startsWith("/superbigshit")) {
            return path.substring("/superbigshit".length());
        }
        return path;
    }

    /**
     * Extract category from path.
     * e.g., /superbigshit/articles/tech/my-article -> tech
     */
    public String getCategorySlug() {
        if (path == null) {
            return "";
        }
        String[] parts = path.split("/");
        // Path format: /superbigshit/articles/{category}/{slug}
        for (int i = 0; i < parts.length; i++) {
            if ("articles".equals(parts[i]) && i + 1 < parts.length) {
                return parts[i + 1];
            }
        }
        return "";
    }

    /**
     * Extract article slug from path.
     */
    public String getSlug() {
        if (path == null) {
            return "";
        }
        String[] parts = path.split("/");
        return parts.length > 0 ? parts[parts.length - 1] : "";
    }
}
