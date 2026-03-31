package com.raisindb.newsfeed.repository;

import com.fasterxml.jackson.core.JsonProcessingException;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.raisindb.newsfeed.domain.*;
import com.raisindb.newsfeed.util.PathUtils;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;
import org.springframework.jdbc.core.JdbcTemplate;
import org.springframework.jdbc.core.RowMapper;
import org.springframework.stereotype.Repository;

import java.time.OffsetDateTime;
import java.util.List;
import java.util.Optional;

/**
 * Repository for Graph operations using GRAPH_TABLE, NEIGHBORS, RELATE, and UNRELATE.
 */
@Repository
public class GraphRepository {

    private static final Logger log = LoggerFactory.getLogger(GraphRepository.class);

    private final JdbcTemplate jdbcTemplate;
    private final ObjectMapper objectMapper;

    public GraphRepository(JdbcTemplate jdbcTemplate, ObjectMapper objectMapper) {
        this.jdbcTemplate = jdbcTemplate;
        this.objectMapper = objectMapper;
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
     * Find correction article (incoming corrects edge) using GRAPH_TABLE.
     */
    public Optional<Article> findCorrectionFor(String articlePath) {
        String safePath = PathUtils.escapeSql(articlePath);
        String sql = String.format("""
            SELECT * FROM GRAPH_TABLE(
                MATCH (this:Article)<-[:corrects]-(correction:Article)
                WHERE this.path = '%s'
                COLUMNS (
                    correction.id AS id,
                    correction.path AS path,
                    correction.name AS name,
                    correction.node_type AS node_type,
                    correction.properties AS properties,
                    correction.created_at AS created_at,
                    correction.updated_at AS updated_at
                )
            ) AS g
            LIMIT 1
            """, safePath);

        try {
            List<Article> results = jdbcTemplate.query(sql, getArticleMapper());
            return results.isEmpty() ? Optional.empty() : Optional.of(results.get(0));
        } catch (Exception e) {
            log.debug("Graph query failed (likely no results): {}", e.getMessage());
            return Optional.empty();
        }
    }

    /**
     * Find article that this article corrects (outgoing corrects edge).
     */
    public Optional<Article> findArticleCorrectedBy(String articlePath) {
        String safePath = PathUtils.escapeSql(articlePath);
        String sql = String.format("""
            SELECT * FROM GRAPH_TABLE(
                MATCH (this:Article)-[:corrects]->(original:Article)
                WHERE this.path = '%s'
                COLUMNS (
                    original.id AS id,
                    original.path AS path,
                    original.name AS name,
                    original.node_type AS node_type,
                    original.properties AS properties,
                    original.created_at AS created_at,
                    original.updated_at AS updated_at
                )
            ) AS g
            LIMIT 1
            """, safePath);

        try {
            List<Article> results = jdbcTemplate.query(sql, getArticleMapper());
            return results.isEmpty() ? Optional.empty() : Optional.of(results.get(0));
        } catch (Exception e) {
            log.debug("Graph query failed (likely no results): {}", e.getMessage());
            return Optional.empty();
        }
    }

    /**
     * Find predecessors (multi-hop continues chain).
     */
    public List<Article> findPredecessors(String articlePath) {
        String safePath = PathUtils.escapeSql(articlePath);
        String sql = String.format("""
            SELECT * FROM GRAPH_TABLE(
                MATCH (this)-[:continues*]->(prev)
                WHERE this.path = '%s'
                COLUMNS (
                    prev.id AS id,
                    prev.path AS path,
                    prev.name AS name,
                    prev.node_type AS node_type,
                    prev.properties AS properties,
                    prev.created_at AS created_at,
                    prev.updated_at AS updated_at
                )
            ) AS g
            ORDER BY (g.properties ->> 'publishing_date')::TIMESTAMP ASC NULLS LAST
            """, safePath);

        try {
            return jdbcTemplate.query(sql, getArticleMapper());
        } catch (Exception e) {
            log.debug("Graph query failed (likely no results): {}", e.getMessage());
            return List.of();
        }
    }

    /**
     * Find successors (multi-hop reverse continues chain).
     */
    public List<Article> findSuccessors(String articlePath) {
        String safePath = PathUtils.escapeSql(articlePath);
        String sql = String.format("""
            SELECT * FROM GRAPH_TABLE(
                MATCH (this:Article)<-[:continues*]-(next:Article)
                WHERE this.path = '%s'
                COLUMNS (
                    next.id AS id,
                    next.path AS path,
                    next.name AS name,
                    next.node_type AS node_type,
                    next.properties AS properties,
                    next.created_at AS created_at,
                    next.updated_at AS updated_at
                )
            ) AS g
            ORDER BY (g.properties ->> 'publishing_date')::TIMESTAMP ASC NULLS LAST
            """, safePath);

        try {
            return jdbcTemplate.query(sql, getArticleMapper());
        } catch (Exception e) {
            log.debug("Graph query failed (likely no results): {}", e.getMessage());
            return List.of();
        }
    }

    /**
     * Find smart related articles (similar-to, see-also, updates relations).
     */
    public List<ArticleGraphData.SmartRelatedArticle> findSmartRelated(String articlePath, int limit) {
        String safePath = PathUtils.escapeSql(articlePath);
        String sql = String.format("""
            SELECT * FROM GRAPH_TABLE(
                MATCH (this)-[r:`similar-to`|`see-also`|updates]->(related)
                WHERE this.path = '%s'
                COLUMNS (
                    related.id AS id,
                    related.path AS path,
                    related.name AS name,
                    related.node_type AS node_type,
                    related.properties AS properties,
                    related.created_at AS created_at,
                    related.updated_at AS updated_at,
                    r.type AS relation_type,
                    r.weight AS weight
                )
            ) AS g
            ORDER BY g.weight DESC
            LIMIT %d
            """, safePath, limit);

        try {
            return jdbcTemplate.query(sql, (rs, rowNum) -> {
                ArticleGraphData.SmartRelatedArticle sra = new ArticleGraphData.SmartRelatedArticle();
                Article article = getArticleMapper().mapRow(rs, rowNum);
                sra.setArticle(article);
                sra.setRelationType(rs.getString("relation_type"));
                double weight = rs.getDouble("weight");
                sra.setWeight((int) (weight * 100));
                return sra;
            });
        } catch (Exception e) {
            log.debug("Graph query failed (likely no results): {}", e.getMessage());
            return List.of();
        }
    }

    /**
     * Find opposing views (bidirectional contradicts).
     */
    public List<Article> findOpposingViews(String articlePath) {
        String safePath = PathUtils.escapeSql(articlePath);
        String sql = String.format("""
            SELECT * FROM GRAPH_TABLE(
                MATCH (this:Article)-[:contradicts]-(other:Article)
                WHERE this.path = '%s'
                COLUMNS (
                    other.id AS id,
                    other.path AS path,
                    other.name AS name,
                    other.node_type AS node_type,
                    other.properties AS properties,
                    other.created_at AS created_at,
                    other.updated_at AS updated_at
                )
            ) AS g
            """, safePath);

        try {
            return jdbcTemplate.query(sql, getArticleMapper());
        } catch (Exception e) {
            log.debug("Graph query failed (likely no results): {}", e.getMessage());
            return List.of();
        }
    }

    /**
     * Find evidence articles (bidirectional provides-evidence-for).
     */
    public List<Article> findEvidence(String articlePath) {
        String safePath = PathUtils.escapeSql(articlePath);
        String sql = String.format("""
            SELECT * FROM GRAPH_TABLE(
                MATCH (this:Article)-[:`provides-evidence-for`]-(other:Article)
                WHERE this.path = '%s'
                COLUMNS (
                    other.id AS id,
                    other.path AS path,
                    other.name AS name,
                    other.node_type AS node_type,
                    other.properties AS properties,
                    other.created_at AS created_at,
                    other.updated_at AS updated_at
                )
            ) AS g
            """, safePath);

        try {
            return jdbcTemplate.query(sql, getArticleMapper());
        } catch (Exception e) {
            log.debug("Graph query failed (likely no results): {}", e.getMessage());
            return List.of();
        }
    }

    /**
     * Find articles sharing same tags (2-hop pattern).
     */
    public List<SharedTagArticle> findSharedTagArticles(String articlePath, int limit) {
        String safePath = PathUtils.escapeSql(articlePath);
        String sql = String.format("""
            SELECT * FROM GRAPH_TABLE(
                MATCH (this)-[:`tagged-with`]->(tag)<-[:`tagged-with`]-(other)
                WHERE this.path = '%s'
                  AND other.path <> this.path
                COLUMNS (
                    other.id AS article_id,
                    other.path AS article_path,
                    other.name AS article_title,
                    tag.name AS shared_tag,
                    tag.path AS tag_path
                )
            ) AS g
            LIMIT %d
            """, safePath, limit);

        try {
            return jdbcTemplate.query(sql, (rs, rowNum) -> {
                SharedTagArticle sta = new SharedTagArticle();
                sta.setArticleId(rs.getString("article_id"));
                sta.setArticlePath(rs.getString("article_path"));
                sta.setArticleTitle(rs.getString("article_title"));
                sta.setSharedTag(rs.getString("shared_tag"));
                sta.setTagPath(rs.getString("tag_path"));
                return sta;
            });
        } catch (Exception e) {
            log.debug("Graph query failed (likely no results): {}", e.getMessage());
            return List.of();
        }
    }

    /**
     * Find article's tags using GRAPH_TABLE.
     */
    public List<ArticleGraphData.ArticleTag> findArticleTags(String articlePath) {
        String safePath = PathUtils.escapeSql(articlePath);
        String sql = String.format("""
            SELECT * FROM GRAPH_TABLE(
                MATCH (article:Article)-[:`tagged-with`]->(tag:Tag)
                WHERE article.path = '%s'
                COLUMNS (
                    tag.path AS path,
                    tag.name AS label,
                    tag.icon AS icon,
                    tag.color AS color
                )
            ) AS tags
            """, safePath);

        try {
            return jdbcTemplate.query(sql, (rs, rowNum) -> {
                ArticleGraphData.ArticleTag tag = new ArticleGraphData.ArticleTag();
                tag.setPath(rs.getString("path"));
                tag.setLabel(rs.getString("label"));
                tag.setIcon(rs.getString("icon"));
                tag.setColor(rs.getString("color"));
                return tag;
            });
        } catch (Exception e) {
            log.debug("Graph query failed (likely no results): {}", e.getMessage());
            return List.of();
        }
    }

    /**
     * Find incoming connections using NEIGHBORS.
     */
    public List<IncomingConnection> findIncomingConnections(String articlePath) {
        String workspacePath = "social:" + PathUtils.escapeSql(articlePath);
        String sql = String.format("""
            SELECT n.id, n.path, n.properties, n.relation_type, n.weight
            FROM NEIGHBORS('%s', 'IN', NULL) AS n
            WHERE n.node_type = 'news:Article'
            """, workspacePath);

        try {
            return jdbcTemplate.query(sql, (rs, rowNum) -> {
                IncomingConnection conn = new IncomingConnection();
                conn.setSourceId(rs.getString("id"));
                conn.setSourcePath(rs.getString("path"));

                String propsJson = rs.getString("properties");
                if (propsJson != null) {
                    try {
                        ArticleProperties props = objectMapper.readValue(propsJson, ArticleProperties.class);
                        conn.setSourceTitle(props.getTitle() != null ? props.getTitle() : "Unknown");
                    } catch (JsonProcessingException e) {
                        conn.setSourceTitle("Unknown");
                    }
                }

                conn.setRelationType(rs.getString("relation_type"));
                double weight = rs.getDouble("weight");
                conn.setWeight((int) (weight * 100));
                return conn;
            });
        } catch (Exception e) {
            log.debug("NEIGHBORS query failed: {}", e.getMessage());
            return List.of();
        }
    }

    /**
     * Find outgoing relations using NEIGHBORS.
     */
    public List<ExistingRelation> findOutgoingRelations(String articlePath) {
        String workspacePath = "social:" + PathUtils.escapeSql(articlePath);
        String sql = String.format("""
            SELECT n.path, n.relation_type
            FROM NEIGHBORS('%s', 'OUT', NULL) AS n
            """, workspacePath);

        try {
            return jdbcTemplate.query(sql, (rs, rowNum) -> {
                ExistingRelation rel = new ExistingRelation();
                rel.setPath(rs.getString("path"));
                rel.setRelationType(rs.getString("relation_type"));
                return rel;
            });
        } catch (Exception e) {
            log.debug("NEIGHBORS query failed: {}", e.getMessage());
            return List.of();
        }
    }

    /**
     * Create relation using RELATE.
     */
    public void createRelation(String fromPath, String toPath, String relationType,
                               double weight, String accessToken) {
        String sql = String.format("""
            RELATE FROM path='%s' IN WORKSPACE 'social'
              TO path='%s' IN WORKSPACE 'social'
              TYPE '%s' WEIGHT %f
            """,
                PathUtils.escapeSql(fromPath),
                PathUtils.escapeSql(toPath),
                PathUtils.escapeSql(relationType),
                weight);

        executeWithUserContext(accessToken, () -> {
            jdbcTemplate.execute(sql);
            return null;
        });
    }

    /**
     * Remove relation using UNRELATE.
     */
    public void removeRelation(String fromPath, String toPath, String relationType,
                               String accessToken) {
        String sql = String.format("""
            UNRELATE FROM path='%s' IN WORKSPACE 'social'
              TO path='%s' IN WORKSPACE 'social'
              TYPE '%s'
            """,
                PathUtils.escapeSql(fromPath),
                PathUtils.escapeSql(toPath),
                PathUtils.escapeSql(relationType));

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

    /**
     * Helper class for existing relations.
     */
    public static class ExistingRelation {
        private String path;
        private String relationType;

        public String getPath() {
            return path;
        }

        public void setPath(String path) {
            this.path = path;
        }

        public String getRelationType() {
            return relationType;
        }

        public void setRelationType(String relationType) {
            this.relationType = relationType;
        }
    }
}
