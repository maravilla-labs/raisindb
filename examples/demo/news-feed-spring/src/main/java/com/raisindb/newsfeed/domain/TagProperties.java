package com.raisindb.newsfeed.domain;

/**
 * Properties for news:Tag node type.
 */
public class TagProperties {

    private String label;
    private String icon;
    private String color;

    public TagProperties() {
    }

    public TagProperties(String label, String icon, String color) {
        this.label = label;
        this.icon = icon;
        this.color = color;
    }

    public String getLabel() {
        return label;
    }

    public void setLabel(String label) {
        this.label = label;
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
