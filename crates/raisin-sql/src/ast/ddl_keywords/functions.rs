//! SQL function keywords (hierarchy, full-text, vector, JSON, aggregate, window)

use super::types::{KeywordCategory, KeywordInfo};

/// Hierarchy SQL function keywords (DEPTH, PARENT, ANCESTOR, PATH_STARTS_WITH)
pub(super) fn hierarchy_function_keywords() -> Vec<KeywordInfo> {
    vec![
        KeywordInfo {
            keyword: "DEPTH".into(),
            category: KeywordCategory::SqlFunction,
            description: "Returns the depth level (number of segments) of a path.".into(),
            syntax: Some("DEPTH(path)".into()),
            example: Some("SELECT * FROM nodes WHERE DEPTH(path) = 3".into()),
        },
        KeywordInfo {
            keyword: "PARENT".into(),
            category: KeywordCategory::SqlFunction,
            description: "Returns the parent path. Optional second arg for levels up.".into(),
            syntax: Some("PARENT(path [, levels])".into()),
            example: Some("SELECT * FROM nodes WHERE PARENT(path) = '/content'".into()),
        },
        KeywordInfo {
            keyword: "ANCESTOR".into(),
            category: KeywordCategory::SqlFunction,
            description: "Returns ancestor at specific absolute depth from root.".into(),
            syntax: Some("ANCESTOR(path, depth)".into()),
            example: Some("SELECT ANCESTOR(path, 2) AS section FROM nodes".into()),
        },
        KeywordInfo {
            keyword: "PATH_STARTS_WITH".into(),
            category: KeywordCategory::SqlFunction,
            description:
                "Returns true if path starts with prefix. Efficient for descendant queries.".into(),
            syntax: Some("PATH_STARTS_WITH(path, prefix)".into()),
            example: Some(
                "SELECT * FROM nodes WHERE PATH_STARTS_WITH(path, '/content/blog')".into(),
            ),
        },
    ]
}

/// Full-text search function keywords
pub(super) fn fulltext_function_keywords() -> Vec<KeywordInfo> {
    vec![
        KeywordInfo {
            keyword: "FULLTEXT_MATCH".into(),
            category: KeywordCategory::SqlFunction,
            description: "Full-text search using Tantivy. Supports AND, OR, NOT, wildcards (*), fuzzy (~), phrases.".into(),
            syntax: Some("FULLTEXT_MATCH(query, language)".into()),
            example: Some("SELECT * FROM nodes WHERE FULLTEXT_MATCH('rust AND database', 'en')".into()),
        },
        KeywordInfo {
            keyword: "FULLTEXT_SEARCH".into(),
            category: KeywordCategory::TableFunction,
            description: "Full-text search. The workspace scope is REQUIRED: name one, \
                 'a, b, c' for several, 'content-*' for a family, or 'ALL READABLE' for \
                 every workspace you may read. Language is an ISO 639-1 code ('en', not \
                 'english' -- the index stores two-letter codes)."
                .into(),
            syntax: Some(
                "FULLTEXT_SEARCH(query, language, workspaces => '<scope>' [, limit => n])".into(),
            ),
            example: Some(
                "SELECT * FROM FULLTEXT_SEARCH('content management', 'en', \
                 workspaces => 'library', limit => 50)"
                    .into(),
            ),
        },
    ]
}

