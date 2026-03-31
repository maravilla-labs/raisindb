//! Property type and modifier keywords

use super::types::{KeywordCategory, KeywordInfo};

/// Property type keywords (String, Number, Boolean, etc.)
pub(super) fn property_type_keywords() -> Vec<KeywordInfo> {
    vec![
        KeywordInfo {
            keyword: "String".into(),
            category: KeywordCategory::PropertyType,
            description: "Text data type. Supports FULLTEXT and TRANSLATABLE modifiers.".into(),
            syntax: Some("name String [MODIFIERS]".into()),
            example: Some("title String REQUIRED FULLTEXT".into()),
        },
        KeywordInfo {
            keyword: "Number".into(),
            category: KeywordCategory::PropertyType,
            description: "Numeric data type (stored as f64). Supports PROPERTY_INDEX.".into(),
            syntax: Some("name Number [MODIFIERS]".into()),
            example: Some("price Number DEFAULT 0".into()),
        },
        KeywordInfo {
            keyword: "Boolean".into(),
            category: KeywordCategory::PropertyType,
            description: "True/false data type.".into(),
            syntax: Some("name Boolean [MODIFIERS]".into()),
            example: Some("active Boolean DEFAULT true".into()),
        },
        KeywordInfo {
            keyword: "Date".into(),
            category: KeywordCategory::PropertyType,
            description: "DateTime with ISO-8601 serialization.".into(),
            syntax: Some("name Date [MODIFIERS]".into()),
            example: Some("published_at Date".into()),
        },
        KeywordInfo {
            keyword: "URL".into(),
            category: KeywordCategory::PropertyType,
            description: "URL string type with validation.".into(),
            syntax: Some("name URL [MODIFIERS]".into()),
            example: Some("website URL".into()),
        },
        KeywordInfo {
            keyword: "Reference".into(),
            category: KeywordCategory::PropertyType,
            description: "Cross-node reference type. Creates relationship in graph.".into(),
            syntax: Some("name Reference [MODIFIERS]".into()),
            example: Some("author Reference".into()),
        },
        KeywordInfo {
            keyword: "Resource".into(),
            category: KeywordCategory::PropertyType,
            description: "File/media resource with metadata (URL, mime type, size).".into(),
            syntax: Some("name Resource [MODIFIERS]".into()),
            example: Some("image Resource REQUIRED".into()),
        },
        KeywordInfo {
            keyword: "Object".into(),
            category: KeywordCategory::PropertyType,
            description: "Nested object with inline field definitions.".into(),
            syntax: Some("name Object { field1 Type, field2 Type } [MODIFIERS]".into()),
            example: Some("seo Object { title String, description String }".into()),
        },
        KeywordInfo {
            keyword: "Array".into(),
            category: KeywordCategory::PropertyType,
            description: "Ordered collection type. Use with OF keyword.".into(),
            syntax: Some("name Array OF Type [MODIFIERS]".into()),
            example: Some("tags Array OF String".into()),
        },
        KeywordInfo {
            keyword: "Composite".into(),
            category: KeywordCategory::PropertyType,
            description: "Rich content structure with multiple Elements/blocks.".into(),
            syntax: Some("name Composite [MODIFIERS]".into()),
            example: Some("body Composite".into()),
        },
        KeywordInfo {
            keyword: "Element".into(),
            category: KeywordCategory::PropertyType,
            description: "Single element reference for Composite content.".into(),
            syntax: Some("name Element [MODIFIERS]".into()),
            example: Some("hero Element".into()),
        },
        KeywordInfo {
            keyword: "NodeType".into(),
            category: KeywordCategory::PropertyType,
            description: "Reference to a NodeType definition.".into(),
            syntax: Some("name NodeType [MODIFIERS]".into()),
            example: Some("type_ref NodeType".into()),
        },
    ]
}

