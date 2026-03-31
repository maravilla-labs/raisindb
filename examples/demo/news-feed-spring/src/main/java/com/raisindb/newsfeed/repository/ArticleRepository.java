package com.raisindb.newsfeed.repository;

import com.fasterxml.jackson.core.JsonProcessingException;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.raisindb.newsfeed.domain.Article;
import com.raisindb.newsfeed.domain.ArticleProperties;
import com.raisindb.newsfeed.util.PathUtils;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;
import org.springframework.beans.factory.annotation.Value;
import org.springframework.jdbc.core.JdbcTemplate;
import org.springframework.jdbc.core.RowMapper;
import org.springframework.stereotype.Repository;

import java.time.OffsetDateTime;
import java.util.List;
import java.util.Optional;

/**
 * Repository for Article operations using RaisinDB custom SQL.
 */
@Repository
public class ArticleRepository {

    private static final Logger log = LoggerFactory.getLogger(ArticleRepository.class);

    private final JdbcTemplate jdbcTemplate;
    private final ObjectMapper objectMapper;
    private final String articlesPath;

    public ArticleRepository(JdbcTemplate jdbcTemplate,
                            ObjectMapper objectMapper,
                            @Value("${raisindb.articles-path}") String articlesPath) {
        this.jdbcTemplate = jdbcTemplate;
        this.objectMapper = objectMapper;
        this.articlesPath = articlesPath;
    }

    private RowMapper<Article> getArticleMapper() {
        return (rs, rowNum) -> {
            Article article = new Article();
            article.setId(rs.getString("id"));
            article.setPath(rs.getString("path"));
            article.setName(rs.getString("name"));
            article.setNodeType(rs.getString("node_type"));

            article.setCreatedAt(rs.getObject("created_at", OffsetDateTime.class));
            article.setUpdatedAt(rs.getObject("updated_at", OffsetDateTime.class));

            String propsJson = rs.getString("properties");
            if (propsJson != null) {
                try {
                    article.setProperties(objectMapper.readValue(propsJson, ArticleProperties.class));
                } catch (JsonProcessingException e) {
                    log.error("Failed to parse article properties: {}", e.getMessage());
                }
            }
            return article;
        };
    }

    /**
     * Find featured articles using DESCENDANT_OF and JSONB containment.
     */
    public List<Article> findFeaturedArticles(String accessToken, int limit) {
        String sql = String.format("""
            SELECT id, path, name, node_type, properties, created_at, updated_at
            FROM social
            WHERE DESCENDANT_OF('%s')
              AND node_type = 'news:Article'
              AND properties @> '{"featured": true, "status": "published"}'
              AND (properties ->> 'publishing_date')::TIMESTAMP <= NOW()
            ORDER BY properties ->> 'publishing_date' DESC
            LIMIT ?
            """, PathUtils.escapeSql(articlesPath));

        return executeWithUserContext(accessToken, () ->
                jdbcTemplate.query(sql, getArticleMapper(), limit));
    }

    /**
     * Find recent published articles.
     */
    public List<Article> findRecentArticles(String accessToken, int limit) {
        String sql = String.format("""
            SELECT id, path, name, node_type, properties, created_at, updated_at
            FROM social
            WHERE DESCENDANT_OF('%s')
              AND node_type = 'news:Article'
              AND properties ->> 'status'::TEXT = 'published'
              AND (properties ->> 'publishing_date')::TIMESTAMP <= NOW()
            ORDER BY properties ->> 'publishing_date' DESC
            LIMIT ?
            """, PathUtils.escapeSql(articlesPath));

        return executeWithUserContext(accessToken, () ->
                jdbcTemplate.query(sql, getArticleMapper(), limit));
    }

    /**
     * Find article by path.
     */
    public Optional<Article> findByPath(String path) {
        String sql = """
            SELECT id, path, name, node_type, properties, created_at, updated_at
            FROM social
            WHERE path = ?
              AND node_type = 'news:Article'
            """;

        List<Article> results = jdbcTemplate.query(sql, getArticleMapper(), path);
        return results.isEmpty() ? Optional.empty() : Optional.of(results.get(0));
    }

    /**
     * Find articles by category using CHILD_OF (direct children only).
     */
    public List<Article> findByCategory(String categoryPath, String accessToken, int limit) {
        String sql = String.format("""
            SELECT id, path, name, node_type, properties, created_at, updated_at
            FROM social
            WHERE CHILD_OF('%s')
              AND node_type = 'news:Article'
              AND properties ->> 'status'::TEXT = 'published'
              AND (properties ->> 'publishing_date')::TIMESTAMP <= NOW()
            ORDER BY properties ->> 'publishing_date' DESC
            LIMIT ?
            """, PathUtils.escapeSql(categoryPath));

        return executeWithUserContext(accessToken, () ->
                jdbcTemplate.query(sql, getArticleMapper(), limit));
    }

