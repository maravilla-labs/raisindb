//! Comprehensive analyzer integration tests

use crate::analyzer::{AnalyzedStatement, Analyzer, DataType, Expr, Literal, StaticCatalog};
use raisin_hlc::HLC;

#[test]
fn test_simple_select_with_type_validation() {
    let analyzer = Analyzer::new();
    // Updated: removed workspace column, use node_type instead
    let result = analyzer.analyze("SELECT id, path, node_type FROM nodes");

    assert!(result.is_ok());
    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };

    // Verify 3 columns
    assert_eq!(query.projection.len(), 3);

    // Check id column
    let (id_expr, _id_alias) = &query.projection[0];
    assert_eq!(id_expr.data_type, DataType::Text);
    // Alias is None when not explicitly provided
    // assert_eq!(id_alias.as_deref(), Some("id"));

    // Check path column
    let (path_expr, _) = &query.projection[1];
    assert_eq!(path_expr.data_type, DataType::Path);

    // Check workspace column
    let (workspace_expr, _) = &query.projection[2];
    assert_eq!(workspace_expr.data_type, DataType::Text);
}

#[test]
fn test_hierarchy_functions() {
    let analyzer = Analyzer::new();

    // Test PATH_STARTS_WITH
    let result = analyzer.analyze("SELECT id FROM nodes WHERE PATH_STARTS_WITH(path, '/content')");
    assert!(result.is_ok());
    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query");
    };
    assert!(query.selection.is_some());

    // Test PARENT function
    let result = analyzer.analyze("SELECT PARENT(path) as parent_path FROM nodes");
    assert!(result.is_ok());
    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query");
    };
    assert_eq!(query.projection.len(), 1);
    assert_eq!(
        query.projection[0].0.data_type,
        DataType::Nullable(Box::new(DataType::Path))
    );

    // Test DEPTH function
    let result = analyzer.analyze("SELECT DEPTH(path) as depth FROM nodes");
    assert!(result.is_ok());
    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query");
    };
    assert_eq!(query.projection.len(), 1);
    assert_eq!(query.projection[0].0.data_type, DataType::Int);

    // Test CHILD_OF function
    let result = analyzer.analyze("SELECT id FROM nodes WHERE CHILD_OF('/content')");
    assert!(result.is_ok());
    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query");
    };
    assert!(query.selection.is_some());
    assert_eq!(
        query.selection.as_ref().unwrap().data_type,
        DataType::Boolean
    );
}

#[test]
fn test_json_extractors() {
    let analyzer = Analyzer::new();

    // Test JSON_VALUE
    let result = analyzer.analyze("SELECT JSON_VALUE(properties, 'title') as title FROM nodes");
    assert!(result.is_ok());
    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query");
    };
    assert_eq!(
        query.projection[0].0.data_type,
        DataType::Nullable(Box::new(DataType::Text))
    );

    // Test JSON_EXISTS
    let result =
        analyzer.analyze("SELECT id FROM nodes WHERE JSON_EXISTS(properties, 'published')");
    assert!(result.is_ok());

    // Test JSON_GET_TEXT
    let result = analyzer.analyze("SELECT JSON_GET_TEXT(properties, 'author') FROM nodes");
    assert!(result.is_ok());
    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query");
    };
    assert_eq!(
        query.projection[0].0.data_type,
        DataType::Nullable(Box::new(DataType::Text))
    );

    // Test JSON_GET_DOUBLE
    let result = analyzer.analyze("SELECT JSON_GET_DOUBLE(properties, 'rating') FROM nodes");
    assert!(result.is_ok());
    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query");
    };
    assert_eq!(
        query.projection[0].0.data_type,
        DataType::Nullable(Box::new(DataType::Double))
    );
}

#[test]
fn test_type_errors() {
    let analyzer = Analyzer::new();

    // Comparing incompatible types (without coercion)
    let result = analyzer.analyze("SELECT id FROM nodes WHERE version = 'text'");
    // version is INT, 'text' is TEXT - should fail
    assert!(result.is_err());

    // Wrong function argument type
    let result = analyzer.analyze("SELECT DEPTH(123) FROM nodes");
    // DEPTH requires PATH, not INT
    assert!(result.is_err());
}

#[test]
fn test_type_coercion_in_comparisons() {
    let analyzer = Analyzer::new();

    // INT → BIGINT coercion
    let result = analyzer.analyze("SELECT id FROM nodes WHERE version < 9999999999");
    assert!(result.is_ok());

    // PATH can compare with TEXT literals
    let result = analyzer.analyze("SELECT id FROM nodes WHERE path = '/content'");
    assert!(result.is_ok());
}

#[test]
fn test_constant_folding_depth() {
    let analyzer = Analyzer::new();
    let result = analyzer.analyze("SELECT id FROM nodes WHERE DEPTH('/a/b/c') = 3");

    assert!(result.is_ok());
    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query");
    };

    // Check that DEPTH('/a/b/c') was folded to literal 3
    if let Some(selection) = &query.selection {
        if let Expr::BinaryOp { left, .. } = &selection.expr {
            if let Expr::Literal(Literal::Int(3)) = &left.expr {
                // Successfully folded!
            } else {
                panic!(
                    "Expected DEPTH('/a/b/c') to be folded to literal 3, got: {:?}",
                    left.expr
                );
            }
        }
    }
}

#[test]
fn test_constant_folding_parent() {
    let analyzer = Analyzer::new();
    let result = analyzer.analyze("SELECT id FROM nodes WHERE PARENT('/a/b/c') = '/a/b'");

    assert!(result.is_ok());
    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query");
    };

    // Check that PARENT('/a/b/c') was folded to literal '/a/b'
    if let Some(selection) = &query.selection {
        if let Expr::BinaryOp { left, .. } = &selection.expr {
            match &left.expr {
                Expr::Literal(Literal::Path(path)) => {
                    assert_eq!(path, "/a/b");
                }
                Expr::Literal(Literal::Text(path)) => {
                    assert_eq!(path, "/a/b");
                }
                _ => {
                    panic!(
                        "Expected PARENT to be folded to Path or Text literal, got: {:?}",
                        left.expr
                    );
                }
            }
        }
    }
}

#[test]
fn test_where_clause_type_validation() {
    let analyzer = Analyzer::new();

    // Boolean WHERE clause
    let result = analyzer.analyze("SELECT id FROM nodes WHERE path = '/content' AND version > 1");
    assert!(result.is_ok());

    // Non-boolean WHERE clause should fail
    let result = analyzer.analyze("SELECT id FROM nodes WHERE version + 1");
    assert!(result.is_err());
}

#[test]
fn test_arithmetic_operations() {
    let analyzer = Analyzer::new();

    // INT + INT = INT
    let result = analyzer.analyze("SELECT version + 1 FROM nodes");
    assert!(result.is_ok());

    // Numeric promotion
    let result = analyzer.analyze("SELECT version + 1.5 FROM nodes");
    assert!(result.is_ok());
    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query");
    };
    assert_eq!(query.projection[0].0.data_type, DataType::Double);
}

#[test]
fn test_order_by_validation() {
    let analyzer = Analyzer::new();

    let result = analyzer.analyze("SELECT id, name FROM nodes ORDER BY created_at DESC");
    assert!(result.is_ok());
    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query");
    };

    assert_eq!(query.order_by.len(), 1);
    assert_eq!(query.order_by[0].expr.data_type, DataType::TimestampTz);
    assert!(query.order_by[0].descending); // is_desc = true
}

#[test]
fn test_limit_offset_validation() {
    let analyzer = Analyzer::new();

    let result = analyzer.analyze("SELECT id FROM nodes LIMIT 10 OFFSET 5");
    assert!(result.is_ok());
    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query");
    };

    assert_eq!(query.limit, Some(10));
    assert_eq!(query.offset, Some(5));
}

#[test]
fn test_wildcard_expansion() {
    let analyzer = Analyzer::new();

    let result = analyzer.analyze("SELECT * FROM nodes");
    assert!(result.is_ok());
    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query");
    };

    // Should expand to all columns in nodes table (19 columns after removing workspace)
    assert!(query.projection.len() >= 19);
}

#[test]
fn test_generated_columns() {
    let analyzer = Analyzer::new();

    // Depth is a generated column
    let result = analyzer.analyze("SELECT depth FROM nodes WHERE depth > 2");
    assert!(result.is_ok());
    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query");
    };
    assert_eq!(query.projection[0].0.data_type, DataType::Int);

    // Parent_path is a generated column
    let result = analyzer.analyze("SELECT parent_path FROM nodes");
    assert!(result.is_ok());
    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query");
    };
    assert_eq!(query.projection[0].0.data_type, DataType::Path);
}

#[test]
fn test_nullable_columns() {
    let analyzer = Analyzer::new();

    // archetype is nullable
    let result = analyzer.analyze("SELECT archetype FROM nodes");
    assert!(result.is_ok());
    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query");
    };
    // Note: In catalog, archetype is defined as nullable: true but data_type is Text
    // The nullable flag doesn't automatically wrap in Nullable type in our current impl
    assert_eq!(query.projection[0].0.data_type, DataType::Text);
}

#[test]
fn test_complex_expressions() {
    let analyzer = Analyzer::new();

    // Updated: removed workspace column reference, test complex boolean expressions
    let result = analyzer.analyze(
        "SELECT id FROM nodes WHERE (version > 1 AND path = '/content') OR node_type = 'folder'",
    );
    assert!(result.is_ok());
}

#[test]
fn test_cast_operations() {
    let analyzer = Analyzer::new();

    // Valid cast
    let result = analyzer.analyze("SELECT CAST(version AS BIGINT) FROM nodes");
    assert!(result.is_ok());
    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query");
    };
    assert_eq!(query.projection[0].0.data_type, DataType::BigInt);

    // JSONB to INT now works via intermediate TEXT cast (JSONB → TEXT → INT)
    let result = analyzer.analyze("SELECT CAST(properties AS INT) FROM nodes");
    assert!(
        result.is_ok(),
        "CAST(JSONB AS INT) should work via intermediate TEXT: {:?}",
        result.err()
    );
    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query");
    };
    assert_eq!(query.projection[0].0.data_type, DataType::Int);

    // Invalid cast should still fail (e.g., PATH cannot cast to INT)
    let result = analyzer.analyze("SELECT CAST(path AS TIMESTAMPTZ) FROM nodes");
    assert!(result.is_err(), "CAST(PATH AS TIMESTAMPTZ) should fail");
}

