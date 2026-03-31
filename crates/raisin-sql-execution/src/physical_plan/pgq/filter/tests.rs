//! Tests for the filter module.

#[cfg(test)]
mod tests {
    use raisin_sql::ast::{BinaryOperator, Expr, Literal};

    use crate::physical_plan::pgq::filter::functions::extract_path_length;
    use crate::physical_plan::pgq::filter::like_match::like_match;
    use crate::physical_plan::pgq::filter::operators::{
        compare_values, evaluate_binary_op, values_equal,
    };
    use crate::physical_plan::pgq::types::SqlValue;

    fn evaluate_literal(lit: &Literal) -> SqlValue {
        match lit {
            Literal::String(s) => SqlValue::String(s.clone()),
            Literal::Integer(i) => SqlValue::Integer(*i),
            Literal::Float(f) => SqlValue::Float(*f),
            Literal::Boolean(b) => SqlValue::Boolean(*b),
            Literal::Null => SqlValue::Null,
        }
    }

    #[test]
    fn test_evaluate_literal() {
        assert_eq!(
            evaluate_literal(&Literal::String("hello".into())),
            SqlValue::String("hello".into())
        );
        assert_eq!(
            evaluate_literal(&Literal::Integer(42)),
            SqlValue::Integer(42)
        );
        assert_eq!(
            evaluate_literal(&Literal::Float(3.14)),
            SqlValue::Float(3.14)
        );
        assert_eq!(
            evaluate_literal(&Literal::Boolean(true)),
            SqlValue::Boolean(true)
        );
        assert_eq!(evaluate_literal(&Literal::Null), SqlValue::Null);
    }

    #[test]
    fn test_values_equal() {
        assert!(values_equal(&SqlValue::Integer(42), &SqlValue::Integer(42)));
        assert!(!values_equal(
            &SqlValue::Integer(42),
            &SqlValue::Integer(43)
        ));
        assert!(values_equal(
            &SqlValue::String("hello".into()),
            &SqlValue::String("hello".into())
        ));
        assert!(values_equal(&SqlValue::Integer(42), &SqlValue::Float(42.0)));
    }

    #[test]
    fn test_compare_values() {
        assert_eq!(
            compare_values(&SqlValue::Integer(1), &SqlValue::Integer(2)),
            Some(std::cmp::Ordering::Less)
        );
        assert_eq!(
            compare_values(&SqlValue::Float(2.0), &SqlValue::Integer(1)),
            Some(std::cmp::Ordering::Greater)
        );
    }

    #[test]
    fn test_like_match() {
        assert!(like_match("hello", "hello"));
        assert!(like_match("hello", "%"));
        assert!(like_match("hello", "h%"));
        assert!(like_match("hello", "%o"));
        assert!(like_match("hello", "h%o"));
        assert!(like_match("hello", "h_llo"));
        assert!(!like_match("hello", "h_lo"));
        assert!(like_match("hello world", "%world"));
        assert!(like_match("hello", "HELLO")); // case insensitive
    }

    #[test]
    fn test_binary_ops() {
        // Comparison
        assert_eq!(
            evaluate_binary_op(
                BinaryOperator::Eq,
                SqlValue::Integer(1),
                SqlValue::Integer(1)
            )
            .unwrap(),
            SqlValue::Boolean(true)
        );

        // Arithmetic
        assert_eq!(
            evaluate_binary_op(
                BinaryOperator::Plus,
                SqlValue::Integer(2),
                SqlValue::Integer(3)
            )
            .unwrap(),
            SqlValue::Integer(5)
        );

        // Logical
        assert_eq!(
            evaluate_binary_op(
                BinaryOperator::And,
                SqlValue::Boolean(true),
                SqlValue::Boolean(false)
            )
            .unwrap(),
            SqlValue::Boolean(false)
        );
    }

    #[test]
    fn test_extract_path_length() {
        // Variable-length path with encoded length
        assert_eq!(extract_path_length("FRIENDS_WITH[2]"), Some(2));
        assert_eq!(extract_path_length("FRIENDS_WITH[3]"), Some(3));
        assert_eq!(extract_path_length("FOLLOWS[10]"), Some(10));

        // Single-hop path without encoding
        assert_eq!(extract_path_length("FRIENDS_WITH"), None);
        assert_eq!(extract_path_length("FOLLOWS"), None);

        // Edge cases
        assert_eq!(extract_path_length("TYPE[0]"), Some(0));
        assert_eq!(extract_path_length(""), None);
        assert_eq!(extract_path_length("[1]"), Some(1));
    }

    #[test]
    fn test_cardinality_function() {
        use crate::physical_plan::pgq::filter::functions::evaluate_function;
        use crate::physical_plan::pgq::types::{RelationInfo, VariableBinding};
        use raisin_sql::ast::SourceSpan;

        let mut binding = VariableBinding::new();
        binding.bind_relation(
            "r".into(),
            RelationInfo::new("FRIENDS_WITH[2]".into(), None, "a".into(), "b".into()),
        );

        // CARDINALITY(r) should return 2 for a 2-hop path
        let args = vec![Expr::PropertyAccess {
            variable: "r".into(),
            properties: vec![],
            span: SourceSpan::empty(),
        }];
        let result = evaluate_function("CARDINALITY", &args, &binding).unwrap();
        assert_eq!(result, SqlValue::Integer(2));

        // Test with 3-hop path
        binding.bind_relation(
            "r2".into(),
            RelationInfo::new("FRIENDS_WITH[3]".into(), None, "a".into(), "c".into()),
        );
        let args2 = vec![Expr::PropertyAccess {
            variable: "r2".into(),
            properties: vec![],
            span: SourceSpan::empty(),
        }];
        let result2 = evaluate_function("cardinality", &args2, &binding).unwrap();
        assert_eq!(result2, SqlValue::Integer(3));

        // Single-hop relationship (no encoding) should return 1
        binding.bind_relation(
            "r3".into(),
            RelationInfo::new("FOLLOWS".into(), None, "x".into(), "y".into()),
        );
        let args3 = vec![Expr::PropertyAccess {
            variable: "r3".into(),
            properties: vec![],
            span: SourceSpan::empty(),
        }];
        let result3 = evaluate_function("CARDINALITY", &args3, &binding).unwrap();
        assert_eq!(result3, SqlValue::Integer(1));
    }
}