/// Property modifier keywords (REQUIRED, UNIQUE, FULLTEXT, etc.)
pub(super) fn modifier_keywords() -> Vec<KeywordInfo> {
    vec![
        KeywordInfo {
            keyword: "REQUIRED".into(),
            category: KeywordCategory::Modifier,
            description: "Property must have a value. Validation will fail if not provided.".into(),
            syntax: Some("name Type REQUIRED".into()),
            example: Some("title String REQUIRED".into()),
        },
        KeywordInfo {
            keyword: "UNIQUE".into(),
            category: KeywordCategory::Modifier,
            description: "Property value must be unique across all nodes of this type.".into(),
            syntax: Some("name Type UNIQUE".into()),
            example: Some("slug String REQUIRED UNIQUE".into()),
        },
        KeywordInfo {
            keyword: "FULLTEXT".into(),
            category: KeywordCategory::Modifier,
            description: "Enable full-text search indexing via Tantivy. Use FULLTEXT_MATCH() to search.".into(),
            syntax: Some("name String FULLTEXT".into()),
            example: Some("body String FULLTEXT".into()),
        },
        KeywordInfo {
            keyword: "VECTOR".into(),
            category: KeywordCategory::Modifier,
            description: "Enable vector embedding indexing for semantic search. Use KNN() for similarity search.".into(),
            syntax: Some("name Array OF Number VECTOR".into()),
            example: Some("embedding Array OF Number VECTOR".into()),
        },
        KeywordInfo {
            keyword: "PROPERTY_INDEX".into(),
            category: KeywordCategory::Modifier,
            description: "Create a RocksDB index for fast exact-match filtering in WHERE clauses.".into(),
            syntax: Some("name Type PROPERTY_INDEX".into()),
            example: Some("sku String PROPERTY_INDEX".into()),
        },
        KeywordInfo {
            keyword: "TRANSLATABLE".into(),
            category: KeywordCategory::Modifier,
            description: "Property supports i18n translations. Use locale filter in queries.".into(),
            syntax: Some("name String TRANSLATABLE".into()),
            example: Some("description String TRANSLATABLE".into()),
        },
        KeywordInfo {
            keyword: "DEFAULT".into(),
            category: KeywordCategory::Modifier,
            description: "Set a default value when property is not provided.".into(),
            syntax: Some("name Type DEFAULT value".into()),
            example: Some("status String DEFAULT 'draft'".into()),
        },
        KeywordInfo {
            keyword: "LABEL".into(),
            category: KeywordCategory::Modifier,
            description: "Human-readable label for UI display. Stored in property metadata.".into(),
            syntax: Some("name Type LABEL 'Display Label'".into()),
            example: Some("title String LABEL 'Article Title'".into()),
        },
        KeywordInfo {
            keyword: "ORDER".into(),
            category: KeywordCategory::Modifier,
            description: "Display order hint for UI forms. Lower numbers appear first.".into(),
            syntax: Some("name Type ORDER number".into()),
            example: Some("title String ORDER 1".into()),
        },
        KeywordInfo {
            keyword: "ALLOW_ADDITIONAL_PROPERTIES".into(),
            category: KeywordCategory::Modifier,
            description: "For Object types: allow properties not defined in schema.".into(),
            syntax: Some("name Object { ... } ALLOW_ADDITIONAL_PROPERTIES".into()),
            example: Some("meta Object {} ALLOW_ADDITIONAL_PROPERTIES".into()),
        },
    ]
}

/// NodeType flag keywords (VERSIONABLE, PUBLISHABLE, etc.)
pub(super) fn flag_keywords() -> Vec<KeywordInfo> {
    vec![
        KeywordInfo {
            keyword: "VERSIONABLE".into(),
            category: KeywordCategory::Flag,
            description: "Enable version history. Each edit creates a new revision.".into(),
            syntax: Some("CREATE NODETYPE '...' ... VERSIONABLE".into()),
            example: Some("CREATE NODETYPE 'cms:Article' PROPERTIES (...) VERSIONABLE".into()),
        },
        KeywordInfo {
            keyword: "PUBLISHABLE".into(),
            category: KeywordCategory::Flag,
            description: "Enable publish workflow. Content has draft/published states.".into(),
            syntax: Some("CREATE NODETYPE '...' ... PUBLISHABLE".into()),
            example: Some("CREATE NODETYPE 'cms:Article' PROPERTIES (...) PUBLISHABLE".into()),
        },
        KeywordInfo {
            keyword: "AUDITABLE".into(),
            category: KeywordCategory::Flag,
            description: "Track all changes with user and timestamp.".into(),
            syntax: Some("CREATE NODETYPE '...' ... AUDITABLE".into()),
            example: Some("CREATE NODETYPE 'cms:Article' PROPERTIES (...) AUDITABLE".into()),
        },
        KeywordInfo {
            keyword: "INDEXABLE".into(),
            category: KeywordCategory::Flag,
            description: "Include nodes in search indexes. Default is true.".into(),
            syntax: Some("CREATE NODETYPE '...' ... INDEXABLE".into()),
            example: Some("CREATE NODETYPE 'cms:Article' PROPERTIES (...) INDEXABLE".into()),
        },
        KeywordInfo {
            keyword: "STRICT".into(),
            category: KeywordCategory::Flag,
            description: "Reject unknown properties. Validates against schema strictly.".into(),
            syntax: Some("CREATE NODETYPE '...' ... STRICT".into()),
            example: Some("CREATE NODETYPE 'cms:Article' PROPERTIES (...) STRICT".into()),
        },
    ]
}

