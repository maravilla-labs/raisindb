//! DDL statement, schema object, and clause keywords

use super::types::{KeywordCategory, KeywordInfo};

/// DDL statement keywords (CREATE, ALTER, DROP)
pub(super) fn statement_keywords() -> Vec<KeywordInfo> {
    vec![
        KeywordInfo {
            keyword: "CREATE".into(),
            category: KeywordCategory::Statement,
            description: "Creates a new schema object (NODETYPE, ARCHETYPE, or ELEMENTTYPE)".into(),
            syntax: Some("CREATE NODETYPE|ARCHETYPE|ELEMENTTYPE 'name' ...".into()),
            example: Some(
                "CREATE NODETYPE 'myapp:Article' PROPERTIES (title String REQUIRED)".into(),
            ),
        },
        KeywordInfo {
            keyword: "ALTER".into(),
            category: KeywordCategory::Statement,
            description: "Modifies an existing schema object".into(),
            syntax: Some(
                "ALTER NODETYPE|ARCHETYPE|ELEMENTTYPE 'name' ADD|DROP|MODIFY|SET ...".into(),
            ),
            example: Some(
                "ALTER NODETYPE 'myapp:Article' ADD PROPERTY subtitle String FULLTEXT".into(),
            ),
        },
        KeywordInfo {
            keyword: "DROP".into(),
            category: KeywordCategory::Statement,
            description: "Removes a schema object".into(),
            syntax: Some("DROP NODETYPE|ARCHETYPE|ELEMENTTYPE 'name' [CASCADE]".into()),
            example: Some("DROP NODETYPE 'myapp:OldType' CASCADE".into()),
        },
    ]
}

/// Schema object type keywords (NODETYPE, ARCHETYPE, ELEMENTTYPE)
pub(super) fn schema_object_keywords() -> Vec<KeywordInfo> {
    vec![
        KeywordInfo {
            keyword: "NODETYPE".into(),
            category: KeywordCategory::SchemaObject,
            description: "Defines a node type schema with properties, inheritance, and behavior flags".into(),
            syntax: Some("CREATE NODETYPE 'namespace:Name' [EXTENDS 'parent'] PROPERTIES (...)".into()),
            example: Some("CREATE NODETYPE 'cms:Article' EXTENDS 'raisin:Page' PROPERTIES (title String REQUIRED)".into()),
        },
        KeywordInfo {
            keyword: "ARCHETYPE".into(),
            category: KeywordCategory::SchemaObject,
            description: "Defines a content archetype (pre-configured template) based on a node type".into(),
            syntax: Some("CREATE ARCHETYPE 'name' BASE_NODE_TYPE 'type' FIELDS (...)".into()),
            example: Some("CREATE ARCHETYPE 'blog-post' BASE_NODE_TYPE 'cms:Article' TITLE 'Blog Post'".into()),
        },
        KeywordInfo {
            keyword: "ELEMENTTYPE".into(),
            category: KeywordCategory::SchemaObject,
            description: "Defines a reusable element type for composite content blocks".into(),
            syntax: Some("CREATE ELEMENTTYPE 'namespace:Name' FIELDS (...)".into()),
            example: Some("CREATE ELEMENTTYPE 'ui:HeroBanner' FIELDS (heading String REQUIRED, image Resource)".into()),
        },
    ]
}

/// DDL clause keywords (EXTENDS, PROPERTIES, FIELDS, etc.)
pub(super) fn clause_keywords() -> Vec<KeywordInfo> {
    vec![
        KeywordInfo {
            keyword: "EXTENDS".into(),
            category: KeywordCategory::Clause,
            description: "Inherits properties and behavior from a parent type".into(),
            syntax: Some("EXTENDS 'namespace:ParentType'".into()),
            example: Some("CREATE NODETYPE 'myapp:Article' EXTENDS 'raisin:Page'".into()),
        },
        KeywordInfo {
            keyword: "MIXINS".into(),
            category: KeywordCategory::Clause,
            description: "Includes additional mixin types for composition".into(),
            syntax: Some("MIXINS ('mixin1', 'mixin2')".into()),
            example: Some("CREATE NODETYPE 'myapp:Article' MIXINS ('myapp:Publishable', 'myapp:SEO')".into()),
        },
        KeywordInfo {
            keyword: "PROPERTIES".into(),
            category: KeywordCategory::Clause,
            description: "Defines the properties (fields) of a node type".into(),
            syntax: Some("PROPERTIES (name Type [MODIFIERS], ...)".into()),
            example: Some("PROPERTIES (title String REQUIRED FULLTEXT, slug String REQUIRED UNIQUE)".into()),
        },
        KeywordInfo {
            keyword: "FIELDS".into(),
            category: KeywordCategory::Clause,
            description: "Defines fields for archetypes and element types".into(),
            syntax: Some("FIELDS (name Type [MODIFIERS], ...)".into()),
            example: Some("FIELDS (heading String REQUIRED, image Resource)".into()),
        },
        KeywordInfo {
            keyword: "ALLOWED_CHILDREN".into(),
            category: KeywordCategory::Clause,
            description: "Restricts which node types can be children of this type".into(),
            syntax: Some("ALLOWED_CHILDREN ('type1', 'type2')".into()),
            example: Some("ALLOWED_CHILDREN ('cms:Paragraph', 'cms:Image')".into()),
        },
        KeywordInfo {
            keyword: "REQUIRED_NODES".into(),
            category: KeywordCategory::Clause,
            description: "Specifies node types that must exist as children".into(),
            syntax: Some("REQUIRED_NODES ('type1', 'type2')".into()),
            example: Some("REQUIRED_NODES ('cms:MetaData')".into()),
        },
        KeywordInfo {
            keyword: "COMPOUND_INDEX".into(),
            category: KeywordCategory::Clause,
            description: "Define a compound index for efficient ORDER BY + filter queries".into(),
            syntax: Some("COMPOUND_INDEX 'name' ON (col1, col2, col3 DESC)".into()),
            example: Some("COMPOUND_INDEX 'idx_category_status_created' ON (category, status, __created_at DESC)".into()),
        },
        KeywordInfo {
            keyword: "BASE_NODE_TYPE".into(),
            category: KeywordCategory::Clause,
            description: "Specifies the underlying node type for an archetype".into(),
            syntax: Some("BASE_NODE_TYPE 'namespace:NodeType'".into()),
            example: Some("CREATE ARCHETYPE 'blog' BASE_NODE_TYPE 'cms:Article'".into()),
        },
        KeywordInfo {
            keyword: "DESCRIPTION".into(),
            category: KeywordCategory::Clause,
            description: "Human-readable description of the schema object or property".into(),
            syntax: Some("DESCRIPTION 'description text'".into()),
            example: Some("DESCRIPTION 'Blog article content type'".into()),
        },
        KeywordInfo {
            keyword: "TITLE".into(),
            category: KeywordCategory::Clause,
            description: "Display title for archetypes".into(),
            syntax: Some("TITLE 'Display Title'".into()),
            example: Some("TITLE 'Blog Post'".into()),
        },
        KeywordInfo {
            keyword: "ICON".into(),
            category: KeywordCategory::Clause,
            description: "Icon identifier for UI display".into(),
            syntax: Some("ICON 'icon-name'".into()),
            example: Some("ICON 'article'".into()),
        },
    ]
}
