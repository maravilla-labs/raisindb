package com.raisindb.newsfeed.domain;

import com.fasterxml.jackson.annotation.JsonProperty;

import java.time.OffsetDateTime;
import java.util.List;

/**
 * Properties for news:Article node type.
 */
public class ArticleProperties {

    private String title;
    private String slug;
    private String excerpt;
    private String body;
    private String category;
    private List<String> keywords;
    private List<RaisinReference> tags;
    private boolean featured;
    private String status = "draft";

    @JsonProperty("publishing_date")
    private OffsetDateTime publishingDate;

    private int views;
    private String author;
    private String imageUrl;
    private List<ArticleConnection> connections;

    public ArticleProperties() {
    }

    public String getTitle() {
        return title;
    }

    public void setTitle(String title) {
        this.title = title;
    }

    public String getSlug() {
        return slug;
    }

    public void setSlug(String slug) {
        this.slug = slug;
    }

    public String getExcerpt() {
        return excerpt;
    }

    public void setExcerpt(String excerpt) {
        this.excerpt = excerpt;
    }

    public String getBody() {
        return body;
    }

    public void setBody(String body) {
        this.body = body;
    }

    public String getCategory() {
        return category;
    }

    public void setCategory(String category) {
        this.category = category;
    }

    public List<String> getKeywords() {
        return keywords;
    }

    public void setKeywords(List<String> keywords) {
        this.keywords = keywords;
    }

    public List<RaisinReference> getTags() {
        return tags;
    }

    public void setTags(List<RaisinReference> tags) {
        this.tags = tags;
    }

    public boolean isFeatured() {
        return featured;
    }

    public void setFeatured(boolean featured) {
        this.featured = featured;
    }

    public String getStatus() {
        return status;
    }

    public void setStatus(String status) {
        this.status = status;
    }

    public OffsetDateTime getPublishingDate() {
        return publishingDate;
    }

    public void setPublishingDate(OffsetDateTime publishingDate) {
        this.publishingDate = publishingDate;
    }

    public int getViews() {
        return views;
    }

    public void setViews(int views) {
        this.views = views;
    }

    public String getAuthor() {
        return author;
    }

    public void setAuthor(String author) {
        this.author = author;
    }

    public String getImageUrl() {
        return imageUrl;
    }

    public void setImageUrl(String imageUrl) {
        this.imageUrl = imageUrl;
    }

    public List<ArticleConnection> getConnections() {
        return connections;
    }

    public void setConnections(List<ArticleConnection> connections) {
        this.connections = connections;
    }
}