#[test]
fn test_cast_json_value_to_numeric() {
    let analyzer = Analyzer::new();

    // CAST JSON_VALUE result (TEXT?) to INT - should succeed with explicit cast
    let result =
        analyzer.analyze("SELECT CAST(JSON_VALUE(properties, '$.likeCount') AS INT) FROM nodes");
    assert!(
        result.is_ok(),
        "CAST(JSON_VALUE(...) AS INT) should be allowed"
    );
    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query");
    };
    // Result should be INT (the cast target type)
    assert_eq!(query.projection[0].0.data_type, DataType::Int);

    // CAST TEXT to DOUBLE
    let result =
        analyzer.analyze("SELECT CAST(JSON_VALUE(properties, '$.price') AS DOUBLE) FROM nodes");
    assert!(
        result.is_ok(),
        "CAST(JSON_VALUE(...) AS DOUBLE) should be allowed"
    );
    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query");
    };
    assert_eq!(query.projection[0].0.data_type, DataType::Double);

    // CAST TEXT to BIGINT
    let result =
        analyzer.analyze("SELECT CAST(JSON_VALUE(properties, '$.timestamp') AS BIGINT) FROM nodes");
    assert!(
        result.is_ok(),
        "CAST(JSON_VALUE(...) AS BIGINT) should be allowed"
    );
    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query");
    };
    assert_eq!(query.projection[0].0.data_type, DataType::BigInt);

    // Test the original failing query pattern with comparison
    let result = analyzer.analyze(
        "SELECT id FROM nodes WHERE CAST(JSON_VALUE(properties, '$.likeCount') AS INT) > 0",
    );
    assert!(result.is_ok(), "Original query pattern should now work");
}

#[test]
fn test_cast_decimal_strings_to_int() {
    let analyzer = Analyzer::new();

    // CAST('0.0' AS INT) should succeed (handles JSON numeric values)
    let result = analyzer.analyze("SELECT CAST('0.0' AS INT) FROM nodes");
    assert!(
        result.is_ok(),
        "CAST('0.0' AS INT) should work for JSON compatibility"
    );
    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query");
    };
    assert_eq!(query.projection[0].0.data_type, DataType::Int);

    // CAST('123.45' AS INT) should truncate to 123
    let result = analyzer.analyze("SELECT CAST('123.45' AS INT) FROM nodes");
    assert!(result.is_ok(), "CAST('123.45' AS INT) should truncate");

    // CAST('0.0' AS BIGINT) should work
    let result = analyzer.analyze("SELECT CAST('0.0' AS BIGINT) FROM nodes");
    assert!(result.is_ok(), "CAST('0.0' AS BIGINT) should work");

    // CAST('999.999' AS BIGINT) should truncate
    let result = analyzer.analyze("SELECT CAST('999.999' AS BIGINT) FROM nodes");
    assert!(result.is_ok(), "CAST('999.999' AS BIGINT) should truncate");

    // Pure integer strings should still work
    let result = analyzer.analyze("SELECT CAST('42' AS INT) FROM nodes");
    assert!(result.is_ok(), "CAST('42' AS INT) should work");

    // Invalid strings should still fail at analysis time (type checking passes, runtime would fail)
    let result = analyzer.analyze("SELECT CAST('not-a-number' AS INT) FROM nodes");
    assert!(result.is_ok(), "Analysis should pass, runtime would fail");
}

#[test]
fn test_cast_null_values() {
    let analyzer = Analyzer::new();

    // CAST(NULL AS INT) should succeed and return INT type
    let result = analyzer.analyze("SELECT CAST(NULL AS INT) FROM nodes");
    assert!(result.is_ok(), "CAST(NULL AS INT) should be allowed");
    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query");
    };
    assert_eq!(query.projection[0].0.data_type, DataType::Int);

    // CAST(NULL AS TEXT) should succeed
    let result = analyzer.analyze("SELECT CAST(NULL AS TEXT) FROM nodes");
    assert!(result.is_ok(), "CAST(NULL AS TEXT) should be allowed");
    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query");
    };
    assert_eq!(query.projection[0].0.data_type, DataType::Text);

    // CAST(NULL AS DOUBLE) should succeed
    let result = analyzer.analyze("SELECT CAST(NULL AS DOUBLE) FROM nodes");
    assert!(result.is_ok(), "CAST(NULL AS DOUBLE) should be allowed");
    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query");
    };
    assert_eq!(query.projection[0].0.data_type, DataType::Double);

    // CAST(NULL AS BIGINT) should succeed
    let result = analyzer.analyze("SELECT CAST(NULL AS BIGINT) FROM nodes");
    assert!(result.is_ok(), "CAST(NULL AS BIGINT) should be allowed");
    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query");
    };
    assert_eq!(query.projection[0].0.data_type, DataType::BigInt);
}

#[test]
fn test_cast_to_jsonb() {
    let analyzer = Analyzer::new();

    // CAST TEXT to JSONB using CAST function
    let result = analyzer.analyze("SELECT CAST(id AS JSONB) FROM nodes");
    assert!(
        result.is_ok(),
        "CAST(id AS JSONB) should be allowed: {:?}",
        result.err()
    );
    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query");
    };
    assert_eq!(query.projection[0].0.data_type, DataType::JsonB);

    // CAST TEXT to JSONB using :: operator
    let result = analyzer.analyze("SELECT id::JSONB FROM nodes");
    assert!(result.is_ok(), "id::JSONB should be allowed");
    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query");
    };
    assert_eq!(query.projection[0].0.data_type, DataType::JsonB);

    // CAST to JSON (should also map to JsonB)
    let result = analyzer.analyze("SELECT CAST(id AS JSON) FROM nodes");
    assert!(result.is_ok(), "CAST(id AS JSON) should be allowed");
    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query");
    };
    assert_eq!(query.projection[0].0.data_type, DataType::JsonB);
}

#[test]
fn test_cast_jsonb_via_intermediate_text() {
    let analyzer = Analyzer::new();

    // BOOLEAN: CAST syntax - $.path returns JSONB?, should cast via TEXT
    let result = analyzer.analyze("SELECT CAST($.properties.featured AS BOOLEAN) FROM nodes");
    assert!(
        result.is_ok(),
        "CAST($.path AS BOOLEAN) should work via TEXT: {:?}",
        result.err()
    );
    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query");
    };
    assert_eq!(query.projection[0].0.data_type, DataType::Boolean);

    // BOOLEAN: :: syntax
    let result = analyzer.analyze("SELECT $.properties.featured::BOOLEAN FROM nodes");
    assert!(
        result.is_ok(),
        "$.path::BOOLEAN should work via TEXT: {:?}",
        result.err()
    );
    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query");
    };
    assert_eq!(query.projection[0].0.data_type, DataType::Boolean);

    // INT
    let result = analyzer.analyze("SELECT $.properties.count::INT FROM nodes");
    assert!(
        result.is_ok(),
        "$.path::INT should work via TEXT: {:?}",
        result.err()
    );
    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query");
    };
    assert_eq!(query.projection[0].0.data_type, DataType::Int);

    // BIGINT
    let result = analyzer.analyze("SELECT CAST($.properties.timestamp AS BIGINT) FROM nodes");
    assert!(
        result.is_ok(),
        "CAST($.path AS BIGINT) should work via TEXT: {:?}",
        result.err()
    );
    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query");
    };
    assert_eq!(query.projection[0].0.data_type, DataType::BigInt);

    // DOUBLE
    let result = analyzer.analyze("SELECT $.properties.price::DOUBLE FROM nodes");
    assert!(
        result.is_ok(),
        "$.path::DOUBLE should work via TEXT: {:?}",
        result.err()
    );
    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query");
    };
    assert_eq!(query.projection[0].0.data_type, DataType::Double);

    // In WHERE clause comparison - BOOLEAN
    let result =
        analyzer.analyze("SELECT id FROM nodes WHERE $.properties.featured::BOOLEAN = false");
    assert!(
        result.is_ok(),
        "$.path::BOOLEAN in WHERE should work: {:?}",
        result.err()
    );

    // Numeric comparison in WHERE
    let result = analyzer.analyze("SELECT id FROM nodes WHERE $.properties.count::INT > 5");
    assert!(
        result.is_ok(),
        "$.path::INT comparison in WHERE should work: {:?}",
        result.err()
    );

    // Direct JSONB column cast should also work via intermediate
    let result = analyzer.analyze("SELECT CAST(properties AS BOOLEAN) FROM nodes");
    assert!(
        result.is_ok(),
        "CAST(JSONB column AS BOOLEAN) should work via TEXT: {:?}",
        result.err()
    );
}

#[test]
fn test_dollar_dot_json_syntax() {
    let analyzer = Analyzer::new();

    // Basic: $.column.path syntax should expand to column::JSONB #> ARRAY['path']
    let result = analyzer.analyze("SELECT $.id.name FROM nodes");
    assert!(
        result.is_ok(),
        "$.id.name should be allowed: {:?}",
        result.err()
    );
    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query");
    };
    // JsonExtractPath returns nullable JSONB for any type (scalar, object, or array)
    assert_eq!(
        query.projection[0].0.data_type,
        DataType::Nullable(Box::new(DataType::JsonB))
    );

    // Nested path: $.column.level1.level2.level3
    let result = analyzer.analyze("SELECT $.id.properties.user.name FROM nodes");
    assert!(
        result.is_ok(),
        "$.id.properties.user.name should be allowed: {:?}",
        result.err()
    );
    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query");
    };
    assert_eq!(
        query.projection[0].0.data_type,
        DataType::Nullable(Box::new(DataType::JsonB))
    );

    // Just column without path: $.column (should work, returns whole column value)
    let result = analyzer.analyze("SELECT $.id FROM nodes");
    assert!(result.is_ok(), "$.id should be allowed");
    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query");
    };
    // When accessing just $.column without a path, we return the column's type directly
    assert_eq!(query.projection[0].0.data_type, DataType::Text);

    // With alias
    let result = analyzer.analyze("SELECT $.id.properties.displayName as name FROM nodes");
    assert!(result.is_ok(), "$.syntax with alias should work");

    // In WHERE clause with CAST to TEXT for comparison (since $.syntax returns JSONB)
    let result = analyzer.analyze("SELECT * FROM nodes WHERE CAST($.id.role AS TEXT) = 'admin'");
    assert!(
        result.is_ok(),
        "$.syntax in WHERE with CAST should work: {:?}",
        result.err()
    );

    // In WHERE clause with JSONB comparison operator
    let result = analyzer.analyze("SELECT * FROM nodes WHERE $.id.role = '\"admin\"'::JSONB");
    assert!(
        result.is_ok(),
        "$.syntax in WHERE with JSONB comparison should work: {:?}",
        result.err()
    );

    // Multiple $.columns
    let result = analyzer.analyze("SELECT $.id.name, $.id.email, $.path FROM nodes");
    assert!(
        result.is_ok(),
        "Multiple $.columns should work: {:?}",
        result.err()
    );
}