/// Vector search function keywords
pub(super) fn vector_function_keywords() -> Vec<KeywordInfo> {
    vec![
        KeywordInfo {
            keyword: "EMBEDDING".into(),
            category: KeywordCategory::SqlFunction,
            description: "Generate embedding vector from text for similarity search.".into(),
            syntax: Some("EMBEDDING(text)".into()),
            example: Some(
                "SELECT * FROM KNN(EMBEDDING('find similar articles'), 10, \
                 workspaces => 'ALL READABLE')"
                    .into(),
            ),
        },
        KeywordInfo {
            keyword: "KNN".into(),
            category: KeywordCategory::TableFunction,
            description: "Pure vector search: HYBRID_SEARCH with the full-text leg switched \
                 off, same columns (full-text ones NULL), same row-level security. \
                 Argument 1 is a text literal, EMBEDDING('<text>'), or a vector literal. \
                 The workspace scope is REQUIRED. granularity => 'chunk' returns one row \
                 per PASSAGE with its chunk_text, so LIMIT k means k passages and several \
                 rows may share a node — the RAG form. The default 'node' returns one row \
                 per document, so LIMIT k means k documents."
                .into(),
            syntax: Some(
                "KNN(query, limit, workspaces => '<scope>' [, max_distance => 0.9] \
                 [, kind => 'text'|'image'|'all'] [, granularity => 'node'|'chunk'])"
                    .into(),
            ),
            example: Some(
                "SELECT path, chunk_index, chunk_text FROM \
                 KNN('how does ticketing work', 8, workspaces => 'ALL READABLE', \
                 granularity => 'chunk')"
                    .into(),
            ),
        },
        KeywordInfo {
            keyword: "HYBRID_SEARCH".into(),
            category: KeywordCategory::TableFunction,
            description: "Full-text + vector search fused with Reciprocal Rank Fusion. The \
                 workspace scope is REQUIRED and is part of the query text, so you can \
                 always read the corpus off the statement: one name, 'a, b, c', a glob \
                 'content-*', or 'ALL READABLE' for every workspace you may read (the \
                 recommended RAG form). Set fulltext_weight => 0 for a cross-lingual query \
                 whose lexical leg would only add noise, or vector_weight => 0 for keyword \
                 search without an embedding provider. Each hit reports its own workspace \
                 in workspace_id. granularity => 'chunk' returns one row per PASSAGE \
                 with its chunk_text (LIMIT k = k passages, several may share a node); \
                 the default 'node' returns one row per document."
                .into(),
            syntax: Some(
                "HYBRID_SEARCH(query, limit [, workspace], workspaces => '<scope>' \
                 [, language => 'en'] [, vector_weight => 1.0] \
                 [, fulltext_weight => 1.0] [, max_distance => 0.6] \
                 [, kind => 'text'|'image'|'all'] \
                 [, granularity => 'node'|'chunk'])"
                    .into(),
            ),
            example: Some(
                "SELECT * FROM HYBRID_SEARCH('find articles about AI', 10, \
                 workspaces => 'library, handbook')"
                    .into(),
            ),
        },
        KeywordInfo {
            keyword: "NEIGHBORS".into(),
            category: KeywordCategory::TableFunction,
            description: "Graph traversal to find connected nodes.".into(),
            syntax: Some("NEIGHBORS(node_id, direction, depth)".into()),
            example: Some("SELECT * FROM NEIGHBORS('node-123', 'outbound', 2)".into()),
        },
        KeywordInfo {
            keyword: "CYPHER".into(),
            category: KeywordCategory::TableFunction,
            description: "Execute Cypher graph query and return results as table.".into(),
            syntax: Some("CYPHER(query)".into()),
            example: Some("SELECT * FROM CYPHER('MATCH (a)-[r]->(b) RETURN a, r, b')".into()),
        },
    ]
}

/// JSON function keywords
pub(super) fn json_function_keywords() -> Vec<KeywordInfo> {
    vec![
        KeywordInfo {
            keyword: "JSON_VALUE".into(),
            category: KeywordCategory::JsonFunction,
            description: "Extract scalar value using JSONPath.".into(),
            syntax: Some("JSON_VALUE(json, '$.path.to.value')".into()),
            example: Some("SELECT JSON_VALUE(properties, '$.seo.title') FROM nodes".into()),
        },
        KeywordInfo {
            keyword: "JSON_EXISTS".into(),
            category: KeywordCategory::JsonFunction,
            description: "Check if JSONPath exists in document.".into(),
            syntax: Some("JSON_EXISTS(json, '$.path')".into()),
            example: Some(
                "SELECT * FROM nodes WHERE JSON_EXISTS(properties, '$.metadata.tags')".into(),
            ),
        },
        KeywordInfo {
            keyword: "JSON_GET_TEXT".into(),
            category: KeywordCategory::JsonFunction,
            description: "Extract top-level key as text.".into(),
            syntax: Some("JSON_GET_TEXT(json, 'key')".into()),
            example: Some("SELECT JSON_GET_TEXT(properties, 'title') FROM nodes".into()),
        },
        KeywordInfo {
            keyword: "JSON_GET_DOUBLE".into(),
            category: KeywordCategory::JsonFunction,
            description: "Extract top-level key as double.".into(),
            syntax: Some("JSON_GET_DOUBLE(json, 'key')".into()),
            example: Some("SELECT JSON_GET_DOUBLE(properties, 'price') FROM nodes".into()),
        },
        KeywordInfo {
            keyword: "JSON_GET_INT".into(),
            category: KeywordCategory::JsonFunction,
            description: "Extract top-level key as integer.".into(),
            syntax: Some("JSON_GET_INT(json, 'key')".into()),
            example: Some("SELECT JSON_GET_INT(properties, 'count') FROM nodes".into()),
        },
        KeywordInfo {
            keyword: "JSON_GET_BOOL".into(),
            category: KeywordCategory::JsonFunction,
            description: "Extract top-level key as boolean.".into(),
            syntax: Some("JSON_GET_BOOL(json, 'key')".into()),
            example: Some("SELECT * FROM nodes WHERE JSON_GET_BOOL(properties, 'active')".into()),
        },
        KeywordInfo {
            keyword: "TO_JSON".into(),
            category: KeywordCategory::JsonFunction,
            description: "Convert value or row to JSON.".into(),
            syntax: Some("TO_JSON(value)".into()),
            example: Some("SELECT TO_JSON(n) FROM nodes n".into()),
        },
        KeywordInfo {
            keyword: "TO_JSONB".into(),
            category: KeywordCategory::JsonFunction,
            description: "Convert value or row to JSONB (binary JSON).".into(),
            syntax: Some("TO_JSONB(value)".into()),
            example: Some("SELECT TO_JSONB(properties) FROM nodes".into()),
        },
    ]
}

