---
sidebar_label: "Spring Boot"
sidebar_position: 3
---

# Spring Boot Implementation

## Tech Stack

| Component | Technology | Version |
|-----------|------------|---------|
| **Framework** | Spring Boot | 4.0.0 |
| **Language** | Java | 21 |
| **Database Access** | JdbcTemplate + HikariCP | - |
| **Database Driver** | PostgreSQL JDBC | 42.x |
| **Templates** | Thymeleaf | - |
| **Styling** | TailwindCSS (CDN) | - |
| **JSON** | Jackson | - |
| **Markdown** | Flexmark | 0.64.x |
| **Auth** | Spring Security + JWT | - |

## RaisinDB Connection

```yaml
# src/main/resources/application.yml
spring:
  datasource:
    url: jdbc:postgresql://localhost:5432/your_database
    username: default
    password: your_password
    driver-class-name: org.postgresql.Driver
    hikari:
      maximum-pool-size: 20
      connection-timeout: 5000

# RaisinDB-specific config
raisindb:
  workspace: social
  articles-path: /news/articles
  tags-path: /news/tags
```

## Application Structure

```
src/main/java/com/raisindb/newsfeed/
├── NewsFeedApplication.java      # Spring Boot entry point
├── config/
│   ├── SecurityConfig.java       # Spring Security + JWT
│   ├── WebMvcConfig.java
│   └── JacksonConfig.java        # JSON serialization
├── domain/
│   ├── Article.java              # Article entity
│   ├── ArticleProperties.java    # JSONB properties POJO
│   ├── Tag.java
│   ├── TagProperties.java
│   ├── Category.java
│   ├── ArticleConnection.java    # Graph edge data
│   ├── IncomingConnection.java
│   └── SharedTagArticle.java     # 2-hop query result
├── dto/
│   ├── ArticleCreateDto.java
│   ├── ArticleUpdateDto.java
│   └── LoginDto.java
├── repository/
│   ├── ArticleRepository.java    # DESCENDANT_OF, REFERENCES
│   ├── GraphRepository.java      # GRAPH_TABLE, NEIGHBORS, RELATE
│   ├── TagRepository.java
│   └── CategoryRepository.java
├── service/
│   ├── ArticleService.java       # Business logic
│   ├── TagService.java
│   ├── CategoryService.java
│   └── AuthService.java
├── controller/
│   ├── HomeController.java       # / - home page
│   ├── ArticleController.java    # /articles/** - CRUD
│   ├── SearchController.java     # /search
│   ├── TagController.java        # /settings/tags
│   └── api/
│       └── ArticleApiController.java  # REST API
├── security/
│   ├── JwtAuthenticationFilter.java
│   ├── UserContext.java
│   └── RaisinDbUserContext.java  # Manages SET app.user
└── util/
    ├── PathUtils.java            # SQL escaping
    ├── RelationTypes.java        # Edge type constants
    └── MarkdownService.java

src/main/resources/
├── application.yml
├── templates/
│   ├── layout.html               # Thymeleaf layout
│   ├── home.html
│   ├── article/
│   │   ├── detail.html           # Article + graph widgets
│   │   ├── edit.html
│   │   └── new.html
│   ├── search.html
│   └── settings/
│       ├── tags.html
│       └── categories.html
└── static/
    └── css/custom.css
```

## Page Navigation Flow

```
┌─────────────────────────────────────────────────────────────┐
│  Thymeleaf Layout (layout.html)                             │
│  [Home] [Tech] [Business] [Sports] [Entertainment] [Search] │
└─────────────────────────────────────────────────────────────┘
                              │
           ┌──────────────────┼──────────────────┐
           ▼                  ▼                  ▼
    HomeController     ArticleController   SearchController
    GET /              GET /articles/{cat}  GET /search
           │                  │                   │
           └────────────┬─────┴───────────────────┘
                        ▼
              ArticleController
              GET /articles/{category}/{slug}
                        │
              ┌─────────┴─────────┐
              ▼                   ▼
        ArticleRepository   GraphRepository
        (hierarchical)      (relationships)
```