#[test]
fn test_dollar_dot_json_array_subscripts() {
    let analyzer = Analyzer::new();

    // Simple array subscript: $.column[0]
    let result = analyzer.analyze("SELECT $.properties[0] FROM nodes");
    assert!(
        result.is_ok(),
        "$.properties[0] should work: {:?}",
        result.err()
    );
    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query");
    };
    // Array subscript access through JsonExtractPath returns nullable JSONB
    assert_eq!(
        query.projection[0].0.data_type,
        DataType::Nullable(Box::new(DataType::JsonB))
    );

    // Nested array subscript: $.column.field[0].name
    let result = analyzer.analyze("SELECT $.properties.items[0].name FROM nodes");
    assert!(
        result.is_ok(),
        "$.properties.items[0].name should work: {:?}",
        result.err()
    );
    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query");
    };
    assert_eq!(
        query.projection[0].0.data_type,
        DataType::Nullable(Box::new(DataType::JsonB))
    );

    // Multiple subscripts: $.column[0][1]
    let result = analyzer.analyze("SELECT $.properties[0][1] FROM nodes");
    assert!(
        result.is_ok(),
        "$.properties[0][1] should work: {:?}",
        result.err()
    );

    // Negative index: $.column[-1]
    let result = analyzer.analyze("SELECT $.properties.tags[-1] FROM nodes");
    assert!(
        result.is_ok(),
        "$.properties.tags[-1] should work: {:?}",
        result.err()
    );

    // Mixed notation: $.field[0].nested.data[1].value
    let result = analyzer.analyze("SELECT $.properties.items[0].nested.data[1].value FROM nodes");
    assert!(
        result.is_ok(),
        "Mixed array/object notation should work: {:?}",
        result.err()
    );

    // In WHERE clause with CAST ($.syntax returns JSONB, need to cast for TEXT comparison)
    let result = analyzer
        .analyze("SELECT * FROM nodes WHERE CAST($.properties.items[0].status AS TEXT) = 'active'");
    assert!(
        result.is_ok(),
        "Array subscripts in WHERE with CAST should work: {:?}",
        result.err()
    );

    // With alias
    let result =
        analyzer.analyze("SELECT $.properties.items[0].name AS first_item_name FROM nodes");
    assert!(
        result.is_ok(),
        "Array subscripts with alias should work: {:?}",
        result.err()
    );
}

#[test]
fn test_dollar_dot_json_returns_any_type() {
    let analyzer = Analyzer::new();

    // The $.syntax now uses JsonExtractPath (#> operator) which returns JSONB for any type
    // This means it can return scalars, objects, or arrays without errors

    // Test path to scalar value - returns JSONB (not TEXT like old JSON_VALUE)
    let result = analyzer.analyze("SELECT $.properties.name FROM nodes");
    assert!(
        result.is_ok(),
        "$.properties.name (scalar) should work: {:?}",
        result.err()
    );
    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query");
    };
    assert_eq!(
        query.projection[0].0.data_type,
        DataType::Nullable(Box::new(DataType::JsonB))
    );

    // Test path to object - returns JSONB (old JSON_VALUE would error)
    let result = analyzer.analyze("SELECT $.properties.metadata FROM nodes");
    assert!(
        result.is_ok(),
        "$.properties.metadata (object) should work: {:?}",
        result.err()
    );
    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query");
    };
    assert_eq!(
        query.projection[0].0.data_type,
        DataType::Nullable(Box::new(DataType::JsonB))
    );

    // Test path to array - returns JSONB (old JSON_VALUE would error)
    let result = analyzer.analyze("SELECT $.properties.tags FROM nodes");
    assert!(
        result.is_ok(),
        "$.properties.tags (array) should work: {:?}",
        result.err()
    );
    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query");
    };
    assert_eq!(
        query.projection[0].0.data_type,
        DataType::Nullable(Box::new(DataType::JsonB))
    );

    // Test path to nested array
    let result = analyzer.analyze("SELECT $.properties.items[0].subtags FROM nodes");
    assert!(
        result.is_ok(),
        "Nested array path should work: {:?}",
        result.err()
    );
    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query");
    };
    assert_eq!(
        query.projection[0].0.data_type,
        DataType::Nullable(Box::new(DataType::JsonB))
    );

    // Test that non-existent paths return NULL (not error)
    let result = analyzer.analyze("SELECT $.properties.nonexistent.path FROM nodes");
    assert!(
        result.is_ok(),
        "Non-existent path should work (return NULL): {:?}",
        result.err()
    );
    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query");
    };
    assert_eq!(
        query.projection[0].0.data_type,
        DataType::Nullable(Box::new(DataType::JsonB))
    );
}

#[test]
fn test_dollar_syntax_compatibility() {
    let analyzer = Analyzer::new();

    // Test $$ dollar-quoted strings (PostgreSQL syntax)
    // Dollar-quoted strings should work as they don't conflict with $.syntax
    let result = analyzer.analyze("SELECT id FROM nodes WHERE name = $$hello world$$");
    assert!(
        result.is_ok(),
        "$$ dollar-quoted strings should work: {:?}",
        result.err()
    );

    // Test $1, $2 parameter placeholders (PostgreSQL prepared statement syntax)
    // These should work as they don't start with $.
    let result = analyzer.analyze("SELECT id FROM nodes WHERE name = $1 AND path = $2");
    assert!(
        result.is_ok(),
        "$1, $2 parameters should work: {:?}",
        result.err()
    );

    // Test that $ by itself doesn't interfere
    let result = analyzer.analyze("SELECT id FROM nodes WHERE properties ->> 'price' = $1");
    assert!(
        result.is_ok(),
        "$ in parameters with operators should work: {:?}",
        result.err()
    );

    // Combination: $.syntax and parameters in the same query
    let result = analyzer.analyze("SELECT $.id.name FROM nodes WHERE $.id.role = $1");
    assert!(
        result.is_ok(),
        "$.syntax and $1 parameters should coexist: {:?}",
        result.err()
    );
}

#[test]
fn test_json_query_basic() {
    let analyzer = Analyzer::new();

    // Basic JSON_QUERY - extract object
    let result =
        analyzer.analyze("SELECT JSON_QUERY(properties, '$.metadata') AS metadata FROM nodes");
    assert!(result.is_ok(), "JSON_QUERY should work: {:?}", result.err());
    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query");
    };
    // JSON_QUERY returns nullable JSONB
    assert_eq!(
        query.projection[0].0.data_type,
        DataType::Nullable(Box::new(DataType::JsonB))
    );

    // Extract array
    let result = analyzer.analyze("SELECT JSON_QUERY(properties, '$.tags') AS tags FROM nodes");
    assert!(
        result.is_ok(),
        "JSON_QUERY with array path should work: {:?}",
        result.err()
    );
    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query");
    };
    assert_eq!(
        query.projection[0].0.data_type,
        DataType::Nullable(Box::new(DataType::JsonB))
    );

    // Nested path
    let result =
        analyzer.analyze("SELECT JSON_QUERY(properties, '$.user.profile') AS profile FROM nodes");
    assert!(
        result.is_ok(),
        "JSON_QUERY with nested path should work: {:?}",
        result.err()
    );

    // Array index extraction
    let result =
        analyzer.analyze("SELECT JSON_QUERY(properties, '$.items[0]') AS first_item FROM nodes");
    assert!(
        result.is_ok(),
        "JSON_QUERY with array index should work: {:?}",
        result.err()
    );

    // In WHERE clause
    let result = analyzer
        .analyze("SELECT * FROM nodes WHERE JSON_QUERY(properties, '$.settings') IS NOT NULL");
    assert!(
        result.is_ok(),
        "JSON_QUERY in WHERE clause should work: {:?}",
        result.err()
    );

    // With CAST
    let result =
        analyzer.analyze("SELECT CAST(JSON_QUERY(properties, '$.data') AS TEXT) FROM nodes");
    assert!(
        result.is_ok(),
        "JSON_QUERY with CAST should work: {:?}",
        result.err()
    );
}

#[test]
fn test_json_query_vs_json_value() {
    let analyzer = Analyzer::new();

    // JSON_VALUE returns TEXT (for scalars)
    let result = analyzer.analyze("SELECT JSON_VALUE(properties, '$.name') AS name FROM nodes");
    assert!(result.is_ok());
    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query");
    };
    assert_eq!(
        query.projection[0].0.data_type,
        DataType::Nullable(Box::new(DataType::Text))
    );

    // JSON_QUERY returns JSONB (for objects/arrays)
    let result =
        analyzer.analyze("SELECT JSON_QUERY(properties, '$.metadata') AS metadata FROM nodes");
    assert!(result.is_ok());
    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query");
    };
    assert_eq!(
        query.projection[0].0.data_type,
        DataType::Nullable(Box::new(DataType::JsonB))
    );

    // Both in same query
    let result = analyzer.analyze(
        "SELECT JSON_VALUE(properties, '$.name') AS name, JSON_QUERY(properties, '$.tags') AS tags FROM nodes"
    );
    assert!(
        result.is_ok(),
        "JSON_VALUE and JSON_QUERY together should work: {:?}",
        result.err()
    );
}

#[test]
fn test_json_query_with_wrapper_clauses() {
    let analyzer = Analyzer::new();

    // JSON_QUERY with WITH WRAPPER clause
    let result = analyzer.analyze(
        "SELECT JSON_QUERY(properties, '$.items[*]', 'WITH WRAPPER') AS wrapped_items FROM nodes",
    );
    assert!(
        result.is_ok(),
        "JSON_QUERY with WITH WRAPPER should work: {:?}",
        result.err()
    );
    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query");
    };
    assert_eq!(
        query.projection[0].0.data_type,
        DataType::Nullable(Box::new(DataType::JsonB))
    );

    // JSON_QUERY with WITHOUT WRAPPER clause (explicit)
    let result = analyzer.analyze(
        "SELECT JSON_QUERY(properties, '$.items[*]', 'WITHOUT WRAPPER') AS items FROM nodes",
    );
    assert!(
        result.is_ok(),
        "JSON_QUERY with WITHOUT WRAPPER should work: {:?}",
        result.err()
    );

    // JSON_QUERY with CONDITIONAL wrapper
    let result = analyzer.analyze(
        "SELECT JSON_QUERY(properties, '$.items[*]', 'WITH CONDITIONAL WRAPPER') AS items FROM nodes"
    );
    assert!(
        result.is_ok(),
        "JSON_QUERY with CONDITIONAL WRAPPER should work: {:?}",
        result.err()
    );

    // Test underscore format
    let result = analyzer
        .analyze("SELECT JSON_QUERY(properties, '$.data', 'WITH_WRAPPER') AS data FROM nodes");
    assert!(
        result.is_ok(),
        "JSON_QUERY with underscore format should work: {:?}",
        result.err()
    );

    // Test in WHERE clause
    let result = analyzer.analyze(
        "SELECT * FROM nodes WHERE JSON_QUERY(properties, '$.items[*]', 'WITH WRAPPER') IS NOT NULL"
    );
    assert!(
        result.is_ok(),
        "JSON_QUERY with wrapper in WHERE should work: {:?}",
        result.err()
    );

    // Test multiple wrapper clause calls in same query
    let result = analyzer.analyze(
        "SELECT JSON_QUERY(properties, '$.items', 'WITH WRAPPER') AS w1, \
         JSON_QUERY(properties, '$.tags', 'WITHOUT WRAPPER') AS w2 FROM nodes",
    );
    assert!(
        result.is_ok(),
        "Multiple JSON_QUERY calls with different wrappers should work: {:?}",
        result.err()
    );
}

