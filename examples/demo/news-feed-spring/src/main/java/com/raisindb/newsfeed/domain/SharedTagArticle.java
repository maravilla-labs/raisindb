package com.raisindb.newsfeed.domain;

/**
 * Result from 2-hop GRAPH_TABLE query finding articles that share tags.
 */
public class SharedTagArticle {

    private String articleId;
    private String articlePath;
    private String articleTitle;
    private String sharedTag;
    private String tagPath;

    public SharedTagArticle() {
    }

    public String getArticleId() {
        return articleId;
    }

    public void setArticleId(String articleId) {
        this.articleId = articleId;
    }

    public String getArticlePath() {
        return articlePath;
    }

    public void setArticlePath(String articlePath) {
        this.articlePath = articlePath;
    }

    public String getArticleTitle() {
        return articleTitle;
    }

    public void setArticleTitle(String articleTitle) {
        this.articleTitle = articleTitle;
    }

    public String getSharedTag() {
        return sharedTag;
    }

    public void setSharedTag(String sharedTag) {
        this.sharedTag = sharedTag;
    }

    public String getTagPath() {
        return tagPath;
    }

    public void setTagPath(String tagPath) {
        this.tagPath = tagPath;
    }

    /**
     * Get URL path for the article.
     */
    public String getUrlPath() {
        if (articlePath == null) {
            return "/";
        }
        if (articlePath.startsWith("/superbigshit")) {
            return articlePath.substring("/superbigshit".length());
        }
        return articlePath;
    }
}
