//! Tests for the DDL parser

use super::*;
use crate::ast::ddl::{
    DdlStatement, DefaultValue, IndexTypeDef, NodeTypeAlteration, PropertyTypeDef,
};

#[test]
fn test_parse_simple_nodetype() {
    let sql = "CREATE NODETYPE 'myapp:Article'";
    let result = parse_ddl(sql).unwrap().unwrap();
    match result {
        DdlStatement::CreateNodeType(create) => {
            assert_eq!(create.name, "myapp:Article");
        }
        _ => panic!("Expected CreateNodeType"),
    }
}

#[test]
fn test_parse_nodetype_with_extends() {
    let sql = "CREATE NODETYPE 'myapp:Article' EXTENDS 'raisin:Page'";
    let result = parse_ddl(sql).unwrap().unwrap();
    match result {
        DdlStatement::CreateNodeType(create) => {
            assert_eq!(create.name, "myapp:Article");
            assert_eq!(create.extends, Some("raisin:Page".to_string()));
        }
        _ => panic!("Expected CreateNodeType"),
    }
}

#[test]
fn test_parse_nodetype_with_properties() {
    let sql = r#"
            CREATE NODETYPE 'myapp:Article'
            PROPERTIES (
                title String REQUIRED FULLTEXT,
                slug String REQUIRED UNIQUE,
                status String DEFAULT 'draft'
            )
        "#;
    let result = parse_ddl(sql).unwrap().unwrap();
    match result {
        DdlStatement::CreateNodeType(create) => {
            assert_eq!(create.name, "myapp:Article");
            assert_eq!(create.properties.len(), 3);

            let title = &create.properties[0];
            assert_eq!(title.name, "title");
            assert!(title.required);
            assert!(title.index.contains(&IndexTypeDef::Fulltext));

            let slug = &create.properties[1];
            assert_eq!(slug.name, "slug");
            assert!(slug.required);
            assert!(slug.unique);

            let status = &create.properties[2];
            assert_eq!(status.name, "status");
            assert_eq!(
                status.default,
                Some(DefaultValue::String("draft".to_string()))
            );
        }
        _ => panic!("Expected CreateNodeType"),
    }
}

#[test]
fn test_parse_nodetype_with_nested_object() {
    let sql = r#"
            CREATE NODETYPE 'myapp:Article'
            PROPERTIES (
                seo_meta Object {
                    title String,
                    description String TRANSLATABLE,
                    keywords Array OF String
                }
            )
        "#;
    let result = parse_ddl(sql).unwrap().unwrap();
    match result {
        DdlStatement::CreateNodeType(create) => {
            assert_eq!(create.properties.len(), 1);
            match &create.properties[0].property_type {
                PropertyTypeDef::Object { fields } => {
                    assert_eq!(fields.len(), 3);
                    assert_eq!(fields[0].name, "title");
                    assert_eq!(fields[1].name, "description");
                    assert!(fields[1].translatable);
                    match &fields[2].property_type {
                        PropertyTypeDef::Array { items } => {
                            assert_eq!(**items, PropertyTypeDef::String);
                        }
                        _ => panic!("Expected Array type"),
                    }
                }
                _ => panic!("Expected Object type"),
            }
        }
        _ => panic!("Expected CreateNodeType"),
    }
}

#[test]
fn test_parse_nodetype_with_flags() {
    let sql = "CREATE NODETYPE 'myapp:Article' VERSIONABLE PUBLISHABLE AUDITABLE";
    let result = parse_ddl(sql).unwrap().unwrap();
    match result {
        DdlStatement::CreateNodeType(create) => {
            assert!(create.versionable);
            assert!(create.publishable);
            assert!(create.auditable);
        }
        _ => panic!("Expected CreateNodeType"),
    }
}

#[test]
fn test_parse_nodetype_with_allowed_children() {
    let sql = "CREATE NODETYPE 'myapp:Article' ALLOWED_CHILDREN ('myapp:Paragraph', 'myapp:Image')";
    let result = parse_ddl(sql).unwrap().unwrap();
    match result {
        DdlStatement::CreateNodeType(create) => {
            assert_eq!(
                create.allowed_children,
                vec!["myapp:Paragraph", "myapp:Image"]
            );
        }
        _ => panic!("Expected CreateNodeType"),
    }
}