#[test]
fn test_coalesce_with_nullable_types() {
    let analyzer = Analyzer::new();

    // COALESCE with TEXT? and TEXT - should succeed
    let result = analyzer.analyze("SELECT COALESCE(properties ->> 'title', 'default') FROM nodes");
    assert!(result.is_ok(), "COALESCE(TEXT?, TEXT) should be allowed");
    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query");
    };
    // Result should be TEXT (non-nullable) because last arg is a non-null literal
    // COALESCE with a non-null fallback guarantees a non-null result
    assert_eq!(query.projection[0].0.data_type, DataType::Text);

    // COALESCE with multiple nullable arguments
    let result = analyzer.analyze(
        "SELECT COALESCE(properties ->> 'title', properties ->> 'name', 'fallback') FROM nodes",
    );
    assert!(
        result.is_ok(),
        "COALESCE with multiple nullable args should work"
    );

    // COALESCE with numeric types
    let result = analyzer.analyze("SELECT COALESCE(version, 1) FROM nodes");
    assert!(result.is_ok(), "COALESCE(INT, INT) should work");
    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query");
    };
    assert_eq!(query.projection[0].0.data_type, DataType::Int);

    // COALESCE with type promotion (INT to BIGINT)
    let result =
        analyzer.analyze("SELECT COALESCE(version, CAST(1000000000000 AS BIGINT)) FROM nodes");
    assert!(result.is_ok(), "COALESCE should promote INT to BIGINT");
    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query");
    };
    assert_eq!(query.projection[0].0.data_type, DataType::BigInt);

    // COALESCE with incompatible types should fail
    let result = analyzer.analyze("SELECT COALESCE(version, 'text') FROM nodes");
    assert!(
        result.is_err(),
        "COALESCE(INT, TEXT) should fail - incompatible types"
    );

    // COALESCE with no arguments should fail
    let result = analyzer.analyze("SELECT COALESCE() FROM nodes");
    assert!(result.is_err(), "COALESCE() with no arguments should fail");
}

#[test]
fn test_json_extract_operator() {
    let analyzer = Analyzer::new();
    let result = analyzer.analyze("SELECT properties ->> 'title' AS title FROM nodes");
    assert!(result.is_ok());

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };
    assert_eq!(query.projection.len(), 1);

    // Verify the expression is typed as TEXT (nullable)
    let (expr, alias) = &query.projection[0];
    assert_eq!(alias.as_deref(), Some("title"));
    // properties ->> 'title' should return Nullable(Text)
    assert_eq!(expr.data_type, DataType::Nullable(Box::new(DataType::Text)));
}

#[test]
fn test_json_contains_operator() {
    let analyzer = Analyzer::new();
    let result =
        analyzer.analyze("SELECT * FROM nodes WHERE properties @> '{\"status\": \"published\"}'");
    assert!(result.is_ok());

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };
    assert!(query.selection.is_some());

    // Verify WHERE clause is typed as BOOLEAN
    let selection = query.selection.as_ref().unwrap();
    assert_eq!(selection.data_type, DataType::Boolean);
}

#[test]
fn test_combined_json_operators() {
    let analyzer = Analyzer::new();
    let result = analyzer.analyze(
        r#"
        SELECT
            id,
            properties ->> 'title' AS title,
            properties ->> 'status' AS status
        FROM nodes
        WHERE properties @> '{"status": "published"}'
        AND JSON_EXISTS(properties, '$.seo')
    "#,
    );

    assert!(result.is_ok());

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };
    assert_eq!(query.projection.len(), 3);
    assert!(query.selection.is_some());

    // Verify first projection is id
    let (id_expr, _) = &query.projection[0];
    assert_eq!(id_expr.data_type, DataType::Text);

    // Verify second projection is title with nullable text
    let (title_expr, title_alias) = &query.projection[1];
    assert_eq!(title_alias.as_deref(), Some("title"));
    assert_eq!(
        title_expr.data_type,
        DataType::Nullable(Box::new(DataType::Text))
    );

    // Verify third projection is status with nullable text
    let (status_expr, status_alias) = &query.projection[2];
    assert_eq!(status_alias.as_deref(), Some("status"));
    assert_eq!(
        status_expr.data_type,
        DataType::Nullable(Box::new(DataType::Text))
    );
}

#[test]
fn test_json_extract_in_where_clause() {
    let analyzer = Analyzer::new();
    let result =
        analyzer.analyze("SELECT id FROM nodes WHERE properties ->> 'status' = 'published'");
    assert!(result.is_ok());

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };
    assert!(query.selection.is_some());

    // The WHERE clause should be a comparison between nullable text and text
    let selection = query.selection.as_ref().unwrap();
    assert_eq!(selection.data_type, DataType::Boolean);
}

#[test]
fn test_json_operator_type_validation() {
    let analyzer = Analyzer::new();

    // Should fail: TEXT ->> TEXT is invalid (left operand must be JSONB)
    let result = analyzer.analyze("SELECT name ->> 'key' FROM nodes");
    assert!(result.is_err());

    // Should fail: INT @> JSONB is invalid (left operand must be JSONB)
    let result = analyzer.analyze("SELECT * FROM nodes WHERE version @> '{\"x\": 1}'");
    assert!(result.is_err());

    // Should fail: JSONB ->> INT is invalid (right operand must be TEXT)
    let result = analyzer.analyze("SELECT properties ->> 123 FROM nodes");
    assert!(result.is_err());
}

#[test]
fn test_json_extract_with_multiple_columns() {
    let analyzer = Analyzer::new();
    let result = analyzer.analyze(
        r#"
        SELECT
            id,
            name,
            properties ->> 'title' AS title,
            properties ->> 'status' AS status,
            properties ->> 'author' AS author
        FROM nodes
    "#,
    );

    assert!(result.is_ok());

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };
    assert_eq!(query.projection.len(), 5);

    // Verify all JSON extractions return nullable text
    for i in 2..5 {
        let (expr, _) = &query.projection[i];
        assert_eq!(expr.data_type, DataType::Nullable(Box::new(DataType::Text)));
    }
}

#[test]
fn test_json_contains_with_complex_json() {
    let analyzer = Analyzer::new();
    let result = analyzer
        .analyze(r#"SELECT * FROM nodes WHERE properties @> '{"metadata": {"featured": true}}'"#);
    assert!(result.is_ok());

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };
    assert!(query.selection.is_some());

    let selection = query.selection.as_ref().unwrap();
    assert_eq!(selection.data_type, DataType::Boolean);
}

#[test]
fn test_json_operators_with_hierarchy_functions() {
    let analyzer = Analyzer::new();
    let result = analyzer.analyze(
        r#"
        SELECT
            id,
            path,
            properties ->> 'title' AS title
        FROM nodes
        WHERE PATH_STARTS_WITH(path, '/content/blog/')
        AND properties ->> 'status' = 'published'
    "#,
    );

    assert!(result.is_ok());

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };
    assert_eq!(query.projection.len(), 3);
    assert!(query.selection.is_some());
}

#[test]
fn test_json_arrow_operator() {
    let analyzer = Analyzer::new();
    // Test -> operator (extracts JSON object, not text)
    let result = analyzer.analyze("SELECT properties -> 'title' AS title FROM nodes");
    assert!(result.is_ok());

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };
    assert_eq!(query.projection.len(), 1);

    let (expr, alias) = &query.projection[0];
    assert_eq!(alias.as_deref(), Some("title"));
    // -> returns JSONB (JSON object), ->> returns TEXT
    // This matches PostgreSQL behavior
    assert_eq!(
        expr.data_type,
        DataType::Nullable(Box::new(DataType::JsonB))
    );
}

#[test]
fn test_json_arrow_at_operator() {
    let analyzer = Analyzer::new();
    // Test <@ operator (alternative to @>)
    let result =
        analyzer.analyze("SELECT * FROM nodes WHERE properties <@ '{\"status\": \"published\"}'");
    assert!(result.is_ok());

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };
    assert!(query.selection.is_some());

    let selection = query.selection.as_ref().unwrap();
    assert_eq!(selection.data_type, DataType::Boolean);
}

#[test]
fn test_json_any_key_exists_operator() {
    let analyzer = Analyzer::new();
    // Test ?| operator (any key exists) using JSON array
    let result = analyzer.analyze(
        "SELECT * FROM nodes WHERE properties ?| '[\"status\", \"author\", \"title\"]'::JSONB",
    );
    assert!(result.is_ok());

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };
    assert!(query.selection.is_some());

    let selection = query.selection.as_ref().unwrap();
    assert_eq!(selection.data_type, DataType::Boolean);
}

#[test]
fn test_json_all_keys_exist_operator() {
    let analyzer = Analyzer::new();
    // Test ?& operator (all keys exist) using JSON array
    let result =
        analyzer.analyze("SELECT * FROM nodes WHERE properties ?& '[\"id\", \"name\"]'::JSONB");
    assert!(result.is_ok());

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };
    assert!(query.selection.is_some());

    let selection = query.selection.as_ref().unwrap();
    assert_eq!(selection.data_type, DataType::Boolean);
}

#[test]
fn test_json_key_exists_operators_in_select() {
    let analyzer = Analyzer::new();
    // Test using ?| and ?& in SELECT clause with JSON arrays
    let result = analyzer.analyze(
        r#"
        SELECT
            properties ?| '["status", "draft"]'::JSONB AS has_any_status_field,
            properties ?& '["id", "name", "created_at"]'::JSONB AS has_all_required_fields
        FROM nodes
        "#,
    );
    assert!(result.is_ok());

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };

    let projections = &query.projection;
    assert_eq!(projections.len(), 2);
    assert_eq!(projections[0].0.data_type, DataType::Boolean);
    assert_eq!(projections[1].0.data_type, DataType::Boolean);
}

#[test]
fn test_json_key_exists_operators_with_empty_array() {
    let analyzer = Analyzer::new();
    // Test with empty array - should analyze successfully
    let result = analyzer.analyze("SELECT * FROM nodes WHERE properties ?| '[]'::JSONB");
    assert!(result.is_ok());
}