    /**
     * Find all articles except a specific one.
     */
    public List<Article> findAllExcept(String excludePath, String accessToken) {
        String sql = String.format("""
            SELECT id, path, name, node_type, properties, created_at, updated_at
            FROM social
            WHERE DESCENDANT_OF('%s')
              AND node_type = 'news:Article'
              AND path != ?
            ORDER BY properties ->> 'publishing_date' DESC
            """, PathUtils.escapeSql(articlesPath));

        return executeWithUserContext(accessToken, () ->
                jdbcTemplate.query(sql, getArticleMapper(), excludePath));
    }

    /**
     * Search articles by tag using REFERENCES predicate.
     */
    public List<Article> findByTagReference(String tagPath, int limit) {
        String referencesTarget = "social:" + tagPath;
        String sql = String.format("""
            SELECT id, path, name, node_type, properties, created_at, updated_at
            FROM social
            WHERE REFERENCES('%s')
              AND node_type = 'news:Article'
              AND properties ->> 'status'::TEXT = 'published'
              AND (properties ->> 'publishing_date')::TIMESTAMP <= NOW()
            ORDER BY properties ->> 'publishing_date' DESC
            LIMIT ?
            """, PathUtils.escapeSql(referencesTarget));

        return jdbcTemplate.query(sql, getArticleMapper(), limit);
    }

    /**
     * Search articles by keyword.
     */
    public List<Article> searchByKeyword(String query, int limit) {
        String sql = String.format("""
            SELECT id, path, name, node_type, properties, created_at, updated_at
            FROM social
            WHERE DESCENDANT_OF('%s')
              AND node_type = 'news:Article'
              AND properties ->> 'status'::TEXT = 'published'
              AND (properties ->> 'publishing_date')::TIMESTAMP <= NOW()
              AND (
                COALESCE(properties ->> 'title', '') ILIKE '%%' || ? || '%%'
                OR COALESCE(properties ->> 'body', '') ILIKE '%%' || ? || '%%'
                OR COALESCE(properties ->> 'excerpt', '') ILIKE '%%' || ? || '%%'
                OR COALESCE(properties::TEXT, '') ILIKE '%%' || ? || '%%'
              )
            ORDER BY properties ->> 'publishing_date' DESC
            LIMIT ?
            """, PathUtils.escapeSql(articlesPath));

        return jdbcTemplate.query(sql, getArticleMapper(), query, query, query, query, limit);
    }

    /**
     * Create article.
     */
    public void create(String path, String name, ArticleProperties properties, String accessToken) {
        String sql = """
            INSERT INTO social (path, node_type, name, properties)
            VALUES (?, 'news:Article', ?, ?::JSONB)
            """;

        try {
            String propsJson = objectMapper.writeValueAsString(properties);
            executeWithUserContext(accessToken, () -> {
                jdbcTemplate.update(sql, path, name, propsJson);
                return null;
            });
        } catch (JsonProcessingException e) {
            throw new RuntimeException("Failed to serialize article properties", e);
        }
    }

    /**
     * Update article properties.
     */
    public void update(String path, String name, ArticleProperties properties, String accessToken) {
        String sql = """
            UPDATE social
            SET name = ?,
                properties = properties || ?::JSONB
            WHERE path = ?
            """;

        try {
            String propsJson = objectMapper.writeValueAsString(properties);
            executeWithUserContext(accessToken, () -> {
                jdbcTemplate.update(sql, name, propsJson, path);
                return null;
            });
        } catch (JsonProcessingException e) {
            throw new RuntimeException("Failed to serialize article properties", e);
        }
    }

    /**
     * Move article using MOVE statement.
     */
    public void move(String originalPath, String newParentPath, String accessToken) {
        String sql = String.format(
                "MOVE social SET path = '%s' TO path = '%s'",
                PathUtils.escapeSql(originalPath),
                PathUtils.escapeSql(newParentPath)
        );

        executeWithUserContext(accessToken, () -> {
            jdbcTemplate.execute(sql);
            return null;
        });
    }

    /**
     * Delete article.
     */
    public int delete(String path, String accessToken) {
        String sql = "DELETE FROM social WHERE path = ?";
        return executeWithUserContext(accessToken, () ->
                jdbcTemplate.update(sql, path));
    }

    /**
     * Increment view count.
     */
    public void incrementViews(String path) {
        String sql = """
            UPDATE social
            SET properties = jsonb_set(
                properties,
                '{views}',
                to_jsonb(COALESCE((properties ->> 'views')::INT, 0) + 1)
            )
            WHERE path = ?
            """;
        jdbcTemplate.update(sql, path);
    }

    /**
     * Execute with user context for RLS.
     */
    private <T> T executeWithUserContext(String accessToken, java.util.function.Supplier<T> operation) {
        if (accessToken != null && !accessToken.isEmpty()) {
            jdbcTemplate.execute("SET app.user = '" + accessToken.replace("'", "''") + "'");
            try {
                return operation.get();
            } finally {
                jdbcTemplate.execute("RESET app.user");
            }
        }
        return operation.get();
    }
}