#[test]
fn test_parse_complex_nodetype() {
    let sql = r#"
            CREATE NODETYPE 'myapp:Article'
            EXTENDS 'raisin:Page'
            MIXINS ('myapp:Publishable', 'myapp:SEO')
            DESCRIPTION 'Blog article content type'
            ICON 'article'
            PROPERTIES (
                title String REQUIRED FULLTEXT,
                slug String REQUIRED UNIQUE,
                status String DEFAULT 'draft',
                author Reference,
                published Boolean DEFAULT false,
                seo_meta Object {
                    title String,
                    description String TRANSLATABLE,
                    keywords Array OF String
                },
                tags Array OF String FULLTEXT,
                image Resource
            )
            ALLOWED_CHILDREN ('myapp:Paragraph', 'myapp:Image', 'myapp:Quote')
            PUBLISHABLE
            VERSIONABLE
            AUDITABLE;
        "#;
    let result = parse_ddl(sql).unwrap().unwrap();
    match result {
        DdlStatement::CreateNodeType(create) => {
            assert_eq!(create.name, "myapp:Article");
            assert_eq!(create.extends, Some("raisin:Page".to_string()));
            assert_eq!(create.mixins.len(), 2);
            assert_eq!(
                create.description,
                Some("Blog article content type".to_string())
            );
            assert_eq!(create.icon, Some("article".to_string()));
            assert_eq!(create.properties.len(), 8);
            assert_eq!(create.allowed_children.len(), 3);
            assert!(create.publishable);
            assert!(create.versionable);
            assert!(create.auditable);
        }
        _ => panic!("Expected CreateNodeType"),
    }
}

#[test]
fn test_parse_alter_nodetype() {
    let sql = r#"
            ALTER NODETYPE 'myapp:Article'
            ADD PROPERTY subtitle String FULLTEXT
            DROP PROPERTY legacy_field
            SET DESCRIPTION = 'Updated description'
        "#;
    let result = parse_ddl(sql).unwrap().unwrap();
    match result {
        DdlStatement::AlterNodeType(alter) => {
            assert_eq!(alter.name, "myapp:Article");
            assert_eq!(alter.alterations.len(), 3);
        }
        _ => panic!("Expected AlterNodeType"),
    }
}

#[test]
fn test_parse_drop_nodetype() {
    let sql = "DROP NODETYPE 'myapp:OldType' CASCADE";
    let result = parse_ddl(sql).unwrap().unwrap();
    match result {
        DdlStatement::DropNodeType(drop) => {
            assert_eq!(drop.name, "myapp:OldType");
            assert!(drop.cascade);
        }
        _ => panic!("Expected DropNodeType"),
    }
}

#[test]
fn test_parse_create_archetype() {
    let sql = r#"
            CREATE ARCHETYPE 'myapp:BlogPost'
            BASE_NODE_TYPE 'myapp:Article'
            TITLE 'Blog Post'
            DESCRIPTION 'Blog post archetype'
            FIELDS (
                title String REQUIRED,
                body Composite
            )
            PUBLISHABLE
        "#;
    let result = parse_ddl(sql).unwrap().unwrap();
    match result {
        DdlStatement::CreateArchetype(create) => {
            assert_eq!(create.name, "myapp:BlogPost");
            assert_eq!(create.base_node_type, Some("myapp:Article".to_string()));
            assert_eq!(create.title, Some("Blog Post".to_string()));
            assert_eq!(create.fields.len(), 2);
            assert!(create.publishable);
        }
        _ => panic!("Expected CreateArchetype"),
    }
}

#[test]
fn test_parse_create_elementtype() {
    let sql = r#"
            CREATE ELEMENTTYPE 'myapp:Paragraph'
            DESCRIPTION 'Rich text paragraph'
            FIELDS (
                text String REQUIRED TRANSLATABLE,
                style String
            )
        "#;
    let result = parse_ddl(sql).unwrap().unwrap();
    match result {
        DdlStatement::CreateElementType(create) => {
            assert_eq!(create.name, "myapp:Paragraph");
            assert_eq!(create.description, Some("Rich text paragraph".to_string()));
            assert_eq!(create.fields.len(), 2);
        }
        _ => panic!("Expected CreateElementType"),
    }
}

#[test]
fn test_non_ddl_statement() {
    let sql = "SELECT * FROM nodes";
    let result = parse_ddl(sql).unwrap();
    assert!(result.is_none());
}

#[test]
fn test_parse_number_defaults() {
    let sql = r#"
            CREATE NODETYPE 'test:Numbers'
            PROPERTIES (
                count Number DEFAULT 42,
                rate Number DEFAULT 3.14,
                negative Number DEFAULT -10
            )
        "#;
    let result = parse_ddl(sql).unwrap().unwrap();
    match result {
        DdlStatement::CreateNodeType(create) => {
            assert_eq!(
                create.properties[0].default,
                Some(DefaultValue::Number(42.0))
            );
            assert_eq!(
                create.properties[1].default,
                Some(DefaultValue::Number(3.14))
            );
            assert_eq!(
                create.properties[2].default,
                Some(DefaultValue::Number(-10.0))
            );
        }
        _ => panic!("Expected CreateNodeType"),
    }
}