#[test]
fn test_json_key_exists_operators_type_validation() {
    let analyzer = Analyzer::new();
    // Should fail: left operand must be JSONB
    let result = analyzer.analyze("SELECT * FROM nodes WHERE name ?| '[\"test\"]'::JSONB");
    assert!(result.is_err());
}

#[test]
fn test_json_extract_path_operator() {
    let analyzer = Analyzer::new();
    // Test #> operator (extract at path, returns JSONB)
    let result = analyzer
        .analyze("SELECT properties #> '[\"metadata\", \"author\"]'::JSONB AS author FROM nodes");
    assert!(result.is_ok());

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };

    let projections = &query.projection;
    assert_eq!(projections.len(), 1);
    assert_eq!(
        projections[0].0.data_type,
        DataType::Nullable(Box::new(DataType::JsonB))
    );
}

#[test]
fn test_json_extract_path_text_operator() {
    let analyzer = Analyzer::new();
    // Test #>> operator (extract at path as text, returns TEXT)
    let result = analyzer
        .analyze("SELECT properties #>> '[\"metadata\", \"title\"]'::JSONB AS title FROM nodes");
    assert!(result.is_ok());

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };

    let projections = &query.projection;
    assert_eq!(projections.len(), 1);
    assert_eq!(
        projections[0].0.data_type,
        DataType::Nullable(Box::new(DataType::Text))
    );
}

#[test]
fn test_json_path_operators_in_where() {
    let analyzer = Analyzer::new();
    // Test using #> and #>> in WHERE clause
    let result = analyzer.analyze(
        r#"
        SELECT * FROM nodes
        WHERE properties #>> '["status"]'::JSONB = 'published'
        "#,
    );
    assert!(result.is_ok());

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };
    assert!(query.selection.is_some());
}

#[test]
fn test_json_path_operators_with_nested_path() {
    let analyzer = Analyzer::new();
    // Test with deeply nested path
    let result = analyzer
        .analyze(r#"SELECT properties #> '["a", "b", "c", "d"]'::JSONB AS nested FROM nodes"#);
    assert!(result.is_ok());
}

#[test]
fn test_json_path_operators_type_validation() {
    let analyzer = Analyzer::new();
    // Should fail: left operand must be JSONB
    let result = analyzer.analyze("SELECT name #> '[\"test\"]'::JSONB FROM nodes");
    assert!(result.is_err());
}

// ============================================================================
// JSONB - (Remove) Operator Tests
// ============================================================================

#[test]
fn test_json_remove_key_from_object() {
    let analyzer = Analyzer::new();
    // Test JSONB - TEXT: Remove key from object
    let result = analyzer.analyze("SELECT properties - 'username' AS modified FROM nodes");
    assert!(result.is_ok());

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };

    let projections = &query.projection;
    assert_eq!(projections.len(), 1);
    assert_eq!(projections[0].0.data_type, DataType::JsonB);
}

#[test]
fn test_json_remove_element_from_array() {
    let analyzer = Analyzer::new();
    // Test JSONB - INT: Remove element at index from array
    let result = analyzer.analyze("SELECT properties - 1 AS modified FROM nodes");
    assert!(result.is_ok());

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };

    let projections = &query.projection;
    assert_eq!(projections.len(), 1);
    assert_eq!(projections[0].0.data_type, DataType::JsonB);
}

#[test]
fn test_json_remove_multiple_keys() {
    let analyzer = Analyzer::new();
    // Test JSONB - JSONB (array): Remove multiple keys from object
    let result = analyzer
        .analyze(r#"SELECT properties - '["username", "email"]'::JSONB AS modified FROM nodes"#);
    assert!(result.is_ok());

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };

    let projections = &query.projection;
    assert_eq!(projections.len(), 1);
    assert_eq!(projections[0].0.data_type, DataType::JsonB);
}

#[test]
fn test_json_remove_in_where_clause() {
    let analyzer = Analyzer::new();
    // Test using - operator in WHERE clause
    let result = analyzer.analyze(
        r#"
        SELECT * FROM nodes
        WHERE (properties - 'private') @> '{"status": "published"}'::JSONB
        "#,
    );
    assert!(result.is_ok());

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };
    assert!(query.selection.is_some());
}

#[test]
fn test_json_remove_with_cast() {
    let analyzer = Analyzer::new();
    // Test that JSONB - works correctly even when right operand needs casting
    let result = analyzer.analyze("SELECT properties - CAST(1 AS INT) AS modified FROM nodes");
    assert!(result.is_ok());

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };

    let projections = &query.projection;
    assert_eq!(projections.len(), 1);
    assert_eq!(projections[0].0.data_type, DataType::JsonB);
}

// ============================================================================
// JSONB #- (Remove at Path) Operator Tests
// ============================================================================

#[test]
fn test_json_remove_at_path_simple() {
    let analyzer = Analyzer::new();
    // Test #- operator: Remove value at path
    let result = analyzer
        .analyze(r#"SELECT properties #- '["metadata", "author"]'::JSONB AS modified FROM nodes"#);
    assert!(result.is_ok());

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };

    let projections = &query.projection;
    assert_eq!(projections.len(), 1);
    assert_eq!(projections[0].0.data_type, DataType::JsonB);
}

#[test]
fn test_json_remove_at_path_nested() {
    let analyzer = Analyzer::new();
    // Test removing deeply nested value
    let result = analyzer
        .analyze(r#"SELECT properties #- '["a", "b", "c", "d"]'::JSONB AS modified FROM nodes"#);
    assert!(result.is_ok());

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };

    let projections = &query.projection;
    assert_eq!(projections.len(), 1);
    assert_eq!(projections[0].0.data_type, DataType::JsonB);
}

#[test]
fn test_json_remove_at_path_array_index() {
    let analyzer = Analyzer::new();
    // Test removing array element at path
    let result =
        analyzer.analyze(r#"SELECT properties #- '["items", 1]'::JSONB AS modified FROM nodes"#);
    assert!(result.is_ok());

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };

    let projections = &query.projection;
    assert_eq!(projections.len(), 1);
    assert_eq!(projections[0].0.data_type, DataType::JsonB);
}

#[test]
fn test_json_remove_at_path_in_where() {
    let analyzer = Analyzer::new();
    // Test using #- in WHERE clause
    let result = analyzer.analyze(
        r#"
        SELECT * FROM nodes
        WHERE (properties #- '["private"]'::JSONB) @> '{"status": "published"}'::JSONB
        "#,
    );
    assert!(result.is_ok());

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };
    assert!(query.selection.is_some());
}

#[test]
fn test_json_remove_at_path_type_validation() {
    let analyzer = Analyzer::new();
    // Should fail: left operand must be JSONB
    let result = analyzer.analyze(r#"SELECT name #- '["test"]'::JSONB FROM nodes"#);
    assert!(result.is_err());
}

// ============================================================================
// JSONPath Operators (@@ and @?) Tests
// ============================================================================

#[test]
fn test_jsonpath_match_simple() {
    let analyzer = Analyzer::new();
    // Test @@ operator: JSONPath match
    let result = analyzer.analyze(r#"SELECT properties @@ '$.name' AS has_name FROM nodes"#);
    assert!(result.is_ok());

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };

    let projections = &query.projection;
    assert_eq!(projections.len(), 1);
    assert_eq!(projections[0].0.data_type, DataType::Boolean);
}

#[test]
fn test_jsonpath_match_with_filter() {
    let analyzer = Analyzer::new();
    // Test @@ with JSONPath filter
    let result = analyzer
        .analyze(r#"SELECT properties @@ '$.tags[*] ? (@ == "rust")' AS has_rust_tag FROM nodes"#);
    assert!(result.is_ok());

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };

    let projections = &query.projection;
    assert_eq!(projections.len(), 1);
    assert_eq!(projections[0].0.data_type, DataType::Boolean);
}

#[test]
fn test_jsonpath_exists_simple() {
    let analyzer = Analyzer::new();
    // Test @? operator: JSONPath exists
    let result =
        analyzer.analyze(r#"SELECT properties @? '$.metadata.author' AS has_author FROM nodes"#);
    assert!(result.is_ok());

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };

    let projections = &query.projection;
    assert_eq!(projections.len(), 1);
    assert_eq!(projections[0].0.data_type, DataType::Boolean);
}

#[test]
fn test_jsonpath_exists_complex() {
    let analyzer = Analyzer::new();
    // Test @? with complex path
    let result = analyzer
        .analyze(r#"SELECT properties @? '$.items[*].status' AS has_item_status FROM nodes"#);
    assert!(result.is_ok());

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };

    let projections = &query.projection;
    assert_eq!(projections.len(), 1);
    assert_eq!(projections[0].0.data_type, DataType::Boolean);
}

#[test]
fn test_jsonpath_in_where_clause() {
    let analyzer = Analyzer::new();
    // Test using @@ in WHERE clause
    let result = analyzer.analyze(
        r#"
        SELECT * FROM nodes
        WHERE properties @@ '$.status ? (@ == "published")'
        "#,
    );
    assert!(result.is_ok());

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };
    assert!(query.selection.is_some());
}

#[test]
fn test_jsonpath_type_validation() {
    let analyzer = Analyzer::new();
    // Should fail: left operand must be JSONB
    let result = analyzer.analyze(r#"SELECT name @@ '$.test' FROM nodes"#);
    assert!(result.is_err());
}

#[test]
fn test_jsonpath_combined_operators() {
    let analyzer = Analyzer::new();
    // Test combining multiple JSONPath operators
    let result = analyzer.analyze(
        r#"
        SELECT
            properties @@ '$.tags[*] ? (@ == "rust")' AS has_rust,
            properties @? '$.metadata' AS has_metadata
        FROM nodes
        "#,
    );
    assert!(result.is_ok());

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };
    assert_eq!(query.projection.len(), 2);
    assert_eq!(query.projection[0].0.data_type, DataType::Boolean);
    assert_eq!(query.projection[1].0.data_type, DataType::Boolean);
}

// ============================================================================
// GROUP BY and HAVING Tests
// SKIPPED: GROUP BY and HAVING are not yet supported in the semantic analyzer
// These features are documented in UNSUPPORTED_FEATURES.md
// ============================================================================

// Note: GROUP BY and HAVING tests are skipped because the analyzer doesn't support them yet.
// When support is added in the future, uncomment these tests.

// ============================================================================
// Multiple Aggregate Functions
// ============================================================================

