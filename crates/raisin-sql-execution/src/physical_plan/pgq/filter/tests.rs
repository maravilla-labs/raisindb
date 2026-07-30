//! Tests for the filter module.

#[cfg(test)]
mod tests {
    use raisin_sql::ast::{BinaryOperator, Expr, Literal};

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
            evaluate_literal(&Literal::Float(3.25)),
            SqlValue::Float(3.25)
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

    /// `CARDINALITY(r)` used to be answered by parsing `"FRIENDS_WITH[2]"`, a
    /// hop count smuggled through the relation type because nothing else
    /// carried it. The real path is bound now, so this asserts the count comes
    /// from the path AND that the relation type is left verbatim.
    #[test]
    fn cardinality_reads_the_bound_path_and_leaves_relation_type_alone() {
        use crate::physical_plan::graph_algo::GraphEdge;
        use crate::physical_plan::pgq::filter::functions::evaluate_function;
        use crate::physical_plan::pgq::matching::{GraphPath, PathNode};
        use crate::physical_plan::pgq::types::{RelationInfo, VariableBinding};
        use raisin_sql::ast::SourceSpan;

        fn hop(mut path: GraphPath, tgt: &str) -> GraphPath {
            path.push(&GraphEdge::new("ws", tgt, "User", "FRIENDS_WITH", None));
            path
        }

        fn arg(name: &str) -> Vec<Expr> {
            vec![Expr::PropertyAccess {
                variable: name.into(),
                properties: vec![],
                span: SourceSpan::empty(),
            }]
        }

        let mut binding = VariableBinding::new();

        // Two-hop path a -> b -> c.
        let two = hop(
            hop(GraphPath::start(PathNode::new("a", "ws", "User")), "b"),
            "c",
        );
        binding.bind_path("r".into(), two);
        binding.bind_relation(
            "r".into(),
            RelationInfo::new("FRIENDS_WITH".into(), None, "a".into(), "b".into()),
        );
        assert_eq!(
            evaluate_function("CARDINALITY", &arg("r"), &binding).unwrap(),
            SqlValue::Integer(2)
        );
        assert_eq!(
            binding.get_relation("r").unwrap().relation_type,
            "FRIENDS_WITH"
        );

        // Three-hop path.
        let three = hop(
            hop(
                hop(GraphPath::start(PathNode::new("a", "ws", "User")), "b"),
                "c",
            ),
            "d",
        );
        binding.bind_path("r2".into(), three);
        assert_eq!(
            evaluate_function("cardinality", &arg("r2"), &binding).unwrap(),
            SqlValue::Integer(3)
        );

        // Single-hop relationship: no path bound, cardinality 1.
        binding.bind_relation(
            "r3".into(),
            RelationInfo::new("FOLLOWS".into(), None, "x".into(), "y".into()),
        );
        assert_eq!(
            evaluate_function("CARDINALITY", &arg("r3"), &binding).unwrap(),
            SqlValue::Integer(1)
        );
    }
}