#[test]
fn test_parse_boolean_defaults() {
    let sql = r#"
            CREATE NODETYPE 'test:Bools'
            PROPERTIES (
                active Boolean DEFAULT true,
                hidden Boolean DEFAULT false
            )
        "#;
    let result = parse_ddl(sql).unwrap().unwrap();
    match result {
        DdlStatement::CreateNodeType(create) => {
            assert_eq!(
                create.properties[0].default,
                Some(DefaultValue::Boolean(true))
            );
            assert_eq!(
                create.properties[1].default,
                Some(DefaultValue::Boolean(false))
            );
        }
        _ => panic!("Expected CreateNodeType"),
    }
}

#[test]
fn test_parse_with_leading_comment() {
    let sql = r#"
            -- This is a comment describing the node type
            CREATE NODETYPE 'myapp:Article'
            PROPERTIES (
                title String REQUIRED
            )
        "#;
    let result = parse_ddl(sql).unwrap().unwrap();
    match result {
        DdlStatement::CreateNodeType(create) => {
            assert_eq!(create.name, "myapp:Article");
            assert_eq!(create.properties.len(), 1);
        }
        _ => panic!("Expected CreateNodeType"),
    }
}

#[test]
fn test_parse_with_multiple_leading_comments() {
    let sql = r#"
            -- First comment
            -- Second comment
            /* Multi-line
               comment */
            CREATE NODETYPE 'myapp:Test'
        "#;
    let result = parse_ddl(sql).unwrap().unwrap();
    match result {
        DdlStatement::CreateNodeType(create) => {
            assert_eq!(create.name, "myapp:Test");
        }
        _ => panic!("Expected CreateNodeType"),
    }
}

#[test]
fn test_create_nodetype_with_parens() {
    let sql = r#"
            CREATE NODETYPE 'cms:Article' (
                PROPERTIES (
                    title String REQUIRED,
                    body String FULLTEXT
                )
                PUBLISHABLE
                VERSIONABLE
            )
        "#;
    let result = parse_ddl(sql).unwrap().unwrap();
    match result {
        DdlStatement::CreateNodeType(create) => {
            assert_eq!(create.name, "cms:Article");
            assert_eq!(create.properties.len(), 2);
            assert!(create.publishable);
            assert!(create.versionable);
        }
        _ => panic!("Expected CreateNodeType"),
    }
}

#[test]
fn test_create_archetype_with_parens() {
    let sql = r#"
            CREATE ARCHETYPE 'blog-post' (
                BASE_NODE_TYPE 'cms:Article'
                TITLE 'Blog Post'
                PUBLISHABLE
            )
        "#;
    let result = parse_ddl(sql).unwrap().unwrap();
    match result {
        DdlStatement::CreateArchetype(create) => {
            assert_eq!(create.name, "blog-post");
            assert_eq!(create.base_node_type, Some("cms:Article".to_string()));
            assert_eq!(create.title, Some("Blog Post".to_string()));
            assert!(create.publishable);
        }
        _ => panic!("Expected CreateArchetype"),
    }
}

#[test]
fn test_create_elementtype_with_parens() {
    let sql = r#"
            CREATE ELEMENTTYPE 'ui:Card' (
                FIELDS (
                    title String REQUIRED,
                    description String
                )
            )
        "#;
    let result = parse_ddl(sql).unwrap().unwrap();
    match result {
        DdlStatement::CreateElementType(create) => {
            assert_eq!(create.name, "ui:Card");
            assert_eq!(create.fields.len(), 2);
        }
        _ => panic!("Expected CreateElementType"),
    }
}

#[test]
fn test_create_nodetype_without_parens_still_works() {
    // Verify backward compatibility
    let sql = r#"
            CREATE NODETYPE 'cms:Article'
            PROPERTIES (title String REQUIRED)
            PUBLISHABLE
        "#;
    let result = parse_ddl(sql).unwrap().unwrap();
    match result {
        DdlStatement::CreateNodeType(create) => {
            assert_eq!(create.name, "cms:Article");
            assert_eq!(create.properties.len(), 1);
            assert!(create.publishable);
        }
        _ => panic!("Expected CreateNodeType"),
    }
}

// ==========================================================================
// Comprehensive Property Modifier Tests
// ==========================================================================

#[test]
fn test_property_with_label_and_description() {
    let sql = r#"
            CREATE NODETYPE 'cms:Article' (
                PROPERTIES (
                    title String REQUIRED LABEL 'Article Title' DESCRIPTION 'The main title of the article' ORDER 1,
                    body String FULLTEXT LABEL 'Content Body' ORDER 2
                )
            )
        "#;
    let result = parse_ddl(sql).unwrap().unwrap();
    match result {
        DdlStatement::CreateNodeType(create) => {
            assert_eq!(create.properties.len(), 2);

            let title = &create.properties[0];
            assert_eq!(title.name, "title");
            assert!(title.required);
            assert_eq!(title.label, Some("Article Title".to_string()));
            assert_eq!(
                title.description,
                Some("The main title of the article".to_string())
            );
            assert_eq!(title.order, Some(1));

            let body = &create.properties[1];
            assert_eq!(body.name, "body");
            assert_eq!(body.label, Some("Content Body".to_string()));
            assert_eq!(body.order, Some(2));
        }
        _ => panic!("Expected CreateNodeType"),
    }
}

