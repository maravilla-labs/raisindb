package com.raisindb.newsfeed.domain;

/**
 * Category entity representing a raisin:Folder used for article categories.
 */
public class Category {

    private String id;
    private String path;
    private String name;
    private CategoryProperties properties;

    public Category() {
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

    public CategoryProperties getProperties() {
        return properties;
    }

    public void setProperties(CategoryProperties properties) {
        this.properties = properties;
    }

    /**
     * Get the URL slug for this category.
     */
    public String getSlug() {
        if (path == null || path.isEmpty()) {
            return "";
        }
        String[] parts = path.split("/");
        return parts.length > 0 ? parts[parts.length - 1] : "";
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

    /**
     * Get color (from properties or default).
     */
    public String getColor() {
        if (properties != null && properties.getColor() != null) {
            return properties.getColor();
        }
        return "#6B7280";
    }
}
