package com.raisindb.newsfeed.repository;

import com.fasterxml.jackson.core.JsonProcessingException;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.raisindb.newsfeed.domain.Category;
import com.raisindb.newsfeed.domain.CategoryProperties;
import com.raisindb.newsfeed.util.PathUtils;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;
import org.springframework.beans.factory.annotation.Value;
import org.springframework.jdbc.core.JdbcTemplate;
import org.springframework.jdbc.core.RowMapper;
import org.springframework.stereotype.Repository;

import java.util.List;
import java.util.Optional;

/**
 * Repository for Category (folder) operations using RaisinDB custom SQL.
 */
@Repository
public class CategoryRepository {

    private static final Logger log = LoggerFactory.getLogger(CategoryRepository.class);

    private final JdbcTemplate jdbcTemplate;
    private final ObjectMapper objectMapper;
    private final String articlesPath;

    public CategoryRepository(JdbcTemplate jdbcTemplate,
                              ObjectMapper objectMapper,
                              @Value("${raisindb.articles-path}") String articlesPath) {
        this.jdbcTemplate = jdbcTemplate;
        this.objectMapper = objectMapper;
        this.articlesPath = articlesPath;
    }

    private RowMapper<Category> getCategoryMapper() {
        return (rs, rowNum) -> {
            Category category = new Category();
            category.setId(rs.getString("id"));
            category.setPath(rs.getString("path"));
            category.setName(rs.getString("name"));

            String propsJson = rs.getString("properties");
            if (propsJson != null) {
                try {
                    category.setProperties(objectMapper.readValue(propsJson, CategoryProperties.class));
                } catch (JsonProcessingException e) {
                    log.error("Failed to parse category properties: {}", e.getMessage());
                }
            }
            return category;
        };
    }

    /**
     * Find all categories using CHILD_OF.
     */
    public List<Category> findAllCategories(String accessToken) {
        String sql = String.format("""
            SELECT id, path, name, properties
            FROM social
            WHERE CHILD_OF('%s')
              AND node_type = 'raisin:Folder'
            ORDER BY COALESCE((properties ->> 'order')::INT, 999), name
            """, PathUtils.escapeSql(articlesPath));

        return executeWithUserContext(accessToken, () ->
                jdbcTemplate.query(sql, getCategoryMapper()));
    }

    /**
     * Find category by path.
     */
    public Optional<Category> findByPath(String path) {
        String sql = """
            SELECT id, path, name, properties
            FROM social
            WHERE path = ?
              AND node_type = 'raisin:Folder'
            """;

        List<Category> results = jdbcTemplate.query(sql, getCategoryMapper(), path);
        return results.isEmpty() ? Optional.empty() : Optional.of(results.get(0));
    }

    /**
     * Find category by slug.
     */
    public Optional<Category> findBySlug(String slug, String accessToken) {
        String path = articlesPath + "/" + slug;
        return findByPath(path);
    }

    /**
     * Create category folder.
     */
    public void create(String path, String name, CategoryProperties properties, String accessToken) {
        String sql = """
            INSERT INTO social (path, node_type, name, properties)
            VALUES (?, 'raisin:Folder', ?, ?::JSONB)
            """;

        try {
            String propsJson = objectMapper.writeValueAsString(properties);
            executeWithUserContext(accessToken, () -> {
                jdbcTemplate.update(sql, path, name, propsJson);
                return null;
            });
        } catch (JsonProcessingException e) {
            throw new RuntimeException("Failed to serialize category properties", e);
        }
    }

    /**
     * Update category.
     */
    public void update(String path, String name, CategoryProperties properties, String accessToken) {
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
            throw new RuntimeException("Failed to serialize category properties", e);
        }
    }

    /**
     * Delete category.
     */
    public int delete(String path, String accessToken) {
        String sql = "DELETE FROM social WHERE path = ?";
        return executeWithUserContext(accessToken, () ->
                jdbcTemplate.update(sql, path));
    }

    /**
     * Reorder category using ORDER statement.
     */
    public void reorder(String pathToMove, String referencePath, String accessToken) {
        String sql = String.format(
                "ORDER social SET path = '%s' ABOVE path = '%s'",
                PathUtils.escapeSql(pathToMove),
                PathUtils.escapeSql(referencePath)
        );

        executeWithUserContext(accessToken, () -> {
            jdbcTemplate.execute(sql);
            return null;
        });
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
