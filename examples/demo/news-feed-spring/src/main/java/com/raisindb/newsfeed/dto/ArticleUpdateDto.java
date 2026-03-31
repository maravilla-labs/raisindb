package com.raisindb.newsfeed.dto;

import com.raisindb.newsfeed.domain.ArticleConnection;
import com.raisindb.newsfeed.domain.RaisinReference;
import jakarta.validation.constraints.NotBlank;

import java.time.OffsetDateTime;
import java.util.ArrayList;
import java.util.List;

/**
 * DTO for updating an existing article.
 */
public class ArticleUpdateDto {

    @NotBlank(message = "Title is required")
    private String title;

    @NotBlank(message = "Slug is required")
    private String slug;

    private String excerpt;
    private String body;

    @NotBlank(message = "Category is required")
    private String category;

    private List<String> keywords = new ArrayList<>();
    private List<String> tagPaths = new ArrayList<>();
    private boolean featured;
    private String status = "draft";
    private OffsetDateTime publishingDate;
    private String author;
    private String imageUrl;
    private List<ArticleConnection> connections = new ArrayList<>();

    public ArticleUpdateDto() {
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

    public List<String> getTagPaths() {
        return tagPaths;
    }

    public void setTagPaths(List<String> tagPaths) {
        this.tagPaths = tagPaths;
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

    /**
     * Convert tag paths to RaisinReferences.
     */
    public List<RaisinReference> getTagReferences(String workspace) {
        List<RaisinReference> refs = new ArrayList<>();
        if (tagPaths != null) {
            for (String path : tagPaths) {
                refs.add(new RaisinReference(path, workspace, path));
            }
        }
        return refs;
    }
}