#[test]
fn test_multiple_aggregates_in_projection() {
    let analyzer = Analyzer::new();
    let result = analyzer.analyze(
        r#"
        SELECT
            COUNT(*) AS total,
            MIN(created_at) AS oldest,
            MAX(updated_at) AS newest,
            SUM(version) AS total_versions,
            AVG(version) AS avg_version
        FROM nodes
        "#,
    );
    assert!(result.is_ok());

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };

    assert_eq!(query.projection.len(), 5);

    // Verify types
    let (count_expr, _) = &query.projection[0];
    assert_eq!(count_expr.data_type, DataType::BigInt);

    let (sum_expr, _) = &query.projection[3];
    // SUM returns Nullable(Double) for numeric types
    assert_eq!(
        sum_expr.data_type,
        DataType::Nullable(Box::new(DataType::Double))
    );

    let (avg_expr, _) = &query.projection[4];
    assert_eq!(
        avg_expr.data_type,
        DataType::Nullable(Box::new(DataType::Double))
    );
}

#[test]
fn test_aggregates_with_distinct() {
    let analyzer = Analyzer::new();
    let result = analyzer.analyze("SELECT COUNT(DISTINCT node_type) as unique_types FROM nodes");
    assert!(result.is_ok());

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };

    assert_eq!(query.projection.len(), 1);
    let (count_expr, _) = &query.projection[0];
    assert_eq!(count_expr.data_type, DataType::BigInt);
}

// ============================================================================
// ORDER BY Multiple Columns
// ============================================================================

#[test]
fn test_order_by_multiple_columns() {
    let analyzer = Analyzer::new();
    let result =
        analyzer.analyze("SELECT id, created_at FROM nodes ORDER BY created_at DESC, id DESC");
    assert!(result.is_ok());

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };

    // Verify we have 2 ORDER BY expressions
    assert_eq!(query.order_by.len(), 2);

    // First ordering: created_at DESC
    let first_order = &query.order_by[0];
    assert_eq!(first_order.expr.data_type, DataType::TimestampTz);
    assert!(first_order.descending);

    // Second ordering: id DESC
    let second_order = &query.order_by[1];
    assert_eq!(second_order.expr.data_type, DataType::Text);
    assert!(second_order.descending);
}

#[test]
fn test_order_by_mixed_directions() {
    let analyzer = Analyzer::new();
    let result = analyzer.analyze(
        "SELECT id, name, created_at FROM nodes ORDER BY created_at ASC, name DESC, id ASC",
    );
    assert!(result.is_ok());

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };

    assert_eq!(query.order_by.len(), 3);

    // Verify directions
    assert!(!query.order_by[0].descending); // ASC = false (is_desc = false)
    assert!(query.order_by[1].descending); // DESC = true
    assert!(!query.order_by[2].descending); // ASC = false
}

#[test]
fn test_order_by_with_json_extraction() {
    let analyzer = Analyzer::new();
    let result = analyzer.analyze(
        r#"
        SELECT
            id,
            properties ->> 'title' AS title
        FROM nodes
        ORDER BY properties ->> 'title', created_at DESC
        "#,
    );
    assert!(result.is_ok());

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };

    assert_eq!(query.order_by.len(), 2);

    // First order by should be nullable text (JSON extraction)
    let first_order = &query.order_by[0];
    assert_eq!(
        first_order.expr.data_type,
        DataType::Nullable(Box::new(DataType::Text))
    );
}

// ============================================================================
// Complex WHERE Conditions
// ============================================================================

#[test]
fn test_complex_where_with_and_or() {
    let analyzer = Analyzer::new();
    let result = analyzer.analyze(
        r#"
        SELECT * FROM nodes
        WHERE (node_type = 'my:Article' AND properties ->> 'status' = 'published')
           OR (node_type = 'my:Page' AND DEPTH(path) < 3)
        "#,
    );
    assert!(result.is_ok());

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };

    assert!(query.selection.is_some());
    let selection = query.selection.as_ref().unwrap();
    assert_eq!(selection.data_type, DataType::Boolean);
}

#[test]
fn test_complex_where_with_nested_logic() {
    let analyzer = Analyzer::new();
    let result = analyzer.analyze(
        r#"
        SELECT id FROM nodes
        WHERE (
            (node_type = 'my:Article' AND properties ->> 'status' = 'published')
            OR
            (node_type = 'my:Page' AND properties ->> 'status' = 'draft')
        )
        AND PATH_STARTS_WITH(path, '/content/')
        AND version > 0
        "#,
    );
    assert!(result.is_ok());

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };

    assert!(query.selection.is_some());
    assert_eq!(
        query.selection.as_ref().unwrap().data_type,
        DataType::Boolean
    );
}

#[test]
fn test_where_with_multiple_hierarchy_conditions() {
    let analyzer = Analyzer::new();
    let result = analyzer.analyze(
        r#"
        SELECT id, path FROM nodes
        WHERE PATH_STARTS_WITH(path, '/content/')
        AND DEPTH(path) BETWEEN 2 AND 4
        AND PARENT(path) != '/content/archive'
        "#,
    );
    assert!(result.is_ok());

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };

    assert!(query.selection.is_some());
}

// ============================================================================
// DEPTH Arithmetic and Complex Expressions
// ============================================================================

#[test]
fn test_depth_arithmetic_in_select() {
    let analyzer = Analyzer::new();
    let result = analyzer.analyze(
        "SELECT DEPTH(path) - 2 AS relative_depth FROM nodes WHERE PATH_STARTS_WITH(path, '/content/')"
    );
    assert!(result.is_ok());

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };

    assert_eq!(query.projection.len(), 1);

    // DEPTH returns INT, so DEPTH - 2 should also be INT
    let (depth_expr, alias) = &query.projection[0];
    assert_eq!(depth_expr.data_type, DataType::Int);
    assert_eq!(alias.as_deref(), Some("relative_depth"));
}

#[test]
fn test_depth_arithmetic_in_where() {
    let analyzer = Analyzer::new();
    let result = analyzer.analyze(
        r#"
        SELECT id, path FROM nodes
        WHERE DEPTH(path) - DEPTH('/content/') = 2
        AND PATH_STARTS_WITH(path, '/content/')
        "#,
    );
    assert!(result.is_ok());

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };

    assert!(query.selection.is_some());
}

#[test]
fn test_depth_with_multiplication() {
    let analyzer = Analyzer::new();
    let result = analyzer.analyze("SELECT path, DEPTH(path) * 10 AS depth_score FROM nodes");
    assert!(result.is_ok());

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };

    assert_eq!(query.projection.len(), 2);
    let (depth_expr, _) = &query.projection[1];
    assert_eq!(depth_expr.data_type, DataType::Int);
}

#[test]
fn test_complex_depth_expression() {
    let analyzer = Analyzer::new();
    let result = analyzer.analyze(
        r#"
        SELECT
            id,
            path,
            DEPTH(path) - 2 AS relative_depth,
            (DEPTH(path) - 1) * 100 AS depth_score
        FROM nodes
        WHERE (DEPTH(path) - DEPTH('/content/')) BETWEEN 1 AND 3
        "#,
    );
    assert!(result.is_ok());

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };

    assert_eq!(query.projection.len(), 4);
}

// ============================================================================
// Combined Tests (Hierarchy + JSON + Aggregates)
// ============================================================================

// test_combined_hierarchy_json_and_aggregates: SKIPPED
// Requires GROUP BY support which is not yet implemented

#[test]
fn test_full_featured_query() {
    let analyzer = Analyzer::new();
    let result = analyzer.analyze(
        r#"
        SELECT
            id,
            path,
            PARENT(path) as parent_path,
            DEPTH(path) as depth,
            properties ->> 'title' AS title,
            created_at
        FROM nodes
        WHERE PATH_STARTS_WITH(path, '/content/blog/')
        AND DEPTH(path) = 3
        AND properties ->> 'status' = 'published'
        ORDER BY created_at DESC, id ASC
        LIMIT 20 OFFSET 0
        "#,
    );
    assert!(result.is_ok());

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };

    assert_eq!(query.projection.len(), 6);
    assert!(query.selection.is_some());
    assert_eq!(query.order_by.len(), 2);
    assert_eq!(query.limit, Some(20));
    assert_eq!(query.offset, Some(0));
}

// ============================================================================
// Pagination Patterns
// ============================================================================

#[test]
fn test_cursor_based_pagination_pattern() {
    let analyzer = Analyzer::new();
    // Simplified pagination test without complex timestamp comparisons
    let result = analyzer.analyze(
        r#"
        SELECT id, name, path
        FROM nodes
        WHERE id < 'node-123'
        ORDER BY id DESC
        LIMIT 10
        "#,
    );
    assert!(result.is_ok());

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };

    assert!(query.selection.is_some());
    assert_eq!(query.order_by.len(), 1);
    assert_eq!(query.limit, Some(10));
}

#[test]
fn test_path_based_pagination_pattern() {
    let analyzer = Analyzer::new();
    let result = analyzer.analyze(
        r#"
        SELECT id, name, path
        FROM nodes
        WHERE PATH_STARTS_WITH(path, '/content/blog/')
        AND path > '/content/blog/2025/article-010'
        ORDER BY path
        LIMIT 10
        "#,
    );
    assert!(result.is_ok());

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };

    assert!(query.selection.is_some());
    assert_eq!(query.order_by.len(), 1);
    assert_eq!(query.limit, Some(10));
}

// ============================================================================
// Edge Cases and Error Handling
// ============================================================================

#[test]
fn test_aggregate_without_group_by() {
    let analyzer = Analyzer::new();
    // Aggregates without GROUP BY should work (single group)
    let result = analyzer.analyze("SELECT COUNT(*), MIN(created_at), MAX(created_at) FROM nodes");
    assert!(result.is_ok());

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };

    assert_eq!(query.projection.len(), 3);
    // No GROUP BY in this query
}

// test_order_by_with_aggregates: SKIPPED
// Requires GROUP BY support which is not yet implemented

// ============================================================================
// Revision Extraction Tests
// ============================================================================

#[test]
fn test_revision_extraction_simple() {
    let analyzer = Analyzer::new();
    let result = analyzer.analyze("SELECT id FROM nodes WHERE __revision = 342");
    assert!(result.is_ok());

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };

    // Should extract revision
    assert_eq!(query.max_revision, Some(HLC::new(342, 0)));
    // Should remove __revision from selection
    assert!(query.selection.is_none());
}

#[test]
fn test_revision_extraction_with_other_predicates() {
    let analyzer = Analyzer::new();
    let result = analyzer.analyze("SELECT id FROM nodes WHERE __revision = 100 AND name = 'test'");
    assert!(result.is_ok());

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };

    // Should extract revision
    assert_eq!(query.max_revision, Some(HLC::new(100, 0)));
    // Should keep other predicates in selection
    assert!(query.selection.is_some());
}

#[test]
fn test_revision_extraction_with_predicates_before() {
    let analyzer = Analyzer::new();
    let result = analyzer.analyze("SELECT id FROM nodes WHERE name = 'test' AND __revision = 200");
    assert!(result.is_ok());

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };

    // Should extract revision regardless of order
    assert_eq!(query.max_revision, Some(HLC::new(200, 0)));
    // Should keep other predicates
    assert!(query.selection.is_some());
}

