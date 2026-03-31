package com.raisindb.newsfeed.domain;

/**
 * Properties for category folders.
 */
public class CategoryProperties {

    private String label;
    private String color;
    private int order;

    public CategoryProperties() {
    }

    public CategoryProperties(String label, String color, int order) {
        this.label = label;
        this.color = color;
        this.order = order;
    }

    public String getLabel() {
        return label;
    }

    public void setLabel(String label) {
        this.label = label;
    }

    public String getColor() {
        return color;
    }

    public void setColor(String color) {
        this.color = color;
    }

    public int getOrder() {
        return order;
    }

    public void setOrder(int order) {
        this.order = order;
    }
}