#[test]
fn test_object_with_allow_additional_properties() {
    let sql = r#"
            CREATE NODETYPE 'cms:Page' (
                PROPERTIES (
                    meta Object {
                        title String LABEL 'SEO Title',
                        description String
                    } ALLOW_ADDITIONAL_PROPERTIES LABEL 'Metadata'
                )
            )
        "#;
    let result = parse_ddl(sql).unwrap().unwrap();
    match result {
        DdlStatement::CreateNodeType(create) => {
            let meta = &create.properties[0];
            assert_eq!(meta.name, "meta");
            assert!(meta.allow_additional_properties);
            assert_eq!(meta.label, Some("Metadata".to_string()));

            match &meta.property_type {
                PropertyTypeDef::Object { fields } => {
                    assert_eq!(fields.len(), 2);
                    assert_eq!(fields[0].label, Some("SEO Title".to_string()));
                }
                _ => panic!("Expected Object type"),
            }
        }
        _ => panic!("Expected CreateNodeType"),
    }
}

// ==========================================================================
// Deep Nesting Tests
// ==========================================================================

#[test]
fn test_deep_nested_objects() {
    let sql = r#"
            CREATE NODETYPE 'cms:ComplexPage' (
                PROPERTIES (
                    seo Object {
                        basic Object {
                            title String REQUIRED LABEL 'SEO Title',
                            description String TRANSLATABLE LABEL 'SEO Description'
                        },
                        social Object {
                            og_title String LABEL 'Open Graph Title',
                            og_image Resource LABEL 'OG Image',
                            twitter Object {
                                card_type String DEFAULT 'summary_large_image',
                                site String DEFAULT '@mysite'
                            }
                        },
                        advanced Object {
                            canonical_url URL,
                            robots String DEFAULT 'index,follow',
                            schema_org Object {
                                type String DEFAULT 'Article',
                                author Reference
                            }
                        }
                    } LABEL 'SEO Configuration' DESCRIPTION 'Search engine optimization settings'
                )
            )
        "#;
    let result = parse_ddl(sql).unwrap().unwrap();
    match result {
        DdlStatement::CreateNodeType(create) => {
            assert_eq!(create.properties.len(), 1);
            let seo = &create.properties[0];
            assert_eq!(seo.name, "seo");
            assert_eq!(seo.label, Some("SEO Configuration".to_string()));

            match &seo.property_type {
                PropertyTypeDef::Object { fields } => {
                    assert_eq!(fields.len(), 3); // basic, social, advanced

                    // Check basic has nested fields
                    match &fields[0].property_type {
                        PropertyTypeDef::Object {
                            fields: basic_fields,
                        } => {
                            assert_eq!(basic_fields.len(), 2);
                            assert!(basic_fields[0].required);
                            assert!(basic_fields[1].translatable);
                        }
                        _ => panic!("Expected Object type for 'basic'"),
                    }

                    // Check social.twitter is deeply nested
                    match &fields[1].property_type {
                        PropertyTypeDef::Object {
                            fields: social_fields,
                        } => {
                            assert_eq!(social_fields.len(), 3);
                            match &social_fields[2].property_type {
                                PropertyTypeDef::Object {
                                    fields: twitter_fields,
                                } => {
                                    assert_eq!(twitter_fields.len(), 2);
                                    assert_eq!(
                                        twitter_fields[0].default,
                                        Some(DefaultValue::String(
                                            "summary_large_image".to_string()
                                        ))
                                    );
                                }
                                _ => panic!("Expected Object type for 'twitter'"),
                            }
                        }
                        _ => panic!("Expected Object type for 'social'"),
                    }
                }
                _ => panic!("Expected Object type for 'seo'"),
            }
        }
        _ => panic!("Expected CreateNodeType"),
    }
}

#[test]
fn test_array_of_objects() {
    let sql = r#"
            CREATE NODETYPE 'cms:Gallery' (
                PROPERTIES (
                    items Array OF Object {
                        image Resource REQUIRED LABEL 'Image File',
                        caption String TRANSLATABLE,
                        alt_text String LABEL 'Alt Text' DESCRIPTION 'Accessibility text'
                    } LABEL 'Gallery Items'
                )
            )
        "#;
    let result = parse_ddl(sql).unwrap().unwrap();
    match result {
        DdlStatement::CreateNodeType(create) => {
            let items = &create.properties[0];
            assert_eq!(items.name, "items");
            assert_eq!(items.label, Some("Gallery Items".to_string()));

            match &items.property_type {
                PropertyTypeDef::Array { items: item_type } => match item_type.as_ref() {
                    PropertyTypeDef::Object { fields } => {
                        assert_eq!(fields.len(), 3);
                        assert!(fields[0].required);
                        assert_eq!(fields[0].label, Some("Image File".to_string()));
                        assert!(fields[1].translatable);
                    }
                    _ => panic!("Expected Object type for array items"),
                },
                _ => panic!("Expected Array type"),
            }
        }
        _ => panic!("Expected CreateNodeType"),
    }
}