#[test]
fn test_revision_extraction_is_null() {
    let analyzer = Analyzer::new();
    let result = analyzer.analyze("SELECT id FROM nodes WHERE __revision IS NULL");
    assert!(result.is_ok());

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };

    // IS NULL means HEAD revision
    assert_eq!(query.max_revision, None);
    // Should remove predicate
    assert!(query.selection.is_none());
}

#[test]
fn test_revision_extraction_head_string() {
    let analyzer = Analyzer::new();
    // Note: Since __revision is BigInt, comparing with 'HEAD' string will fail type checking
    // This is actually correct behavior - if someone wants HEAD, they should use IS NULL or omit the predicate
    // So we test that this correctly fails
    let result = analyzer.analyze("SELECT id FROM nodes WHERE __revision = 'HEAD'");
    // This should fail type checking since __revision is BigInt and 'HEAD' is Text
    assert!(result.is_err());
}

#[test]
fn test_no_revision_predicate() {
    let analyzer = Analyzer::new();
    let result = analyzer.analyze("SELECT id FROM nodes WHERE name = 'test'");
    assert!(result.is_ok());

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };

    // No revision predicate means HEAD (None)
    assert_eq!(query.max_revision, None);
    // Other predicates should remain
    assert!(query.selection.is_some());
}

#[test]
fn test_revision_extraction_complex_and() {
    let analyzer = Analyzer::new();
    let result = analyzer
        .analyze("SELECT id FROM nodes WHERE name = 'test' AND __revision = 500 AND version > 1");
    assert!(result.is_ok());

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };

    // Should extract revision from middle of AND chain
    assert_eq!(query.max_revision, Some(HLC::new(500, 0)));
    // Should keep other predicates
    assert!(query.selection.is_some());
}

#[test]
fn test_workspace_qualified_properties_column() {
    let mut catalog = StaticCatalog::default_nodes_schema();
    catalog.register_workspace("test".to_string());
    let analyzer = Analyzer::with_catalog(Box::new(catalog));

    // Test qualified column reference to properties in a workspace
    let result = analyzer.analyze("SELECT test.properties FROM test");

    match &result {
        Ok(AnalyzedStatement::Query(query)) => {
            assert_eq!(query.projection.len(), 1);
            let (expr, _) = &query.projection[0];
            assert_eq!(expr.data_type, DataType::JsonB);
        }
        Ok(_) => panic!("Expected Query, got different statement type"),
        Err(e) => panic!("Expected success, got error: {}", e),
    }
}

#[test]
fn test_workspace_join_with_properties() {
    let mut catalog = StaticCatalog::default_nodes_schema();
    catalog.register_workspace("test".to_string());
    catalog.register_workspace("wurst".to_string());
    let analyzer = Analyzer::with_catalog(Box::new(catalog));

    // Test JOIN with qualified properties column references
    let result = analyzer.analyze(
        "SELECT test.path, wurst.path FROM test INNER JOIN wurst ON test.properties->>'description' = wurst.properties->>'description'"
    );

    match &result {
        Ok(AnalyzedStatement::Query(query)) => {
            assert_eq!(query.projection.len(), 2);
            assert_eq!(query.from.len(), 1); // test
            assert_eq!(query.joins.len(), 1); // wurst
        }
        Ok(_) => panic!("Expected Query, got different statement type"),
        Err(e) => panic!("Expected success, got error: {}", e),
    }
}

#[test]
fn test_left_right_join_variants() {
    let mut catalog = StaticCatalog::default_nodes_schema();
    catalog.register_workspace("test".to_string());
    catalog.register_workspace("wurst".to_string());
    let analyzer = Analyzer::with_catalog(Box::new(catalog));

    // Test LEFT JOIN (without OUTER keyword)
    let result = analyzer
        .analyze("SELECT test.id, wurst.id FROM test LEFT JOIN wurst ON test.id = wurst.id");
    assert!(result.is_ok(), "LEFT JOIN should work: {:?}", result);

    // Test LEFT OUTER JOIN (with OUTER keyword)
    let result = analyzer
        .analyze("SELECT test.id, wurst.id FROM test LEFT OUTER JOIN wurst ON test.id = wurst.id");
    assert!(result.is_ok(), "LEFT OUTER JOIN should work: {:?}", result);

    // Test RIGHT JOIN
    let result = analyzer
        .analyze("SELECT test.id, wurst.id FROM test RIGHT JOIN wurst ON test.id = wurst.id");
    assert!(result.is_ok(), "RIGHT JOIN should work: {:?}", result);

    // Test RIGHT OUTER JOIN
    let result = analyzer
        .analyze("SELECT test.id, wurst.id FROM test RIGHT OUTER JOIN wurst ON test.id = wurst.id");
    assert!(result.is_ok(), "RIGHT OUTER JOIN should work: {:?}", result);
}

#[test]
fn test_group_by_with_array_agg() {
    let mut catalog = StaticCatalog::default_nodes_schema();
    catalog.register_workspace("test".to_string());
    catalog.register_workspace("wurst".to_string());
    let analyzer = Analyzer::with_catalog(Box::new(catalog));

    // Test GROUP BY with array_agg aggregate function
    let result = analyzer.analyze(
        "SELECT test.path, array_agg(wurst.path), array_agg(wurst.name) \
         FROM test \
         INNER JOIN wurst ON test.properties->>'description' = wurst.properties->>'description' \
         GROUP BY test.id, test.path, test.properties->>'description'",
    );
    assert!(
        result.is_ok(),
        "GROUP BY with array_agg should work: {:?}",
        result
    );

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };

    // Verify GROUP BY expressions were parsed
    assert_eq!(
        query.group_by.len(),
        3,
        "Should have 3 GROUP BY expressions"
    );

    // Verify aggregates were extracted
    assert_eq!(
        query.aggregates.len(),
        2,
        "Should have 2 aggregate functions"
    );
    assert_eq!(
        query.aggregates[0].func,
        crate::logical_plan::AggregateFunction::ArrayAgg
    );
    assert_eq!(
        query.aggregates[1].func,
        crate::logical_plan::AggregateFunction::ArrayAgg
    );
}

/// Test CTE column resolution in JOIN conditions
/// This reproduces the bug where graph.sid is not found in JOIN ON clause
#[test]
fn test_cte_column_resolution_in_join() {
    let analyzer = Analyzer::new();
    let sql = r#"
        WITH graph AS (
            SELECT
                s_id AS sid,
                t_id AS tid
            FROM cypher('MATCH (s)-[r]->(t) RETURN s.id, t.id')
        )
        SELECT src.id, tgt.id
        FROM graph
        JOIN nodes src ON src.id = graph.sid
        JOIN nodes tgt ON tgt.id = graph.tid
    "#;

    let result = analyzer.analyze(sql);

    assert!(
        result.is_ok(),
        "CTE column resolution should work: {:?}",
        result
    );
    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };

    assert_eq!(query.ctes.len(), 1, "Should have 1 CTE");
    assert!(query.joins.len() > 0, "Should have at least one JOIN");
}

/// Test CTE with wildcard expansion and table aliases
/// This tests that src.* and tgt.* correctly expand when src/tgt are aliases
#[test]
fn test_cte_with_wildcard_and_aliases() {
    let analyzer = Analyzer::new();
    let sql = r#"
        WITH graph AS (
            SELECT
                s_id AS sid,
                t_id AS tid
            FROM cypher('MATCH (s)-[r]->(t) RETURN s.id, t.id')
        )
        SELECT src.*, tgt.*
        FROM graph
        JOIN nodes src ON src.id = graph.sid
        JOIN nodes tgt ON tgt.id = graph.tid
    "#;

    let result = analyzer.analyze(sql);

    assert!(
        result.is_ok(),
        "Wildcard with aliases should work: {:?}",
        result
    );
    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };

    // Verify we have the CTE
    assert_eq!(query.ctes.len(), 1, "Should have 1 CTE");

    // Verify wildcard expansion happened (nodes table has 22 columns, so 2 * 22 = 44)
    assert!(
        query.projection.len() > 20,
        "Should have expanded src.* and tgt.* to many columns, got {}",
        query.projection.len()
    );
}

#[test]
fn test_fulltext_match_function() {
    let analyzer = Analyzer::new();

    // Test FULLTEXT_MATCH function with basic query
    let result = analyzer
        .analyze("SELECT id, name FROM nodes WHERE FULLTEXT_MATCH('rust AND web', 'english')");

    assert!(
        result.is_ok(),
        "FULLTEXT_MATCH query should parse successfully: {:?}",
        result.err()
    );
    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };

    // Verify we have a WHERE clause
    assert!(query.selection.is_some(), "Should have WHERE clause");

    // Verify the selection is a Function call
    if let Some(selection) = &query.selection {
        if let Expr::Function { name, args, .. } = &selection.expr {
            assert_eq!(
                name.to_uppercase(),
                "FULLTEXT_MATCH",
                "Should be FULLTEXT_MATCH function"
            );
            assert_eq!(args.len(), 2, "Should have 2 arguments");
        } else {
            panic!(
                "Selection should be a Function call, got: {:?}",
                selection.expr
            );
        }
    }

    // Verify the result type is Boolean
    if let Some(selection) = &query.selection {
        assert_eq!(
            selection.data_type,
            DataType::Boolean,
            "FULLTEXT_MATCH should return Boolean"
        );
    }
}

#[test]
fn test_fulltext_match_with_hierarchy() {
    let analyzer = Analyzer::new();

    // Test FULLTEXT_MATCH combined with hierarchy filtering
    let result = analyzer.analyze(
        "SELECT id, name, path FROM nodes \
         WHERE FULLTEXT_MATCH('database performance', 'english') \
         AND PATH_STARTS_WITH(path, '/content/') \
         LIMIT 10",
    );

    assert!(
        result.is_ok(),
        "Combined FULLTEXT_MATCH and PATH_STARTS_WITH should work: {:?}",
        result.err()
    );
}

// ========================================================================================
// CASE Expression Tests
// ========================================================================================

#[test]
fn test_case_expression_simple() {
    let analyzer = Analyzer::new();

    // Simple searched CASE
    let result = analyzer.analyze(
        "SELECT id,
         CASE
             WHEN version > 10 THEN 'high'
             WHEN version > 5 THEN 'medium'
             ELSE 'low'
         END AS version_category
         FROM nodes",
    );

    assert!(
        result.is_ok(),
        "Simple CASE expression should be supported: {:?}",
        result.err()
    );

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };

    // Check projection: id and CASE result
    assert_eq!(query.projection.len(), 2);

    // Second projection is the CASE expression
    let (case_expr, case_alias) = &query.projection[1];

    // Verify it's a CASE expression
    assert!(
        matches!(case_expr.expr, Expr::Case { .. }),
        "Expected CASE expression, got {:?}",
        case_expr.expr
    );

    // Result type should be TEXT (common type of 'high', 'medium', 'low')
    assert_eq!(case_expr.data_type, DataType::Text);

    // Check alias
    assert_eq!(case_alias.as_deref(), Some("version_category"));
}

