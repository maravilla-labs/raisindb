package com.raisindb.newsfeed.service;

import com.raisindb.newsfeed.domain.*;
import com.raisindb.newsfeed.dto.ArticleCreateDto;
import com.raisindb.newsfeed.dto.ArticleUpdateDto;
import com.raisindb.newsfeed.repository.ArticleRepository;
import com.raisindb.newsfeed.repository.GraphRepository;
import com.raisindb.newsfeed.util.PathUtils;
import org.springframework.beans.factory.annotation.Value;
import org.springframework.stereotype.Service;
import org.springframework.transaction.annotation.Transactional;

import java.time.OffsetDateTime;
import java.util.*;
import java.util.stream.Collectors;

/**
 * Service for article operations.
 */
@Service
public class ArticleService {

    private final ArticleRepository articleRepository;
    private final GraphRepository graphRepository;
    private final String articlesPath;
    private final String workspace;

    public ArticleService(ArticleRepository articleRepository,
                          GraphRepository graphRepository,
                          @Value("${raisindb.articles-path}") String articlesPath,
                          @Value("${raisindb.workspace}") String workspace) {
        this.articleRepository = articleRepository;
        this.graphRepository = graphRepository;
        this.articlesPath = articlesPath;
        this.workspace = workspace;
    }

    public List<Article> getFeaturedArticles(String accessToken) {
        return articleRepository.findFeaturedArticles(accessToken, 3);
    }

    public List<Article> getRecentArticles(String accessToken) {
        return articleRepository.findRecentArticles(accessToken, 12);
    }

    public Optional<Article> getArticleByPath(String path) {
        return articleRepository.findByPath(path);
    }

    public List<Article> getArticlesByCategory(String categoryPath, String accessToken) {
        return articleRepository.findByCategory(categoryPath, accessToken, 20);
    }

    public List<Article> getAllArticlesExcept(String path, String accessToken) {
        return articleRepository.findAllExcept(path, accessToken);
    }

    public List<Article> searchByKeyword(String query) {
        return articleRepository.searchByKeyword(query, 20);
    }

    public List<Article> searchByTag(String tagPath) {
        return articleRepository.findByTagReference(tagPath, 20);
    }

    /**
     * Get article with all view data including graph relationships.
     */
    public ArticleViewData getArticleViewData(String path) {
        Optional<Article> articleOpt = articleRepository.findByPath(path);
        if (articleOpt.isEmpty()) {
            return null;
        }

        Article article = articleOpt.get();

        // Increment views
        articleRepository.incrementViews(path);

        // Build graph data
        ArticleGraphData graphData = buildGraphData(path);

        // Get related articles in same category
        String categoryPath = articlesPath + "/" + article.getCategorySlug();
        List<Article> related = articleRepository.findByCategory(categoryPath, null, 4)
                .stream()
                .filter(a -> !a.getPath().equals(path))
                .limit(3)
                .collect(Collectors.toList());

        return new ArticleViewData(article, graphData, related);
    }

    private ArticleGraphData buildGraphData(String articlePath) {
        ArticleGraphData graphData = new ArticleGraphData();

        // Smart related
        graphData.setSmartRelated(graphRepository.findSmartRelated(articlePath, 5));

        // Timeline
        ArticleGraphData.Timeline timeline = new ArticleGraphData.Timeline();
        timeline.setPredecessors(graphRepository.findPredecessors(articlePath));
        timeline.setSuccessors(graphRepository.findSuccessors(articlePath));
        graphData.setTimeline(timeline);

        // Correction info
        graphRepository.findCorrectionFor(articlePath).ifPresent(correction -> {
            ArticleGraphData.CorrectionInfo info = new ArticleGraphData.CorrectionInfo();
            info.setTitle(correction.getProperties() != null ? correction.getProperties().getTitle() : correction.getName());
            info.setPath(correction.getPath());
            if (correction.getProperties() != null && correction.getProperties().getPublishingDate() != null) {
                info.setPublishedAt(correction.getProperties().getPublishingDate().toString());
            } else if (correction.getUpdatedAt() != null) {
                info.setPublishedAt(correction.getUpdatedAt().toString());
            }
            graphData.setCorrection(info);
        });

        // Article this corrects
        graphRepository.findArticleCorrectedBy(articlePath).ifPresent(original -> {
            ArticleGraphData.CorrectionInfo info = new ArticleGraphData.CorrectionInfo();
            info.setTitle(original.getProperties() != null ? original.getProperties().getTitle() : original.getName());
            info.setPath(original.getPath());
            if (original.getProperties() != null && original.getProperties().getPublishingDate() != null) {
                info.setPublishedAt(original.getProperties().getPublishingDate().toString());
            } else if (original.getUpdatedAt() != null) {
                info.setPublishedAt(original.getUpdatedAt().toString());
            }
            graphData.setCorrectsArticle(info);
        });

        // Opposing views
        graphData.setOpposingViews(graphRepository.findOpposingViews(articlePath));

        // Evidence
        graphData.setEvidence(graphRepository.findEvidence(articlePath));

        // Shared tag articles
        graphData.setSharedTagArticles(graphRepository.findSharedTagArticles(articlePath, 10));

        // Tags
        graphData.setTags(graphRepository.findArticleTags(articlePath));

        return graphData;
    }