#[test]
fn test_complex_nodetype_with_all_features() {
    let sql = r#"
            CREATE NODETYPE 'ecommerce:Product' (
                EXTENDS 'raisin:Node'
                MIXINS ('ecommerce:Purchasable', 'cms:Publishable')
                DESCRIPTION 'E-commerce product with full features'
                ICON 'shopping-cart'
                PROPERTIES (
                    name String REQUIRED FULLTEXT LABEL 'Product Name' ORDER 1,
                    sku String REQUIRED UNIQUE PROPERTY_INDEX LABEL 'SKU' ORDER 2,
                    price Number REQUIRED PROPERTY_INDEX LABEL 'Price' DEFAULT 0,
                    description String FULLTEXT TRANSLATABLE LABEL 'Description' ORDER 3,

                    media Object {
                        primary_image Resource REQUIRED LABEL 'Main Image',
                        gallery Array OF Resource LABEL 'Product Gallery',
                        videos Array OF Object {
                            url URL REQUIRED,
                            title String,
                            thumbnail Resource
                        }
                    } LABEL 'Media Assets' ORDER 4,

                    specifications Object {
                        dimensions Object {
                            width Number LABEL 'Width (cm)',
                            height Number LABEL 'Height (cm)',
                            depth Number LABEL 'Depth (cm)',
                            weight Number LABEL 'Weight (kg)'
                        },
                        material String,
                        color String,
                        custom_attributes Object {} ALLOW_ADDITIONAL_PROPERTIES
                    } LABEL 'Product Specifications' ORDER 5,

                    inventory Object {
                        quantity Number DEFAULT 0,
                        low_stock_threshold Number DEFAULT 10,
                        track_inventory Boolean DEFAULT true
                    } LABEL 'Inventory' ORDER 6,

                    seo Object {
                        title String LABEL 'SEO Title',
                        description String LABEL 'Meta Description',
                        keywords Array OF String
                    } LABEL 'SEO' ORDER 7,

                    category Reference LABEL 'Category',
                    related_products Array OF Reference LABEL 'Related Products',
                    tags Array OF String FULLTEXT LABEL 'Tags'
                )
                ALLOWED_CHILDREN ('ecommerce:Variant')
                VERSIONABLE
                PUBLISHABLE
                AUDITABLE
                INDEXABLE
            )
        "#;
    let result = parse_ddl(sql).unwrap().unwrap();
    match result {
        DdlStatement::CreateNodeType(create) => {
            assert_eq!(create.name, "ecommerce:Product");
            assert_eq!(create.extends, Some("raisin:Node".to_string()));
            assert_eq!(create.mixins.len(), 2);
            assert_eq!(
                create.description,
                Some("E-commerce product with full features".to_string())
            );
            assert_eq!(create.icon, Some("shopping-cart".to_string()));
            assert!(create.versionable);
            assert!(create.publishable);
            assert!(create.auditable);
            assert!(create.indexable);
            assert_eq!(create.allowed_children, vec!["ecommerce:Variant"]);

            // Check we have all properties
            assert!(create.properties.len() >= 10);

            // Check name property
            let name = &create.properties[0];
            assert_eq!(name.name, "name");
            assert!(name.required);
            assert!(name.index.contains(&IndexTypeDef::Fulltext));
            assert_eq!(name.order, Some(1));

            // Check sku has PROPERTY_INDEX
            let sku = &create.properties[1];
            assert!(sku.unique);
            assert!(sku.index.contains(&IndexTypeDef::Property));

            // Find specifications and check custom_attributes allows additional
            let specs = create
                .properties
                .iter()
                .find(|p| p.name == "specifications")
                .expect("specifications not found");
            match &specs.property_type {
                PropertyTypeDef::Object { fields } => {
                    let custom = fields
                        .iter()
                        .find(|f| f.name == "custom_attributes")
                        .expect("custom_attributes not found");
                    assert!(custom.allow_additional_properties);
                }
                _ => panic!("Expected Object"),
            }
        }
        _ => panic!("Expected CreateNodeType"),
    }
}

