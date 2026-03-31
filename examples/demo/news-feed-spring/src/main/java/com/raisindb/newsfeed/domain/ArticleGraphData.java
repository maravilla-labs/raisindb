package com.raisindb.newsfeed.domain;

import java.util.ArrayList;
import java.util.List;

/**
 * Graph data for article display page.
 * Contains all relationship information queried via GRAPH_TABLE and NEIGHBORS.
 */
public class ArticleGraphData {

    private List<SmartRelatedArticle> smartRelated = new ArrayList<>();
    private Timeline timeline = new Timeline();
    private CorrectionInfo correction;
    private CorrectionInfo correctsArticle;
    private List<Article> opposingViews = new ArrayList<>();
    private List<Article> evidence = new ArrayList<>();
    private List<SharedTagArticle> sharedTagArticles = new ArrayList<>();
    private List<ArticleTag> tags = new ArrayList<>();

    public ArticleGraphData() {
    }

    public List<SmartRelatedArticle> getSmartRelated() {
        return smartRelated;
    }

    public void setSmartRelated(List<SmartRelatedArticle> smartRelated) {
        this.smartRelated = smartRelated;
    }

    public Timeline getTimeline() {
        return timeline;
    }

    public void setTimeline(Timeline timeline) {
        this.timeline = timeline;
    }

    public CorrectionInfo getCorrection() {
        return correction;
    }

    public void setCorrection(CorrectionInfo correction) {
        this.correction = correction;
    }

    public CorrectionInfo getCorrectsArticle() {
        return correctsArticle;
    }

    public void setCorrectsArticle(CorrectionInfo correctsArticle) {
        this.correctsArticle = correctsArticle;
    }

    public List<Article> getOpposingViews() {
        return opposingViews;
    }

    public void setOpposingViews(List<Article> opposingViews) {
        this.opposingViews = opposingViews;
    }

    public List<Article> getEvidence() {
        return evidence;
    }

    public void setEvidence(List<Article> evidence) {
        this.evidence = evidence;
    }

    public List<SharedTagArticle> getSharedTagArticles() {
        return sharedTagArticles;
    }

    public void setSharedTagArticles(List<SharedTagArticle> sharedTagArticles) {
        this.sharedTagArticles = sharedTagArticles;
    }

    public List<ArticleTag> getTags() {
        return tags;
    }

    public void setTags(List<ArticleTag> tags) {
        this.tags = tags;
    }

    /**
     * Smart related article with weight and relation type.
     */
    public static class SmartRelatedArticle {
        private Article article;
        private int weight;
        private String relationType;

        public SmartRelatedArticle() {
        }

        public Article getArticle() {
            return article;
        }

        public void setArticle(Article article) {
            this.article = article;
        }

        public int getWeight() {
            return weight;
        }

        public void setWeight(int weight) {
            this.weight = weight;
        }

        public String getRelationType() {
            return relationType;
        }

        public void setRelationType(String relationType) {
            this.relationType = relationType;
        }
    }

    /**
     * Timeline with predecessors and successors.
     */
    public static class Timeline {
        private List<Article> predecessors = new ArrayList<>();
        private List<Article> successors = new ArrayList<>();

        public Timeline() {
        }

        public List<Article> getPredecessors() {
            return predecessors;
        }

        public void setPredecessors(List<Article> predecessors) {
            this.predecessors = predecessors;
        }

        public List<Article> getSuccessors() {
            return successors;
        }

        public void setSuccessors(List<Article> successors) {
            this.successors = successors;
        }
    }

    /**
     * Correction info for display.
     */
    public static class CorrectionInfo {
        private String title;
        private String path;
        private String publishedAt;

        public CorrectionInfo() {
        }

        public String getTitle() {
            return title;
        }

        public void setTitle(String title) {
            this.title = title;
        }

        public String getPath() {
            return path;
        }

        public void setPath(String path) {
            this.path = path;
        }

        public String getPublishedAt() {
            return publishedAt;
        }

        public void setPublishedAt(String publishedAt) {
            this.publishedAt = publishedAt;
        }

        public String getUrlPath() {
            if (path == null) {
                return "/";
            }
            if (path.startsWith("/superbigshit")) {
                return path.substring("/superbigshit".length());
            }
            return path;
        }
    }

    /**
     * Tag info from GRAPH_TABLE query.
     */
    public static class ArticleTag {
        private String path;
        private String label;
        private String icon;
        private String color;

        public ArticleTag() {
        }

        public String getPath() {
            return path;
        }

        public void setPath(String path) {
            this.path = path;
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
}
