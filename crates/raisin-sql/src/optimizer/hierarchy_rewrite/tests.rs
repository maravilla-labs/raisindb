//! Tests for hierarchy rewriting

#[cfg(test)]
mod tests {
    use crate::analyzer::functions::{FunctionCategory, FunctionSignature};
    use crate::analyzer::{BinaryOperator, DataType, Expr, Literal, TypedExpr};
    use crate::optimizer::hierarchy_rewrite::{
        compute_depth, compute_parent_path, rewrite_hierarchy_predicates, CanonicalPredicate,
    };

    #[test]
    fn test_rewrite_path_starts_with() {
        let col = TypedExpr::column("nodes".into(), "path".into(), DataType::Path);
        let pfx = TypedExpr::literal(Literal::Path("/content/".into()));
        let f = TypedExpr::new(
            Expr::Function {
                name: "PATH_STARTS_WITH".into(),
                args: vec![col, pfx],
                signature: FunctionSignature {
                    name: "PATH_STARTS_WITH".into(),
                    params: vec![DataType::Path, DataType::Path],
                    return_type: DataType::Boolean,
                    is_deterministic: true,
                    category: FunctionCategory::Hierarchy,
                },
                filter: None,
            },
            DataType::Boolean,
        );
        let p = rewrite_hierarchy_predicates(f);
        assert_eq!(p.len(), 1);
        assert!(
            matches!(&p[0], CanonicalPredicate::PrefixRange { table, path_col, prefix } if table == "nodes" && path_col == "path" && prefix == "/content/")
        );
    }

    #[test]
    fn test_rewrite_depth_eq() {
        let col = TypedExpr::column("nodes".into(), "path".into(), DataType::Path);
        let df = TypedExpr::new(
            Expr::Function {
                name: "DEPTH".into(),
                args: vec![col],
                signature: FunctionSignature {
                    name: "DEPTH".into(),
                    params: vec![DataType::Path],
                    return_type: DataType::Int,
                    is_deterministic: true,
                    category: FunctionCategory::Hierarchy,
                },
                filter: None,
            },
            DataType::Int,
        );
        let eq = TypedExpr::new(
            Expr::BinaryOp {
                left: Box::new(df),
                op: BinaryOperator::Eq,
                right: Box::new(TypedExpr::literal(Literal::Int(2))),
            },
            DataType::Boolean,
        );
        let p = rewrite_hierarchy_predicates(eq);
        assert_eq!(p.len(), 1);
        assert!(
            matches!(&p[0], CanonicalPredicate::DepthEq { table, path_col, depth_value } if table == "nodes" && path_col == "path" && *depth_value == 2)
        );
    }

    #[test]
    fn test_rewrite_child_of() {
        let pp = TypedExpr::literal(Literal::Path("/content/blog".into()));
        let f = TypedExpr::new(
            Expr::Function {
                name: "CHILD_OF".into(),
                args: vec![pp],
                signature: FunctionSignature {
                    name: "CHILD_OF".into(),
                    params: vec![DataType::Path],
                    return_type: DataType::Boolean,
                    is_deterministic: true,
                    category: FunctionCategory::Hierarchy,
                },
                filter: None,
            },
            DataType::Boolean,
        );
        let p = rewrite_hierarchy_predicates(f);
        assert_eq!(p.len(), 1);
        assert!(
            matches!(&p[0], CanonicalPredicate::ChildOf { parent_path } if parent_path == "/content/blog")
        );
    }

    #[test]
    fn test_compute_depth() {
        assert_eq!(compute_depth("/"), 0);
        assert_eq!(compute_depth("/content"), 1);
        assert_eq!(compute_depth("/content/blog"), 2);
    }

    #[test]
    fn test_compute_parent_path() {
        assert_eq!(compute_parent_path("/"), None);
        assert_eq!(compute_parent_path("/content"), Some("/".into()));
        assert_eq!(
            compute_parent_path("/content/blog"),
            Some("/content".into())
        );
    }

    #[test]
    fn test_to_expr() {
        let pred = CanonicalPredicate::PrefixRange {
            table: "nodes".into(),
            path_col: "path".into(),
            prefix: "/content/".into(),
        };
        let expr = pred.to_expr();
        assert!(matches!(expr.expr, Expr::Function { .. }));
    }
}