#[test]
fn test_all_property_types() {
    let sql = r#"
            CREATE NODETYPE 'test:AllTypes' (
                PROPERTIES (
                    str_field String,
                    num_field Number,
                    bool_field Boolean,
                    date_field Date,
                    url_field URL,
                    ref_field Reference,
                    res_field Resource,
                    comp_field Composite,
                    elem_field Element,
                    nodetype_field NodeType,
                    obj_field Object {
                        nested String
                    },
                    arr_str Array OF String,
                    arr_num Array OF Number,
                    arr_obj Array OF Object {
                        item_field String
                    }
                )
            )
        "#;
    let result = parse_ddl(sql).unwrap().unwrap();
    match result {
        DdlStatement::CreateNodeType(create) => {
            assert_eq!(create.properties.len(), 14);

            let types: Vec<_> = create
                .properties
                .iter()
                .map(|p| format!("{}", p.property_type))
                .collect();
            assert!(types.contains(&"String".to_string()));
            assert!(types.contains(&"Number".to_string()));
            assert!(types.contains(&"Boolean".to_string()));
            assert!(types.contains(&"Date".to_string()));
            assert!(types.contains(&"URL".to_string()));
            assert!(types.contains(&"Reference".to_string()));
            assert!(types.contains(&"Resource".to_string()));
            assert!(types.contains(&"Composite".to_string()));
            assert!(types.contains(&"Element".to_string()));
            assert!(types.contains(&"NodeType".to_string()));
            assert!(types.contains(&"Object".to_string()));
            assert!(types.contains(&"Array OF String".to_string()));
            assert!(types.contains(&"Array OF Number".to_string()));
            assert!(types.contains(&"Array OF Object".to_string()));
        }
        _ => panic!("Expected CreateNodeType"),
    }
}

// ==========================================================================
// Nested Path Alteration Tests
// ==========================================================================

#[test]
fn test_alter_nested_property_modify() {
    let sql = r#"
            ALTER NODETYPE 'ecommerce:Product'
            MODIFY PROPERTY 'specs.dimensions.width' Number LABEL 'Width (cm)'
        "#;
    let result = parse_ddl(sql).unwrap().unwrap();
    match result {
        DdlStatement::AlterNodeType(alter) => {
            assert_eq!(alter.name, "ecommerce:Product");
            assert_eq!(alter.alterations.len(), 1);

            match &alter.alterations[0] {
                NodeTypeAlteration::ModifyProperty(prop) => {
                    assert_eq!(prop.name, "specs.dimensions.width");
                    assert!(prop.is_nested_path());
                    assert_eq!(prop.path_segments(), vec!["specs", "dimensions", "width"]);
                    assert_eq!(prop.leaf_name(), "width");
                    assert_eq!(prop.label, Some("Width (cm)".to_string()));
                }
                _ => panic!("Expected ModifyProperty"),
            }
        }
        _ => panic!("Expected AlterNodeType"),
    }
}

#[test]
fn test_alter_nested_property_add() {
    let sql = r#"
            ALTER NODETYPE 'ecommerce:Product'
            ADD PROPERTY 'specs.dimensions.depth' Number LABEL 'Depth (cm)' DEFAULT 0
        "#;
    let result = parse_ddl(sql).unwrap().unwrap();
    match result {
        DdlStatement::AlterNodeType(alter) => {
            assert_eq!(alter.alterations.len(), 1);

            match &alter.alterations[0] {
                NodeTypeAlteration::AddProperty(prop) => {
                    assert_eq!(prop.name, "specs.dimensions.depth");
                    assert!(prop.is_nested_path());
                    assert_eq!(prop.label, Some("Depth (cm)".to_string()));
                    assert_eq!(prop.default, Some(DefaultValue::Number(0.0)));
                }
                _ => panic!("Expected AddProperty"),
            }
        }
        _ => panic!("Expected AlterNodeType"),
    }
}

#[test]
fn test_alter_nested_property_drop() {
    let sql = r#"
            ALTER NODETYPE 'ecommerce:Product'
            DROP PROPERTY 'specs.dimensions.legacy_field'
        "#;
    let result = parse_ddl(sql).unwrap().unwrap();
    match result {
        DdlStatement::AlterNodeType(alter) => {
            assert_eq!(alter.alterations.len(), 1);

            match &alter.alterations[0] {
                NodeTypeAlteration::DropProperty(name) => {
                    assert_eq!(name, "specs.dimensions.legacy_field");
                }
                _ => panic!("Expected DropProperty"),
            }
        }
        _ => panic!("Expected AlterNodeType"),
    }
}

