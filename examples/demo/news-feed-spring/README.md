# News Feed Spring Demo

A Spring Boot 4.0.0 implementation of the News Feed demo, showcasing RaisinDB's unique features including hierarchical data, graph relationships, and SQL/PGQ queries.

## Features

- **Article Management**: Create, edit, delete articles with categories and tags
- **Hierarchical Tags**: Nested tag structure using RaisinDB's path-based hierarchy
- **Graph Relationships**: Article connections (continues, updates, corrects, contradicts, etc.)
- **Search**: Keyword search and tag-based filtering using REFERENCES predicate
- **Authentication**: JWT-based auth with external auth API integration
- **View Counting**: Track article views

## Tech Stack

- **Backend**: Spring Boot 4.0.0, Java 21
- **Build**: Gradle 8.14+
- **Database**: RaisinDB (PostgreSQL-compatible)
- **Frontend**: Thymeleaf + TailwindCSS (via CDN)
- **Security**: Spring Security with JWT cookies

## Prerequisites

- Java 21 (required - Java 25 not yet supported by build tools)
- Gradle 8.14+ (included via wrapper)
- RaisinDB running on localhost:5432
- Auth API running on localhost:8081 (optional, for authentication)

## Database Configuration

The application connects to the same database as the SvelteKit demo:

```
URL: jdbc:postgresql://localhost:5432/social_feed_demo_rel4
Username: default
Password: raisin_N7U7POgxOh9WqZIaPC5YK1W23HlEieb9
Workspace: social
```

## Running the Application

```bash
cd examples/demo/news-feed-spring
./gradlew bootRun
```

The application will be available at http://localhost:8080

## RaisinDB Features Demonstrated

### Path-based Hierarchical Queries

```sql
-- Find all articles
SELECT * FROM social WHERE DESCENDANT_OF('/superbigshit/articles')

-- Find categories (direct children)
SELECT * FROM social WHERE CHILD_OF('/superbigshit/articles')
```

### Reference Index Queries

```sql
-- Find articles by tag using REFERENCES predicate
SELECT * FROM social WHERE REFERENCES('social:/superbigshit/tags/tech-stack/rust')
```

### SQL/PGQ Graph Queries

```sql
-- Find article corrections using GRAPH_TABLE
SELECT * FROM GRAPH_TABLE(
    MATCH (this)<-[:corrects]-(correction)
    WHERE this.path = '/superbigshit/articles/tech/ai-coding-assistants'
    COLUMNS (correction.id, correction.path, correction.name)
)

-- Multi-hop timeline traversal
SELECT * FROM GRAPH_TABLE(
    MATCH (this)-[:continues*]->(prev)
    WHERE this.path = '/superbigshit/articles/tech/rust-web'
    COLUMNS (prev.id, prev.path, prev.properties)
)

-- 2-hop pattern: articles sharing tags
SELECT * FROM GRAPH_TABLE(
    MATCH (this)-[:tagged-with]->(tag)<-[:tagged-with]-(other)
    WHERE this.path = '...' AND other.path <> this.path
    COLUMNS (other.id, other.path, tag.name AS shared_tag)
)
```

### NEIGHBORS Function

```sql
-- Find incoming connections
SELECT n.id, n.path, n.relation_type, n.weight
FROM NEIGHBORS('social:/superbigshit/articles/...', 'IN', NULL) AS n
WHERE n.node_type = 'news:Article'
```

### RELATE/UNRELATE for Graph Mutations

```sql
-- Create relationship
RELATE FROM path='/article1' IN WORKSPACE 'social'
  TO path='/article2' IN WORKSPACE 'social'
  TYPE 'similar-to' WEIGHT 0.8

-- Remove relationship
UNRELATE FROM path='/article1' IN WORKSPACE 'social'
  TO path='/article2' IN WORKSPACE 'social'
  TYPE 'similar-to'
```

### Row-Level Security

```sql
-- Set user context before queries
SET app.user = 'jwt-token-here';
SELECT * FROM social WHERE ...;
RESET app.user;
```

## Project Structure

```
src/main/java/com/raisindb/newsfeed/
├── NewsFeedApplication.java
├── config/          # Security, DataSource, WebMvc config
├── domain/          # Entity classes (Article, Tag, Category, etc.)
├── dto/             # Data transfer objects
├── repository/      # Database access with JdbcTemplate
├── service/         # Business logic
├── controller/      # Web controllers
├── security/        # JWT authentication
└── util/            # Path utilities, relation types

src/main/resources/
├── application.yml
├── templates/       # Thymeleaf templates
└── static/          # CSS and JavaScript
```

## Comparison with SvelteKit Demo

| Feature | SvelteKit Demo | Spring Boot Demo |
|---------|---------------|------------------|
| Database queries | pg library | JdbcTemplate |
| Templating | Svelte components | Thymeleaf |
| Styling | TailwindCSS (npm) | TailwindCSS (CDN) |
| Auth tokens | HttpOnly cookies | HttpOnly cookies |
| RLS context | SET app.user | SET app.user |

Both demos use the same database, same SQL queries, and same authentication API.

## Development

Enable hot reload during development:

```bash
./gradlew bootRun --args='--spring.devtools.restart.enabled=true'
```

## Building for Production

```bash
./gradlew build
java -jar build/libs/news-feed-spring-0.0.1-SNAPSHOT.jar
```
