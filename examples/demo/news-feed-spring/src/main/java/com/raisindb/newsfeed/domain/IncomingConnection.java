package com.raisindb.newsfeed.domain;

/**
 * Incoming connection to an article from another article.
 * Read-only for display purposes.
 */
public class IncomingConnection {

    private String sourcePath;
    private String sourceId;
    private String sourceTitle;
    private String relationType;
    private int weight; // 0-100

    public IncomingConnection() {
    }

    public String getSourcePath() {
        return sourcePath;
    }

    public void setSourcePath(String sourcePath) {
        this.sourcePath = sourcePath;
    }

    public String getSourceId() {
        return sourceId;
    }

    public void setSourceId(String sourceId) {
        this.sourceId = sourceId;
    }

    public String getSourceTitle() {
        return sourceTitle;
    }

    public void setSourceTitle(String sourceTitle) {
        this.sourceTitle = sourceTitle;
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
}
