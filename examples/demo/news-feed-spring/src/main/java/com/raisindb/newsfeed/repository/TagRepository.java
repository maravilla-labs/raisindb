package com.raisindb.newsfeed.repository;

import com.fasterxml.jackson.core.JsonProcessingException;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.raisindb.newsfeed.domain.Tag;
import com.raisindb.newsfeed.domain.TagProperties;
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
 * Repository for Tag operations using RaisinDB custom SQL.
 */
@Repository
public class TagRepository {

    private static final Logger log = LoggerFactory.getLogger(TagRepository.class);

    private final JdbcTemplate jdbcTemplate;
    private final ObjectMapper objectMapper;
    private final String tagsPath;

    public TagRepository(JdbcTemplate jdbcTemplate,
                         ObjectMapper objectMapper,
                         @Value("${raisindb.tags-path}") String tagsPath) {
        this.jdbcTemplate = jdbcTemplate;
        this.objectMapper = objectMapper;
        this.tagsPath = tagsPath;
    }

    private RowMapper<Tag> getTagMapper() {
        return (rs, rowNum) -> {
            Tag tag = new Tag();
            tag.setId(rs.getString("id"));
            tag.setPath(rs.getString("path"));
            tag.setName(rs.getString("name"));
            tag.setNodeType(rs.getString("node_type"));

            String propsJson = rs.getString("properties");
            if (propsJson != null) {
                try {
                    tag.setProperties(objectMapper.readValue(propsJson, TagProperties.class));
                } catch (JsonProcessingException e) {
                    log.error("Failed to parse tag properties: {}", e.getMessage());
                }
            }
            return tag;
        };
    }

    /**
     * Find all tags using DESCENDANT_OF.
     */
    public List<Tag> findAllTags(String accessToken) {
        String sql = String.format("""
            SELECT id, path, name, node_type, properties
            FROM social
            WHERE DESCENDANT_OF('%s')
              AND node_type = 'news:Tag'
            ORDER BY path
            """, PathUtils.escapeSql(tagsPath));

        return executeWithUserContext(accessToken, () ->
                jdbcTemplate.query(sql, getTagMapper()));
    }

    /**
     * Find tag by path.
     */
    public Optional<Tag> findByPath(String path) {
        String sql = """
            SELECT id, path, name, node_type, properties
            FROM social
            WHERE path = ?
              AND node_type = 'news:Tag'
            """;

        List<Tag> results = jdbcTemplate.query(sql, getTagMapper(), path);
        return results.isEmpty() ? Optional.empty() : Optional.of(results.get(0));
    }

    /**
     * Find tag by ID.
     */
    public Optional<Tag> findById(String id) {
        String sql = """
            SELECT id, path, name, node_type, properties
            FROM social
            WHERE id = ?
              AND node_type = 'news:Tag'
            """;

        List<Tag> results = jdbcTemplate.query(sql, getTagMapper(), id);
        return results.isEmpty() ? Optional.empty() : Optional.of(results.get(0));
    }

    /**
     * Create tag.
     */
    public void create(String path, String name, TagProperties properties, String accessToken) {
        String sql = """
            INSERT INTO social (path, node_type, name, properties)
            VALUES (?, 'news:Tag', ?, ?::JSONB)
            """;

        try {
            String propsJson = objectMapper.writeValueAsString(properties);
            executeWithUserContext(accessToken, () -> {
                jdbcTemplate.update(sql, path, name, propsJson);
                return null;
            });
        } catch (JsonProcessingException e) {
            throw new RuntimeException("Failed to serialize tag properties", e);
        }
    }

    /**
     * Update tag.
     */
    public void update(String path, TagProperties properties, String accessToken) {
        String sql = """
            UPDATE social
            SET properties = ?::JSONB
            WHERE path = ?
            """;

        try {
            String propsJson = objectMapper.writeValueAsString(properties);
            executeWithUserContext(accessToken, () -> {
                jdbcTemplate.update(sql, propsJson, path);
                return null;
            });
        } catch (JsonProcessingException e) {
            throw new RuntimeException("Failed to serialize tag properties", e);
        }
    }

    /**
     * Delete tag.
     */
    public int delete(String path, String accessToken) {
        String sql = "DELETE FROM social WHERE path = ?";
        return executeWithUserContext(accessToken, () ->
                jdbcTemplate.update(sql, path));
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