    @Transactional
    public void createArticle(ArticleCreateDto dto, String author, String accessToken) {
        String path = articlesPath + "/" + dto.getCategory() + "/" + dto.getSlug();

        ArticleProperties properties = new ArticleProperties();
        properties.setTitle(dto.getTitle());
        properties.setSlug(dto.getSlug());
        properties.setExcerpt(dto.getExcerpt());
        properties.setBody(dto.getBody());
        properties.setCategory(dto.getCategory());
        properties.setTags(dto.getTagReferences(workspace));
        properties.setKeywords(dto.getKeywords());
        properties.setFeatured(dto.isFeatured());
        properties.setStatus(dto.getStatus());
        properties.setPublishingDate(dto.getPublishingDate() != null
                ? dto.getPublishingDate() : OffsetDateTime.now());
        properties.setViews(0);
        properties.setAuthor(author);
        properties.setImageUrl(dto.getImageUrl());

        articleRepository.create(path, dto.getTitle(), properties, accessToken);
    }

    @Transactional
    public void updateArticle(String originalPath, ArticleUpdateDto dto, String accessToken) {
        String newPath = articlesPath + "/" + dto.getCategory() + "/" + dto.getSlug();

        ArticleProperties properties = new ArticleProperties();
        properties.setTitle(dto.getTitle());
        properties.setSlug(dto.getSlug());
        properties.setExcerpt(dto.getExcerpt());
        properties.setBody(dto.getBody());
        properties.setCategory(dto.getCategory());
        properties.setTags(dto.getTagReferences(workspace));
        properties.setKeywords(dto.getKeywords());
        properties.setFeatured(dto.isFeatured());
        properties.setStatus(dto.getStatus());
        properties.setPublishingDate(dto.getPublishingDate());
        properties.setAuthor(dto.getAuthor());
        properties.setImageUrl(dto.getImageUrl());

        // Sync graph relations
        syncConnections(originalPath, dto.getConnections(), accessToken);

        // Check if path changed
        if (!originalPath.equals(newPath)) {
            // Move to new path
            String newParentPath = articlesPath + "/" + dto.getCategory();
            articleRepository.move(originalPath, newParentPath, accessToken);
            // Update at new path
            articleRepository.update(newPath, dto.getTitle(), properties, accessToken);
        } else {
            articleRepository.update(originalPath, dto.getTitle(), properties, accessToken);
        }
    }

    private void syncConnections(String articlePath, List<ArticleConnection> newConnections,
                                 String accessToken) {
        if (newConnections == null) {
            newConnections = Collections.emptyList();
        }

        // Get existing outgoing relations
        List<GraphRepository.ExistingRelation> existingRelations =
                graphRepository.findOutgoingRelations(articlePath);

        // Build sets for comparison
        Set<String> existingSet = new HashSet<>();
        for (GraphRepository.ExistingRelation r : existingRelations) {
            existingSet.add(r.getPath() + "|" + r.getRelationType());
        }

        Set<String> newSet = new HashSet<>();
        for (ArticleConnection c : newConnections) {
            newSet.add(c.getTargetPath() + "|" + c.getRelationType());
        }

        // Remove relations not in new set
        for (GraphRepository.ExistingRelation rel : existingRelations) {
            String key = rel.getPath() + "|" + rel.getRelationType();
            if (!newSet.contains(key)) {
                graphRepository.removeRelation(articlePath, rel.getPath(),
                        rel.getRelationType(), accessToken);
            }
        }

        // Add/update relations
        for (ArticleConnection conn : newConnections) {
            String key = conn.getTargetPath() + "|" + conn.getRelationType();
            double weight = conn.getWeightAsDouble();

            if (existingSet.contains(key)) {
                // Update: remove and re-add with new weight
                graphRepository.removeRelation(articlePath, conn.getTargetPath(),
                        conn.getRelationType(), accessToken);
            }

            graphRepository.createRelation(articlePath, conn.getTargetPath(),
                    conn.getRelationType(), weight, accessToken);
        }
    }

    public void deleteArticle(String path, String accessToken) {
        articleRepository.delete(path, accessToken);
    }

    /**
     * View data wrapper.
     */
    public static class ArticleViewData {
        private final Article article;
        private final ArticleGraphData graphData;
        private final List<Article> relatedArticles;

        public ArticleViewData(Article article, ArticleGraphData graphData, List<Article> relatedArticles) {
            this.article = article;
            this.graphData = graphData;
            this.relatedArticles = relatedArticles;
        }

        public Article getArticle() {
            return article;
        }

        public ArticleGraphData getGraphData() {
            return graphData;
        }

        public List<Article> getRelatedArticles() {
            return relatedArticles;
        }
    }
}