/// Aggregate function keywords (COUNT, SUM, AVG, etc.)
pub(super) fn aggregate_function_keywords() -> Vec<KeywordInfo> {
    vec![
        KeywordInfo {
            keyword: "COUNT".into(),
            category: KeywordCategory::AggregateFunction,
            description: "Count rows or non-null values.".into(),
            syntax: Some("COUNT(*) | COUNT(column)".into()),
            example: Some("SELECT COUNT(*) FROM nodes".into()),
        },
        KeywordInfo {
            keyword: "SUM".into(),
            category: KeywordCategory::AggregateFunction,
            description: "Sum numeric values.".into(),
            syntax: Some("SUM(column)".into()),
            example: Some("SELECT SUM(JSON_GET_DOUBLE(properties, 'price')) FROM nodes".into()),
        },
        KeywordInfo {
            keyword: "AVG".into(),
            category: KeywordCategory::AggregateFunction,
            description: "Average of numeric values.".into(),
            syntax: Some("AVG(column)".into()),
            example: Some("SELECT AVG(JSON_GET_DOUBLE(properties, 'rating')) FROM nodes".into()),
        },
        KeywordInfo {
            keyword: "MIN".into(),
            category: KeywordCategory::AggregateFunction,
            description: "Minimum value.".into(),
            syntax: Some("MIN(column)".into()),
            example: Some("SELECT MIN(created_at) FROM nodes".into()),
        },
        KeywordInfo {
            keyword: "MAX".into(),
            category: KeywordCategory::AggregateFunction,
            description: "Maximum value.".into(),
            syntax: Some("MAX(column)".into()),
            example: Some("SELECT MAX(updated_at) FROM nodes".into()),
        },
        KeywordInfo {
            keyword: "ARRAY_AGG".into(),
            category: KeywordCategory::AggregateFunction,
            description: "Collect values into array.".into(),
            syntax: Some("ARRAY_AGG(column)".into()),
            example: Some("SELECT ARRAY_AGG(name) FROM nodes GROUP BY node_type".into()),
        },
    ]
}

/// Window function keywords (ROW_NUMBER, RANK, DENSE_RANK)
pub(super) fn window_function_keywords() -> Vec<KeywordInfo> {
    vec![
        KeywordInfo {
            keyword: "ROW_NUMBER".into(),
            category: KeywordCategory::WindowFunction,
            description: "Sequential row number within partition.".into(),
            syntax: Some("ROW_NUMBER() OVER ([PARTITION BY ...] ORDER BY ...)".into()),
            example: Some("SELECT name, ROW_NUMBER() OVER (ORDER BY created_at) FROM nodes".into()),
        },
        KeywordInfo {
            keyword: "RANK".into(),
            category: KeywordCategory::WindowFunction,
            description: "Rank with gaps for ties (1, 2, 2, 4).".into(),
            syntax: Some("RANK() OVER ([PARTITION BY ...] ORDER BY ...)".into()),
            example: Some("SELECT name, RANK() OVER (ORDER BY score DESC) FROM nodes".into()),
        },
        KeywordInfo {
            keyword: "DENSE_RANK".into(),
            category: KeywordCategory::WindowFunction,
            description: "Rank without gaps for ties (1, 2, 2, 3).".into(),
            syntax: Some("DENSE_RANK() OVER ([PARTITION BY ...] ORDER BY ...)".into()),
            example: Some("SELECT name, DENSE_RANK() OVER (ORDER BY score DESC) FROM nodes".into()),
        },
    ]
}