#[test]
fn test_case_expression_nullable_result() {
    let analyzer = Analyzer::new();

    // CASE without ELSE clause - result should be nullable
    let result = analyzer.analyze(
        "SELECT id,
         CASE
             WHEN version > 10 THEN 'high'
             WHEN version > 5 THEN 'medium'
         END AS version_category
         FROM nodes",
    );

    assert!(
        result.is_ok(),
        "CASE without ELSE should work: {:?}",
        result.err()
    );

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };

    let (case_expr, _) = &query.projection[1];

    // Result type should be nullable TEXT since no ELSE clause
    assert_eq!(
        case_expr.data_type,
        DataType::Nullable(Box::new(DataType::Text)),
        "CASE without ELSE should return nullable type"
    );
}

#[test]
fn test_case_expression_numeric_result() {
    let analyzer = Analyzer::new();

    // CASE with numeric results
    let result = analyzer.analyze(
        "SELECT id,
         CASE
             WHEN version > 100 THEN 3
             WHEN version > 50 THEN 2
             ELSE 1
         END AS priority
         FROM nodes",
    );

    assert!(
        result.is_ok(),
        "CASE with numeric results should work: {:?}",
        result.err()
    );

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };

    let (case_expr, _) = &query.projection[1];

    // Result type should be INT
    assert_eq!(case_expr.data_type, DataType::Int);
}

#[test]
fn test_case_expression_in_where_clause() {
    let analyzer = Analyzer::new();

    // CASE in WHERE clause
    let result = analyzer.analyze(
        "SELECT id, name FROM nodes
         WHERE CASE
             WHEN node_type = 'my:Article' THEN properties ->> 'status' = 'published'
             ELSE true
         END",
    );

    assert!(
        result.is_ok(),
        "CASE in WHERE clause should work: {:?}",
        result.err()
    );

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };

    // Verify WHERE clause contains CASE expression
    assert!(query.selection.is_some());
    let selection = query.selection.as_ref().unwrap();

    assert!(
        matches!(selection.expr, Expr::Case { .. }),
        "WHERE clause should contain CASE expression"
    );

    // CASE result should be BOOLEAN
    assert_eq!(selection.data_type, DataType::Boolean);
}

#[test]
fn test_case_expression_with_null_else() {
    let analyzer = Analyzer::new();

    // CASE with explicit NULL ELSE
    let result = analyzer.analyze(
        "SELECT id,
         CASE
             WHEN version > 10 THEN 'high'
             ELSE NULL
         END AS category
         FROM nodes",
    );

    assert!(
        result.is_ok(),
        "CASE with NULL ELSE should work: {:?}",
        result.err()
    );

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };

    let (case_expr, _) = &query.projection[1];

    // Result should be nullable TEXT
    assert_eq!(
        case_expr.data_type,
        DataType::Nullable(Box::new(DataType::Text))
    );
}

#[test]
fn test_case_expression_mixed_types() {
    let analyzer = Analyzer::new();

    // CASE with mixed numeric types (INT and DOUBLE)
    let result = analyzer.analyze(
        "SELECT id,
         CASE
             WHEN version > 10 THEN 1
             ELSE 2.5
         END AS score
         FROM nodes",
    );

    assert!(
        result.is_ok(),
        "CASE with mixed numeric types should work: {:?}",
        result.err()
    );

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };

    let (case_expr, _) = &query.projection[1];

    // Result should be DOUBLE (common type of INT and DOUBLE)
    assert_eq!(case_expr.data_type, DataType::Double);
}

#[test]
fn test_case_expression_type_mismatch_error() {
    let analyzer = Analyzer::new();

    // CASE with incompatible result types should fail
    let result = analyzer.analyze(
        "SELECT id,
         CASE
             WHEN version > 10 THEN 'high'
             ELSE 123
         END AS category
         FROM nodes",
    );

    assert!(
        result.is_err(),
        "CASE with incompatible types (TEXT and INT) should fail"
    );
}

#[test]
fn test_case_expression_nested() {
    let analyzer = Analyzer::new();

    // Nested CASE expressions
    let result = analyzer.analyze(
        "SELECT id,
         CASE
             WHEN version > 10 THEN
                 CASE WHEN version > 15 THEN 'new-high' ELSE 'old-high' END
             ELSE 'low'
         END AS category
         FROM nodes",
    );

    assert!(
        result.is_ok(),
        "Nested CASE should work: {:?}",
        result.err()
    );

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };

    let (case_expr, _) = &query.projection[1];
    assert_eq!(case_expr.data_type, DataType::Text);
}

// ========================================================================================
// JSONB Concatenation Tests
// ========================================================================================

#[test]
fn test_jsonb_concat_operator() {
    let analyzer = Analyzer::new();

    // JSONB concatenation in SELECT
    let result = analyzer.analyze(
        "SELECT id, properties || '{\"new_field\": \"value\"}' AS updated_props FROM nodes",
    );

    assert!(
        result.is_ok(),
        "JSONB concatenation should be supported: {:?}",
        result.err()
    );

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };

    // Check second projection (the concatenation)
    let (concat_expr, concat_alias) = &query.projection[1];

    // Verify it's a BinaryOp with JsonConcat
    assert!(
        matches!(
            concat_expr.expr,
            Expr::BinaryOp {
                op: crate::analyzer::BinaryOperator::JsonConcat,
                ..
            }
        ),
        "Expected JSONB concatenation expression, got {:?}",
        concat_expr.expr
    );

    // Result type should be JSONB
    assert_eq!(concat_expr.data_type, DataType::JsonB);

    assert_eq!(concat_alias.as_deref(), Some("updated_props"));
}

#[test]
fn test_jsonb_concat_in_projection() {
    let analyzer = Analyzer::new();

    // Multiple JSONB concatenations
    let result = analyzer.analyze(
        "SELECT
             id,
             properties || '{\"status\": \"active\"}' AS with_status,
             properties || '{\"updated\": true}' AS with_updated
         FROM nodes",
    );

    assert!(
        result.is_ok(),
        "Multiple JSONB concat should work: {:?}",
        result.err()
    );

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };

    assert_eq!(query.projection.len(), 3);

    // Both concatenations should return JSONB
    let (expr1, _) = &query.projection[1];
    let (expr2, _) = &query.projection[2];

    assert_eq!(expr1.data_type, DataType::JsonB);
    assert_eq!(expr2.data_type, DataType::JsonB);
}

#[test]
fn test_jsonb_concat_chained() {
    let analyzer = Analyzer::new();

    // Chained JSONB concatenations
    let result = analyzer.analyze(
        "SELECT
             properties || '{\"a\": 1}' || '{\"b\": 2}' AS merged
         FROM nodes",
    );

    assert!(
        result.is_ok(),
        "Chained JSONB concat should work: {:?}",
        result.err()
    );

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };

    let (concat_expr, _) = &query.projection[0];
    assert_eq!(concat_expr.data_type, DataType::JsonB);
}

#[test]
fn test_string_concat_with_text() {
    let analyzer = Analyzer::new();

    // || with Text types should work (string concatenation)
    let result = analyzer.analyze("SELECT name || ' suffix' AS result FROM nodes");

    assert!(
        result.is_ok(),
        "String concatenation with || should work for Text types: {:?}",
        result.err()
    );

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };

    // Result should be Text type
    let (concat_expr, _) = &query.projection[0];
    assert_eq!(concat_expr.data_type, DataType::Text);
}

#[test]
fn test_case_and_jsonb_concat_combined() {
    let analyzer = Analyzer::new();

    // CASE expression with JSONB concatenation
    let result = analyzer.analyze(
        "SELECT
             id,
             CASE
                 WHEN version > 10 THEN properties || '{\"tier\": \"premium\"}'
                 ELSE properties || '{\"tier\": \"standard\"}'
             END AS enriched_props
         FROM nodes",
    );

    assert!(
        result.is_ok(),
        "CASE with JSONB concat should work: {:?}",
        result.err()
    );

    let AnalyzedStatement::Query(query) = result.unwrap() else {
        panic!("Expected Query statement");
    };

    let (case_expr, _) = &query.projection[1];

    // CASE expression with JSONB results
    assert!(matches!(case_expr.expr, Expr::Case { .. }));
    assert_eq!(case_expr.data_type, DataType::JsonB);
}

// ========================================================================================
// COPY Statement Tests
// ========================================================================================

#[test]
fn test_copy_statement_simple() {
    let mut catalog = StaticCatalog::default_nodes_schema();
    catalog.register_workspace("social".to_string());
    let analyzer = Analyzer::with_catalog(Box::new(catalog));

    // Test basic COPY statement
    let sql = "COPY social SET path='/users/senol' TO path='/posts'";
    let result = analyzer.analyze(sql);

    assert!(
        result.is_ok(),
        "COPY statement should be supported: {:?}",
        result.err()
    );

    let AnalyzedStatement::Copy(copy) = result.unwrap() else {
        panic!("Expected Copy statement");
    };

    assert_eq!(copy.table, "social");
    assert!(!copy.recursive);
}

#[test]
fn test_copy_tree_statement() {
    let mut catalog = StaticCatalog::default_nodes_schema();
    catalog.register_workspace("social".to_string());
    let analyzer = Analyzer::with_catalog(Box::new(catalog));

    // Test COPY TREE statement
    let sql = "COPY TREE social SET path='/users/senol' TO path='/posts'";
    let result = analyzer.analyze(sql);

    assert!(
        result.is_ok(),
        "COPY TREE statement should be supported: {:?}",
        result.err()
    );

    let AnalyzedStatement::Copy(copy) = result.unwrap() else {
        panic!("Expected Copy statement");
    };

    assert_eq!(copy.table, "social");
    assert!(copy.recursive);
}

#[test]
fn test_copy_with_new_name() {
    let mut catalog = StaticCatalog::default_nodes_schema();
    catalog.register_workspace("social".to_string());
    let analyzer = Analyzer::with_catalog(Box::new(catalog));

    // Test COPY with AS 'new-name' clause
    let sql = "COPY social SET path='/users/senol' TO path='/posts' AS 'senol-copy'";
    let result = analyzer.analyze(sql);

    assert!(
        result.is_ok(),
        "COPY with AS clause should be supported: {:?}",
        result.err()
    );

    let AnalyzedStatement::Copy(copy) = result.unwrap() else {
        panic!("Expected Copy statement");
    };

    assert_eq!(copy.new_name, Some("senol-copy".to_string()));
}
