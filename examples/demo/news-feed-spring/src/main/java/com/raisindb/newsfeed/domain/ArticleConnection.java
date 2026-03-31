package com.raisindb.newsfeed.domain;

/**
 * Outgoing connection from an article to another article.
 */
public class ArticleConnection {

    private String targetPath;
    private String targetId;
    private String targetTitle;
    private String relationType;
    private int weight; // 0-100 for UI, stored as 0-1 in DB
    private String editorialNote;

    public ArticleConnection() {
    }

    public String getTargetPath() {
        return targetPath;
    }

    public void setTargetPath(String targetPath) {
        this.targetPath = targetPath;
    }

    public String getTargetId() {
        return targetId;
    }

    public void setTargetId(String targetId) {
        this.targetId = targetId;
    }

    public String getTargetTitle() {
        return targetTitle;
    }

    public void setTargetTitle(String targetTitle) {
        this.targetTitle = targetTitle;
    }

    public String getRelationType() {
        return relationType;
    }

    public void setRelationType(String relationType) {
        this.relationType = relationType;
    }

    public int getWeight() {
        return weight;
    }

    public void setWeight(int weight) {
        this.weight = weight;
    }

    public String getEditorialNote() {
        return editorialNote;
    }

    public void setEditorialNote(String editorialNote) {
        this.editorialNote = editorialNote;
    }

    /**
     * Get weight as a double for database storage (0.0 - 1.0).
     */
    public double getWeightAsDouble() {
        return weight / 100.0;
    }
}