## Project Setup

```bash
# Clone the demo
git clone https://github.com/maravilla-labs/raisindb.git
cd raisindb/examples/demo/news-feed-spring

# Run with Gradle (Java 21 required)
./gradlew bootRun
```

Open http://localhost:8090

## Key Code Patterns

### Repository with JdbcTemplate

```java
// repository/ArticleRepository.java
@Repository
public class ArticleRepository {
    private final JdbcTemplate jdbcTemplate;
    private final ObjectMapper objectMapper;

    public List<Article> findFeaturedArticles(String accessToken, int limit) {
        String sql = """
            SELECT id, path, name, node_type, properties, created_at, updated_at
            FROM social
            WHERE DESCENDANT_OF('/news/articles')
              AND node_type = 'news:Article'
              AND properties @> '{"featured": true, "status": "published"}'
            ORDER BY properties ->> 'publishing_date' DESC
            LIMIT ?
            """;

        return executeWithUserContext(accessToken, () ->
            jdbcTemplate.query(sql, getArticleMapper(), limit)
        );
    }

    private RowMapper<Article> getArticleMapper() {
        return (rs, rowNum) -> {
            Article article = new Article();
            article.setId(rs.getString("id"));
            article.setPath(rs.getString("path"));
            article.setName(rs.getString("name"));
            article.setNodeType(rs.getString("node_type"));
            article.setCreatedAt(rs.getObject("created_at", OffsetDateTime.class));

            // Parse JSONB properties
            String propsJson = rs.getString("properties");
            if (propsJson != null) {
                article.setProperties(
                    objectMapper.readValue(propsJson, ArticleProperties.class)
                );
            }
            return article;
        };
    }
}
```

### Dynamic Navigation from Database

```java
// repository/CategoryRepository.java
public List<Category> findAllCategories() {
    String sql = """
        SELECT path, name, properties
        FROM social
        WHERE CHILD_OF('/news/articles')
          AND node_type = 'news:Category'
        ORDER BY properties ->> 'sort_order' ASC
        """;
    return jdbcTemplate.query(sql, getCategoryMapper());
}
```

### Graph Repository with GRAPH_TABLE

```java
// repository/GraphRepository.java
@Repository
public class GraphRepository {
    private final JdbcTemplate jdbcTemplate;

    public List<Article> findPredecessors(String articlePath) {
        String sql = """
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
            ORDER BY (g.properties ->> 'publishing_date')::TIMESTAMP ASC
            """.formatted(escapeSql(articlePath));

        return jdbcTemplate.query(sql, getArticleMapper());
    }

    public void createRelation(String fromPath, String toPath,
                               String relationType, double weight) {
        String sql = """
            RELATE FROM path='%s' IN WORKSPACE 'social'
              TO path='%s' IN WORKSPACE 'social'
              TYPE '%s' WEIGHT %f
            """.formatted(
                escapeSql(fromPath),
                escapeSql(toPath),
                escapeSql(relationType),
                weight
            );
        jdbcTemplate.execute(sql);
    }
}
```

### RLS Context Helper

```java
// Shared pattern in repositories
private <T> T executeWithUserContext(String accessToken,
                                     Supplier<T> operation) {
    if (accessToken != null && !accessToken.isEmpty()) {
        // Escape single quotes to prevent SQL injection
        String safeToken = accessToken.replace("'", "''");
        jdbcTemplate.execute("SET app.user = '" + safeToken + "'");
        try {
            return operation.get();
        } finally {
            jdbcTemplate.execute("RESET app.user");
        }
    }
    return operation.get();
}
```

---

## Source Code

Full implementation: [news-feed-spring](https://github.com/maravilla-labs/raisindb/tree/main/examples/demo/news-feed-spring)