#[test]
fn test_alter_deeply_nested_property() {
    let sql = r#"
            ALTER NODETYPE 'cms:Page'
            MODIFY PROPERTY 'seo.social.twitter.card_type' String DEFAULT 'summary'
            ADD PROPERTY 'seo.social.twitter.creator' String LABEL 'Twitter Creator'
            DROP PROPERTY 'seo.social.twitter.legacy'
        "#;
    let result = parse_ddl(sql).unwrap().unwrap();
    match result {
        DdlStatement::AlterNodeType(alter) => {
            assert_eq!(alter.alterations.len(), 3);

            // Check MODIFY
            match &alter.alterations[0] {
                NodeTypeAlteration::ModifyProperty(prop) => {
                    assert_eq!(prop.name, "seo.social.twitter.card_type");
                    assert_eq!(
                        prop.path_segments(),
                        vec!["seo", "social", "twitter", "card_type"]
                    );
                }
                _ => panic!("Expected ModifyProperty"),
            }

            // Check ADD
            match &alter.alterations[1] {
                NodeTypeAlteration::AddProperty(prop) => {
                    assert_eq!(prop.name, "seo.social.twitter.creator");
                    assert_eq!(prop.label, Some("Twitter Creator".to_string()));
                }
                _ => panic!("Expected AddProperty"),
            }

            // Check DROP
            match &alter.alterations[2] {
                NodeTypeAlteration::DropProperty(name) => {
                    assert_eq!(name, "seo.social.twitter.legacy");
                }
                _ => panic!("Expected DropProperty"),
            }
        }
        _ => panic!("Expected AlterNodeType"),
    }
}

#[test]
fn test_alter_mixed_simple_and_nested() {
    let sql = r#"
            ALTER NODETYPE 'cms:Article'
            ADD PROPERTY excerpt String FULLTEXT
            MODIFY PROPERTY 'seo.title' String LABEL 'SEO Title'
            DROP PROPERTY legacy_field
            DROP PROPERTY 'meta.old_setting'
        "#;
    let result = parse_ddl(sql).unwrap().unwrap();
    match result {
        DdlStatement::AlterNodeType(alter) => {
            assert_eq!(alter.alterations.len(), 4);

            // Simple ADD (no path)
            match &alter.alterations[0] {
                NodeTypeAlteration::AddProperty(prop) => {
                    assert_eq!(prop.name, "excerpt");
                    assert!(!prop.is_nested_path());
                }
                _ => panic!("Expected AddProperty"),
            }

            // Nested MODIFY
            match &alter.alterations[1] {
                NodeTypeAlteration::ModifyProperty(prop) => {
                    assert_eq!(prop.name, "seo.title");
                    assert!(prop.is_nested_path());
                }
                _ => panic!("Expected ModifyProperty"),
            }

            // Simple DROP
            match &alter.alterations[2] {
                NodeTypeAlteration::DropProperty(name) => {
                    assert_eq!(name, "legacy_field");
                    assert!(!name.contains('.'));
                }
                _ => panic!("Expected DropProperty"),
            }

            // Nested DROP
            match &alter.alterations[3] {
                NodeTypeAlteration::DropProperty(name) => {
                    assert_eq!(name, "meta.old_setting");
                    assert!(name.contains('.'));
                }
                _ => panic!("Expected DropProperty"),
            }
        }
        _ => panic!("Expected AlterNodeType"),
    }
}

#[test]
fn test_property_def_path_helpers() {
    use crate::ast::ddl::PropertyDef;

    let simple = PropertyDef {
        name: "title".to_string(),
        ..Default::default()
    };
    assert!(!simple.is_nested_path());
    assert_eq!(simple.path_segments(), vec!["title"]);
    assert_eq!(simple.leaf_name(), "title");

    let nested = PropertyDef {
        name: "specs.dimensions.width".to_string(),
        ..Default::default()
    };
    assert!(nested.is_nested_path());
    assert_eq!(nested.path_segments(), vec!["specs", "dimensions", "width"]);
    assert_eq!(nested.leaf_name(), "width");
}

// ==========================================================================
// Compound Index Tests
// ==========================================================================

#[test]
fn test_parse_compound_index_basic() {
    let sql = r#"
            CREATE NODETYPE 'news:Article'
            PROPERTIES (
                title String REQUIRED,
                category String,
                status String
            )
            COMPOUND_INDEX 'idx_category_status' ON (category, status)
        "#;
    let result = parse_ddl(sql).unwrap().unwrap();
    match result {
        DdlStatement::CreateNodeType(create) => {
            assert_eq!(create.compound_indexes.len(), 1);
            let idx = &create.compound_indexes[0];
            assert_eq!(idx.name, "idx_category_status");
            assert_eq!(idx.columns.len(), 2);
            assert_eq!(idx.columns[0].property, "category");
            assert!(idx.columns[0].ascending);
            assert_eq!(idx.columns[1].property, "status");
            assert!(idx.columns[1].ascending);
        }
        _ => panic!("Expected CreateNodeType"),
    }
}

#[test]
fn test_parse_compound_index_with_ordering() {
    let sql = r#"
            CREATE NODETYPE 'news:Article'
            PROPERTIES (
                category String,
                status String
            )
            COMPOUND_INDEX 'idx_category_status_created' ON (
                category,
                status,
                __created_at DESC
            )
        "#;
    let result = parse_ddl(sql).unwrap().unwrap();
    match result {
        DdlStatement::CreateNodeType(create) => {
            assert_eq!(create.compound_indexes.len(), 1);
            let idx = &create.compound_indexes[0];
            assert_eq!(idx.name, "idx_category_status_created");
            assert_eq!(idx.columns.len(), 3);
            assert_eq!(idx.columns[0].property, "category");
            assert!(idx.columns[0].ascending);
            assert_eq!(idx.columns[1].property, "status");
            assert!(idx.columns[1].ascending);
            assert_eq!(idx.columns[2].property, "__created_at");
            assert!(!idx.columns[2].ascending); // DESC
            assert!(idx.has_order_column);
        }
        _ => panic!("Expected CreateNodeType"),
    }
}

