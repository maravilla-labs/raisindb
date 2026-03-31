package com.raisindb.newsfeed.dto;

import jakarta.validation.constraints.NotBlank;

/**
 * DTO for creating a new tag.
 */
public class TagCreateDto {

    @NotBlank(message = "Label is required")
    private String label;

    @NotBlank(message = "Name is required")
    private String name;

    @NotBlank(message = "Parent path is required")
    private String parentPath;

    private String icon;
    private String color;

    public TagCreateDto() {
    }

    public String getLabel() {
        return label;
    }

    public void setLabel(String label) {
        this.label = label;
    }

    public String getName() {
        return name;
    }

    public void setName(String name) {
        this.name = name;
    }

    public String getParentPath() {
        return parentPath;
    }

    public void setParentPath(String parentPath) {
        this.parentPath = parentPath;
    }

    public String getIcon() {
        return icon;
    }

    public void setIcon(String icon) {
        this.icon = icon;
    }

    public String getColor() {
        return color;
    }

    public void setColor(String color) {
        this.color = color;
    }
}
