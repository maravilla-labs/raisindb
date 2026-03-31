//! Function call constant folding

use crate::analyzer::{Expr, Literal, TypedExpr};
use crate::optimizer::hierarchy_rewrite::{compute_depth, compute_parent_path};

/// Fold a function call with literal arguments
pub(super) fn fold_function(name: &str, args: &[TypedExpr]) -> Option<TypedExpr> {
    let name_upper = name.to_uppercase();

    match name_upper.as_str() {
        // Hierarchy functions
        "DEPTH" if args.len() == 1 => fold_depth(args),
        "PARENT" if args.len() == 1 => fold_parent(args),
        "PATH_STARTS_WITH" if args.len() == 2 => fold_path_starts_with(args),

        // String functions
        "LOWER" if args.len() == 1 => fold_lower(args),
        "UPPER" if args.len() == 1 => fold_upper(args),
        "LENGTH" if args.len() == 1 => fold_length(args),

        _ => None,
    }
}

fn fold_depth(args: &[TypedExpr]) -> Option<TypedExpr> {
    if let Expr::Literal(lit) = &args[0].expr {
        let path = match lit {
            Literal::Path(p) => p,
            Literal::Text(t) => t,
            _ => return None,
        };
        let depth = compute_depth(path);
        return Some(TypedExpr::literal(Literal::Int(depth)));
    }
    None
}

fn fold_parent(args: &[TypedExpr]) -> Option<TypedExpr> {
    if let Expr::Literal(lit) = &args[0].expr {
        let path = match lit {
            Literal::Path(p) => p,
            Literal::Text(t) => t,
            _ => return None,
        };
        if let Some(parent) = compute_parent_path(path) {
            return Some(TypedExpr::literal(Literal::Path(parent)));
        } else {
            return Some(TypedExpr::literal(Literal::Null));
        }
    }
    None
}

fn fold_path_starts_with(args: &[TypedExpr]) -> Option<TypedExpr> {
    if let (Expr::Literal(lit1), Expr::Literal(lit2)) = (&args[0].expr, &args[1].expr) {
        let path = match lit1 {
            Literal::Path(p) => p,
            Literal::Text(t) => t,
            _ => return None,
        };
        let prefix = match lit2 {
            Literal::Path(p) => p,
            Literal::Text(t) => t,
            _ => return None,
        };
        return Some(TypedExpr::literal(Literal::Boolean(
            path.starts_with(prefix),
        )));
    }
    None
}

fn fold_lower(args: &[TypedExpr]) -> Option<TypedExpr> {
    if let Expr::Literal(Literal::Text(s)) = &args[0].expr {
        return Some(TypedExpr::literal(Literal::Text(s.to_lowercase())));
    }
    None
}

fn fold_upper(args: &[TypedExpr]) -> Option<TypedExpr> {
    if let Expr::Literal(Literal::Text(s)) = &args[0].expr {
        return Some(TypedExpr::literal(Literal::Text(s.to_uppercase())));
    }
    None
}

fn fold_length(args: &[TypedExpr]) -> Option<TypedExpr> {
    if let Expr::Literal(Literal::Text(s)) = &args[0].expr {
        return Some(TypedExpr::literal(Literal::Int(s.len() as i32)));
    }
    None
}