/// Operator keywords (OF, CASCADE, ADD, etc.)
pub(super) fn operator_keywords() -> Vec<KeywordInfo> {
    vec![
        KeywordInfo {
            keyword: "ON".into(),
            category: KeywordCategory::Operator,
            description: "Specifies columns for COMPOUND_INDEX definition".into(),
            syntax: Some("COMPOUND_INDEX 'name' ON (columns...)".into()),
            example: Some("COMPOUND_INDEX 'idx_name' ON (category, status)".into()),
        },
        KeywordInfo {
            keyword: "ASC".into(),
            category: KeywordCategory::Operator,
            description: "Ascending sort order for ordering column in compound index".into(),
            syntax: Some("column ASC".into()),
            example: Some("COMPOUND_INDEX 'idx' ON (status, created_at ASC)".into()),
        },
        KeywordInfo {
            keyword: "DESC".into(),
            category: KeywordCategory::Operator,
            description: "Descending sort order for ordering column in compound index".into(),
            syntax: Some("column DESC".into()),
            example: Some("COMPOUND_INDEX 'idx' ON (status, created_at DESC)".into()),
        },
        KeywordInfo {
            keyword: "OF".into(),
            category: KeywordCategory::Operator,
            description: "Specifies the item type for Array properties.".into(),
            syntax: Some("Array OF Type".into()),
            example: Some("tags Array OF String".into()),
        },
        KeywordInfo {
            keyword: "CASCADE".into(),
            category: KeywordCategory::Operator,
            description: "When dropping, also remove dependent objects and data.".into(),
            syntax: Some("DROP ... CASCADE".into()),
            example: Some("DROP NODETYPE 'myapp:OldType' CASCADE".into()),
        },
        KeywordInfo {
            keyword: "ADD".into(),
            category: KeywordCategory::Operator,
            description: "Add a new property, field, or mixin.".into(),
            syntax: Some("ALTER ... ADD PROPERTY|FIELD|MIXIN ...".into()),
            example: Some("ALTER NODETYPE 'myapp:Article' ADD PROPERTY subtitle String".into()),
        },
        KeywordInfo {
            keyword: "MODIFY".into(),
            category: KeywordCategory::Operator,
            description: "Modify an existing property or field.".into(),
            syntax: Some("ALTER ... MODIFY PROPERTY|FIELD name Type [MODIFIERS]".into()),
            example: Some(
                "ALTER NODETYPE 'myapp:Article' MODIFY PROPERTY title String REQUIRED".into(),
            ),
        },
        KeywordInfo {
            keyword: "SET".into(),
            category: KeywordCategory::Operator,
            description: "Set a type attribute (DESCRIPTION, ICON, flags).".into(),
            syntax: Some("ALTER ... SET attribute = value".into()),
            example: Some("ALTER NODETYPE 'myapp:Article' SET DESCRIPTION = 'Updated'".into()),
        },
        KeywordInfo {
            keyword: "PROPERTY".into(),
            category: KeywordCategory::Operator,
            description: "References a property in ALTER statements.".into(),
            syntax: Some("ADD|DROP|MODIFY PROPERTY name ...".into()),
            example: Some("ADD PROPERTY subtitle String".into()),
        },
        KeywordInfo {
            keyword: "FIELD".into(),
            category: KeywordCategory::Operator,
            description: "References a field in ALTER ARCHETYPE/ELEMENTTYPE.".into(),
            syntax: Some("ADD|DROP|MODIFY FIELD name ...".into()),
            example: Some("ADD FIELD heading String".into()),
        },
        KeywordInfo {
            keyword: "MIXIN".into(),
            category: KeywordCategory::Operator,
            description: "References a mixin in ALTER statements.".into(),
            syntax: Some("ADD|DROP MIXIN 'mixin-name'".into()),
            example: Some("ADD MIXIN 'cms:Publishable'".into()),
        },
        KeywordInfo {
            keyword: "NULL".into(),
            category: KeywordCategory::Operator,
            description: "Null value in default clauses or comparisons.".into(),
            syntax: Some("DEFAULT NULL".into()),
            example: Some("optional_field String DEFAULT NULL".into()),
        },
    ]
}
