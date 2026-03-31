//! TRANSLATE statement parser using nom combinators
//!
//! Parses translation-aware UPDATE statements:
//! - UPDATE Page FOR LOCALE 'de' SET title = 'Titel' WHERE path = '/post'
//! - UPDATE Page FOR LOCALE 'fr' SET metadata.author = 'Jean' WHERE id = 'abc'
//! - UPDATE Page FOR LOCALE 'de' SET blocks[uuid='550e8400'].text = 'Hallo' WHERE path = '/post'

mod filters;
mod helpers;
mod parsers;

pub use parsers::{is_translate_statement, parse_translate, TranslateParseError};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::translate::{
        TranslateFilter, TranslateStatement, TranslationAssignment, TranslationPath,
        TranslationValue,
    };

    #[test]
    fn test_is_translate_statement() {
        assert!(is_translate_statement(
            "UPDATE Page FOR LOCALE 'de' SET title = 'Titel' WHERE path = '/post'"
        ));
        assert!(is_translate_statement(
            "update Article for locale 'fr' set name = 'Nom' where id = 'abc'"
        ));
        assert!(is_translate_statement(
            "  UPDATE Page FOR LOCALE 'de' SET title = 'X'  "
        ));
    }

    #[test]
    fn test_is_not_translate_statement() {
        assert!(!is_translate_statement("UPDATE Page SET title = 'X'")); // No FOR LOCALE
        assert!(!is_translate_statement("SELECT * FROM nodes"));
        assert!(!is_translate_statement("INSERT INTO nodes VALUES (1)"));
        assert!(!is_translate_statement(
            "ORDER Page SET path='/a' ABOVE path='/b'"
        ));
    }

    #[test]
    fn test_parse_simple_translation() {
        let sql = "UPDATE Page FOR LOCALE 'de' SET title = 'Titel' WHERE path = '/post'";
        let result = parse_translate(sql).unwrap().unwrap();

        assert_eq!(result.table, "Page");
        assert_eq!(result.locale, "de");
        assert_eq!(result.assignments.len(), 1);
        assert_eq!(
            result.assignments[0].path,
            TranslationPath::property(vec!["title".to_string()])
        );
        assert_eq!(
            result.assignments[0].value,
            TranslationValue::String("Titel".to_string())
        );
        assert_eq!(
            result.filter,
            Some(TranslateFilter::Path("/post".to_string()))
        );
    }

    #[test]
    fn test_parse_nested_property() {
        let sql = "UPDATE Page FOR LOCALE 'fr' SET metadata.author = 'Jean' WHERE id = 'abc'";
        let result = parse_translate(sql).unwrap().unwrap();

        assert_eq!(result.locale, "fr");
        assert_eq!(
            result.assignments[0].path,
            TranslationPath::property(vec!["metadata".to_string(), "author".to_string()])
        );
        assert_eq!(result.filter, Some(TranslateFilter::Id("abc".to_string())));
    }

    #[test]
    fn test_parse_block_translation() {
        let sql =
            "UPDATE Page FOR LOCALE 'de' SET blocks[uuid='550e8400'].content.text = 'Hallo' WHERE path = '/post'";
        let result = parse_translate(sql).unwrap().unwrap();

        assert_eq!(
            result.assignments[0].path,
            TranslationPath::BlockProperty {
                array_field: "blocks".to_string(),
                block_uuid: "550e8400".to_string(),
                property_path: vec!["content".to_string(), "text".to_string()],
            }
        );
    }

    #[test]
    fn test_parse_multiple_assignments() {
        let sql = "UPDATE Article FOR LOCALE 'es' SET title = 'Título', subtitle = 'Subtítulo' WHERE path = '/post'";
        let result = parse_translate(sql).unwrap().unwrap();

        assert_eq!(result.assignments.len(), 2);
        assert_eq!(
            result.assignments[0].path,
            TranslationPath::property(vec!["title".to_string()])
        );
        assert_eq!(
            result.assignments[1].path,
            TranslationPath::property(vec!["subtitle".to_string()])
        );
    }

    #[test]
    fn test_parse_node_type_filter() {
        let sql =
            "UPDATE BlogPost FOR LOCALE 'de' SET footer = 'Fußzeile' WHERE node_type = 'BlogPost'";
        let result = parse_translate(sql).unwrap().unwrap();

        assert_eq!(
            result.filter,
            Some(TranslateFilter::NodeType("BlogPost".to_string()))
        );
    }

    #[test]
    fn test_parse_path_and_type_filter() {
        let sql = "UPDATE Article FOR LOCALE 'de' SET title = 'X' WHERE path = '/post' AND node_type = 'Article'";
        let result = parse_translate(sql).unwrap().unwrap();

        assert_eq!(
            result.filter,
            Some(TranslateFilter::PathAndType {
                path: "/post".to_string(),
                node_type: "Article".to_string()
            })
        );
    }

    #[test]
    fn test_parse_case_insensitive() {
        let sql = "update Page for LOCALE 'de' set Title = 'Titel' where PATH = '/post'";
        let result = parse_translate(sql).unwrap().unwrap();

        assert_eq!(result.table, "Page");
        assert_eq!(result.locale, "de");
    }

    #[test]
    fn test_parse_with_semicolon() {
        let sql = "UPDATE Page FOR LOCALE 'de' SET title = 'Titel' WHERE path = '/post';";
        let result = parse_translate(sql);
        assert!(result.is_ok());
        assert!(result.unwrap().is_some());
    }

    #[test]
    fn test_parse_with_double_quotes() {
        let sql = r#"UPDATE Page FOR LOCALE "de" SET title = "Titel" WHERE path = "/post""#;
        let result = parse_translate(sql).unwrap().unwrap();

        assert_eq!(result.locale, "de");
        assert_eq!(
            result.assignments[0].value,
            TranslationValue::String("Titel".to_string())
        );
    }

    #[test]
    fn test_parse_integer_value() {
        let sql = "UPDATE Page FOR LOCALE 'de' SET order = 42 WHERE path = '/post'";
        let result = parse_translate(sql).unwrap().unwrap();

        assert_eq!(result.assignments[0].value, TranslationValue::Integer(42));
    }

    #[test]
    fn test_parse_float_value() {
        let sql = "UPDATE Page FOR LOCALE 'de' SET rating = 4.5 WHERE path = '/post'";
        let result = parse_translate(sql).unwrap().unwrap();

        assert_eq!(result.assignments[0].value, TranslationValue::Float(4.5));
    }

    #[test]
    fn test_parse_boolean_value() {
        let sql = "UPDATE Page FOR LOCALE 'de' SET featured = true WHERE path = '/post'";
        let result = parse_translate(sql).unwrap().unwrap();

        assert_eq!(result.assignments[0].value, TranslationValue::Boolean(true));
    }

    #[test]
    fn test_parse_null_value() {
        let sql = "UPDATE Page FOR LOCALE 'de' SET subtitle = NULL WHERE path = '/post'";
        let result = parse_translate(sql).unwrap().unwrap();

        assert_eq!(result.assignments[0].value, TranslationValue::Null);
    }

    #[test]
    fn test_parse_non_translate_statement() {
        let sql = "SELECT * FROM nodes";
        let result = parse_translate(sql).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_regular_update_not_matched() {
        let sql = "UPDATE Page SET title = 'X' WHERE path = '/post'";
        let result = parse_translate(sql).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_deep_nested_path() {
        let sql =
            "UPDATE Page FOR LOCALE 'de' SET seo.meta.description = 'Beschreibung' WHERE path = '/post'";
        let result = parse_translate(sql).unwrap().unwrap();

        assert_eq!(
            result.assignments[0].path,
            TranslationPath::property(vec![
                "seo".to_string(),
                "meta".to_string(),
                "description".to_string()
            ])
        );
        assert_eq!(
            result.assignments[0].path.to_json_pointer(),
            Some("/seo/meta/description".to_string())
        );
    }

    #[test]
    fn test_parse_block_with_nested_property() {
        let sql = "UPDATE Page FOR LOCALE 'de' SET sections[uuid='abc123'].header.title = 'Titel' WHERE path = '/post'";
        let result = parse_translate(sql).unwrap().unwrap();

        match &result.assignments[0].path {
            TranslationPath::BlockProperty {
                array_field,
                block_uuid,
                property_path,
            } => {
                assert_eq!(array_field, "sections");
                assert_eq!(block_uuid, "abc123");
                assert_eq!(
                    property_path,
                    &vec!["header".to_string(), "title".to_string()]
                );
            }
            _ => panic!("Expected BlockProperty"),
        }
    }

    #[test]
    fn test_parse_mixed_node_and_block_translations() {
        let sql = "UPDATE Page FOR LOCALE 'de' SET title = 'Titel', blocks[uuid='xyz'].text = 'Hallo' WHERE path = '/post'";
        let result = parse_translate(sql).unwrap().unwrap();

        assert_eq!(result.assignments.len(), 2);
        assert!(result.assignments[0].path.is_property());
        assert!(result.assignments[1].path.is_block_property());
    }

    #[test]
    fn test_translate_statement_display() {
        let stmt = TranslateStatement::new(
            "Page",
            "de",
            vec![TranslationAssignment::new(
                TranslationPath::property(vec!["title".to_string()]),
                TranslationValue::String("Titel".to_string()),
            )],
            Some(TranslateFilter::path("/post")),
        );

        assert_eq!(
            stmt.to_string(),
            "UPDATE Page FOR LOCALE 'de' SET title = 'Titel' WHERE path = '/post'"
        );
    }

    #[test]
    fn test_parse_with_in_branch() {
        let sql = "UPDATE social FOR LOCALE 'de' IN BRANCH 'localetest' SET bio = 'BIO IN DEUTSCH' WHERE path = '/users/senol'";
        let result = parse_translate(sql).unwrap().unwrap();

        assert_eq!(result.table, "social");
        assert_eq!(result.locale, "de");
        assert_eq!(result.branch, Some("localetest".to_string()));
        assert_eq!(result.assignments.len(), 1);
        assert_eq!(
            result.assignments[0].path,
            TranslationPath::property(vec!["bio".to_string()])
        );
    }

    #[test]
    fn test_parse_with_in_branch_multiline() {
        let sql = "UPDATE social FOR LOCALE 'de' IN BRANCH 'localetest'
SET
  bio = 'BIO IN DEUTSCH VON SENOL'
WHERE path = '/users/senol'";
        let result = parse_translate(sql);
        println!("Result: {:?}", result);
        let stmt = result.unwrap().unwrap();

        assert_eq!(stmt.table, "social");
        assert_eq!(stmt.locale, "de");
        assert_eq!(stmt.branch, Some("localetest".to_string()));
        assert_eq!(stmt.assignments.len(), 1);
    }
}