#[test]
fn test_parse_multiple_compound_indexes() {
    let sql = r#"
            CREATE NODETYPE 'news:Article'
            PROPERTIES (
                category String,
                author String,
                status String
            )
            COMPOUND_INDEX 'idx_category_created' ON (category, __created_at DESC)
            COMPOUND_INDEX 'idx_author_created' ON (author, __created_at DESC)
            PUBLISHABLE
        "#;
    let result = parse_ddl(sql).unwrap().unwrap();
    match result {
        DdlStatement::CreateNodeType(create) => {
            assert_eq!(create.compound_indexes.len(), 2);
            assert_eq!(create.compound_indexes[0].name, "idx_category_created");
            assert_eq!(create.compound_indexes[1].name, "idx_author_created");
            assert!(create.publishable);
        }
        _ => panic!("Expected CreateNodeType"),
    }
}

#[test]
fn test_parse_compound_index_with_node_type() {
    let sql = r#"
            CREATE NODETYPE 'news:Article'
            PROPERTIES (
                category String,
                status String
            )
            COMPOUND_INDEX 'idx_type_category_status_created' ON (
                __node_type,
                category,
                status,
                __created_at DESC
            )
        "#;
    let result = parse_ddl(sql).unwrap().unwrap();
    match result {
        DdlStatement::CreateNodeType(create) => {
            assert_eq!(create.compound_indexes.len(), 1);
            let idx = &create.compound_indexes[0];
            assert_eq!(idx.columns.len(), 4);
            assert_eq!(idx.columns[0].property, "__node_type");
            assert_eq!(idx.columns[1].property, "category");
            assert_eq!(idx.columns[2].property, "status");
            assert_eq!(idx.columns[3].property, "__created_at");
        }
        _ => panic!("Expected CreateNodeType"),
    }
}

#[test]
fn test_parse_compound_index_with_explicit_asc() {
    let sql = r#"
            CREATE NODETYPE 'news:Article'
            PROPERTIES (category String)
            COMPOUND_INDEX 'idx_category_created' ON (category, __created_at ASC)
        "#;
    let result = parse_ddl(sql).unwrap().unwrap();
    match result {
        DdlStatement::CreateNodeType(create) => {
            let idx = &create.compound_indexes[0];
            assert_eq!(idx.columns[1].property, "__created_at");
            assert!(idx.columns[1].ascending); // ASC
        }
        _ => panic!("Expected CreateNodeType"),
    }
}

#[test]
fn test_parse_compound_index_with_comments() {
    // This reproduces the issue from setup-db.sql where comments between
    // PROPERTIES and COMPOUND_INDEX cause parsing to fail
    let sql = r#"
CREATE NODETYPE 'news:Article' (
  PROPERTIES (
    title String REQUIRED FULLTEXT LABEL 'Title' ORDER 1,
    slug String REQUIRED PROPERTY_INDEX LABEL 'URL Slug' ORDER 2,
    category String PROPERTY_INDEX LABEL 'Category' ORDER 5,
    status String DEFAULT 'draft' PROPERTY_INDEX LABEL 'Status' ORDER 8
  )
  -- Compound index for efficient "related articles" queries:
  -- SELECT * FROM social WHERE node_type = 'news:Article'
  --   AND properties->>'category' = $1 AND properties->>'status' = 'published'
  -- ORDER BY created_at DESC LIMIT 3
  COMPOUND_INDEX 'idx_article_category_status_created' ON (
    __node_type,
    category,
    status,
    __created_at DESC
  )
  PUBLISHABLE
  INDEXABLE
)
        "#;
    let result = parse_ddl(sql).unwrap().unwrap();
    match result {
        DdlStatement::CreateNodeType(create) => {
            assert_eq!(create.name, "news:Article");
            assert_eq!(create.properties.len(), 4);
            assert_eq!(create.compound_indexes.len(), 1);
            let idx = &create.compound_indexes[0];
            assert_eq!(idx.name, "idx_article_category_status_created");
            assert_eq!(idx.columns.len(), 4);
            assert_eq!(idx.columns[0].property, "__node_type");
            assert_eq!(idx.columns[3].property, "__created_at");
            assert!(!idx.columns[3].ascending); // DESC
            assert!(create.publishable);
            assert!(create.indexable);
        }
        _ => panic!("Expected CreateNodeType"),
    }
}
