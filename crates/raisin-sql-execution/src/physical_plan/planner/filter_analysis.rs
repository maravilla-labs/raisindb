//! Filter and predicate analysis
//!
//! Canonicalizes filter expressions into structured predicates and provides
//! utilities for extracting, removing, and combining predicates.

use super::{
    literal_to_json, CanonicalPredicate, ComparisonOp, Error, Expr, Literal, PhysicalPlanner,
    ScanReason, TypedExpr,
};
use raisin_sql::analyzer::{BinaryOperator, DataType};

impl PhysicalPlanner {
    pub(super) fn determine_scan_reason(&self, canonical: &[CanonicalPredicate]) -> ScanReason {
        let available = self.index_catalog.available_indexes();

        if available.is_empty() {
            return ScanReason::NoIndexAvailable;
        }

        // Check if we have predicates that could use indexes
        let has_prefix = canonical
            .iter()
            .any(|p| matches!(p, CanonicalPredicate::PrefixRange { .. }));
        let has_child_of = canonical
            .iter()
            .any(|p| matches!(p, CanonicalPredicate::ChildOf { .. }));
        let has_descendant_of = canonical
            .iter()
            .any(|p| matches!(p, CanonicalPredicate::DescendantOf { .. }));
        let has_property = canonical
            .iter()
            .any(|p| matches!(p, CanonicalPredicate::JsonPropertyEq { .. }));

        // Check path index requirements for descendant_of
        if has_descendant_of && !self.index_catalog.has_path_index() {
            return ScanReason::IndexNotEnabled {
                index_name: "path_index".to_string(),
            };
        }
        let _ = has_child_of; // Suppress unused warning - CHILD_OF uses ordered_children which is always available

        if has_prefix && !self.index_catalog.has_path_index() {
            return ScanReason::IndexNotEnabled {
                index_name: "path_index".to_string(),
            };
        }

        if has_property && !self.index_catalog.has_property_index() {
            return ScanReason::IndexNotEnabled {
                index_name: "property_index".to_string(),
            };
        }

        // No predicates that can use indexes
        if canonical
            .iter()
            .all(|p| matches!(p, CanonicalPredicate::Other(_)))
        {
            return ScanReason::UnsupportedPredicate {
                details: "no indexable predicates found".to_string(),
            };
        }

        ScanReason::NoMatchingIndex { available }
    }

    /// Analyze filter expression into canonical predicates
    pub(super) fn analyze_filter(
        &self,
        filter: &TypedExpr,
    ) -> Result<Vec<CanonicalPredicate>, Error> {
        // For now, we'll do simple pattern matching
        // In a full implementation, this would use the optimizer's hierarchy_rewrite module

        let mut predicates = Vec::new();

        // Flatten AND operations
        let conjuncts = self.flatten_ands(filter);

        for conjunct in conjuncts {
            if let Some(pred) = self.match_canonical_predicate(&conjunct) {
                predicates.push(pred);
            } else {
                predicates.push(CanonicalPredicate::Other(conjunct));
            }
        }

        Ok(predicates)
    }

    /// Match a single expression to a canonical predicate
    pub(super) fn match_canonical_predicate(&self, expr: &TypedExpr) -> Option<CanonicalPredicate> {
        match &expr.expr {
            // PATH_STARTS_WITH(path, prefix)
            Expr::Function { name, args, .. } if name.to_uppercase() == "PATH_STARTS_WITH" => {
                if args.len() == 2 {
                    if let (Expr::Column { table, column }, Expr::Literal(Literal::Path(prefix)))
                    | (Expr::Column { table, column }, Expr::Literal(Literal::Text(prefix))) =
                        (&args[0].expr, &args[1].expr)
                    {
                        return Some(CanonicalPredicate::PrefixRange {
                            table: table.clone(),
                            path_col: column.clone(),
                            prefix: prefix.clone(),
                        });
                    }
                }
            }

            // CHILD_OF(parent_path) - direct children scan
            Expr::Function { name, args, .. } if name.to_uppercase() == "CHILD_OF" => {
                if args.len() == 1 {
                    if let Expr::Literal(lit) = &args[0].expr {
                        let parent_path = match lit {
                            Literal::Path(p) => p.clone(),
                            Literal::Text(t) => t.clone(),
                            _ => return None,
                        };
                        return Some(CanonicalPredicate::ChildOf { parent_path });
                    }
                }
            }

            // DESCENDANT_OF(parent_path [, max_depth]) - descendants scan
            Expr::Function { name, args, .. } if name.to_uppercase() == "DESCENDANT_OF" => {
                if !args.is_empty() && args.len() <= 2 {
                    if let Expr::Literal(lit) = &args[0].expr {
                        let parent_path = match lit {
                            Literal::Path(p) => p.clone(),
                            Literal::Text(t) => t.clone(),
                            _ => return None,
                        };

                        // Extract optional max_depth parameter
                        let max_depth = if args.len() == 2 {
                            match &args[1].expr {
                                Expr::Literal(Literal::Int(n)) => Some(*n as i64),
                                Expr::Literal(Literal::BigInt(n)) => Some(*n),
                                Expr::Literal(Literal::Null) => None,
                                _ => return None,
                            }
                        } else {
                            None
                        };

                        return Some(CanonicalPredicate::DescendantOf {
                            parent_path,
                            max_depth,
                        });
                    }
                }
            }

            // ST_DWithin(geometry, ST_Point(lon, lat), radius) - spatial proximity scan
            Expr::Function { name, args, .. } if name.to_uppercase() == "ST_DWITHIN" => {
                if args.len() == 3 {
                    // Extract geometry column/property access
                    // Supports: properties->>'loc', properties->'loc', CAST(... AS GEOMETRY), column
                    let (table, geometry_column, property_name) = match
                        raisin_sql::optimizer::hierarchy_rewrite::extract_geometry_source(
                            &args[0].expr,
                        ) {
                        Some(v) => v,
                        None => {
                            tracing::debug!(
                                "ST_DWITHIN spatial index skipped: could not extract geometry source from first argument"
                            );
                            return None;
                        }
                    };

                    // Extract center point from ST_Point(lon, lat)
                    let (center_lon, center_lat) = match &args[1].expr {
                        Expr::Function {
                            name,
                            args: point_args,
                            ..
                        } if name.to_uppercase() == "ST_POINT"
                            || name.to_uppercase() == "ST_MAKEPOINT" =>
                        {
                            if point_args.len() == 2 {
                                let lon = match &point_args[0].expr {
                                    Expr::Literal(Literal::Double(f)) => *f,
                                    Expr::Literal(Literal::Int(i)) => *i as f64,
                                    Expr::Literal(Literal::BigInt(i)) => *i as f64,
                                    _ => return None,
                                };
                                let lat = match &point_args[1].expr {
                                    Expr::Literal(Literal::Double(f)) => *f,
                                    Expr::Literal(Literal::Int(i)) => *i as f64,
                                    Expr::Literal(Literal::BigInt(i)) => *i as f64,
                                    _ => return None,
                                };
                                (lon, lat)
                            } else {
                                return None;
                            }
                        }
                        _ => {
                            tracing::debug!(
                                "ST_DWITHIN spatial index skipped: second argument is not a literal ST_POINT/ST_MAKEPOINT"
                            );
                            return None;
                        }
                    };

                    // Extract radius in meters
                    let radius_meters = match &args[2].expr {
                        Expr::Literal(Literal::Double(f)) => *f,
                        Expr::Literal(Literal::Int(i)) => *i as f64,
                        Expr::Literal(Literal::BigInt(i)) => *i as f64,
                        _ => {
                            tracing::debug!(
                                "ST_DWITHIN spatial index skipped: radius (third argument) is not a numeric literal"
                            );
                            return None;
                        }
                    };

                    return Some(CanonicalPredicate::SpatialDWithin {
                        table,
                        geometry_column,
                        property_name,
                        center_lon,
                        center_lat,
                        radius_meters,
                    });
                }
            }

            // DEPTH(path) = value
            Expr::BinaryOp {
                left,
                op: BinaryOperator::Eq,
                right,
            } => {
                if let Expr::Function { name, args, .. } = &left.expr {
                    if name.to_uppercase() == "DEPTH" && args.len() == 1 {
                        if let Expr::Column { table, column } = &args[0].expr {
                            if let Expr::Literal(Literal::Int(depth_val)) = &right.expr {
                                return Some(CanonicalPredicate::DepthEq {
                                    table: table.clone(),
                                    path_col: column.clone(),
                                    depth_value: *depth_val,
                                });
                            }
                        }
                    }
                }

                // JSON property: properties->>'key' = 'value'
                if let Expr::JsonExtractText { object, key } = &left.expr {
                    if let (Expr::Column { table, column }, Expr::Literal(Literal::Text(key_str))) =
                        (&object.expr, &key.expr)
                    {
                        if let Expr::Literal(lit) = &right.expr {
                            if let Ok(json_val) = literal_to_json(lit) {
                                return Some(CanonicalPredicate::JsonPropertyEq {
                                    table: table.clone(),
                                    json_col: column.clone(),
                                    key: key_str.clone(),
                                    value: json_val,
                                });
                            }
                        }
                    }
                }

                // JSON property via $.syntax: $.properties.key::TEXT = 'value'
                // This is Cast { expr: JsonExtractPath { object, path }, target_type: Text }
                if let Expr::Cast {
                    expr: cast_expr,
                    target_type: DataType::Text,
                } = &left.expr
                {
                    if let Expr::JsonExtractPath { object, path } = &cast_expr.expr {
                        // Check if object is Cast { Column { properties }, target: JsonB }
                        if let Expr::Cast {
                            expr: inner_expr,
                            target_type: DataType::JsonB,
                        } = &object.expr
                        {
                            if let Expr::Column { table, column } = &inner_expr.expr {
                                // Extract the path key - for single-element paths like $.properties.email
                                // the path is a JsonB literal containing an Array of strings
                                if let Expr::Literal(Literal::JsonB(serde_json::Value::Array(
                                    elements,
                                ))) = &path.expr
                                {
                                    if elements.len() == 1 {
                                        if let serde_json::Value::String(key_str) = &elements[0] {
                                            if let Expr::Literal(lit) = &right.expr {
                                                if let Ok(json_val) = literal_to_json(lit) {
                                                    return Some(
                                                        CanonicalPredicate::JsonPropertyEq {
                                                            table: table.clone(),
                                                            json_col: column.clone(),
                                                            key: key_str.clone(),
                                                            value: json_val,
                                                        },
                                                    );
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // column = value (literal)
                if let (Expr::Column { table, column }, Expr::Literal(_)) =
                    (&left.expr, &right.expr)
                {
                    return Some(CanonicalPredicate::ColumnEq {
                        table: table.clone(),
                        column: column.clone(),
                        value: (**right).clone(),
                    });
                }

                // literal = column (reverse case)
                if let (Expr::Literal(_), Expr::Column { table, column }) =
                    (&left.expr, &right.expr)
                {
                    return Some(CanonicalPredicate::ColumnEq {
                        table: table.clone(),
                        column: column.clone(),
                        value: (**left).clone(),
                    });
                }

                // column = constant_expr (e.g., created_at = now())
                // Handle timestamp columns with constant expressions
                if let Expr::Column { table, column } = &left.expr {
                    let col_lower = column.to_lowercase();
                    if (col_lower == "created_at" || col_lower == "updated_at")
                        && self.is_constant_expr(right)
                    {
                        return Some(CanonicalPredicate::ColumnEq {
                            table: table.clone(),
                            column: column.clone(),
                            value: (**right).clone(),
                        });
                    }
                }

                // constant_expr = column (e.g., now() = created_at) - reverse case
                // Handle timestamp columns with constant expressions
                if let Expr::Column { table, column } = &right.expr {
                    let col_lower = column.to_lowercase();
                    if (col_lower == "created_at" || col_lower == "updated_at")
                        && self.is_constant_expr(left)
                    {
                        return Some(CanonicalPredicate::ColumnEq {
                            table: table.clone(),
                            column: column.clone(),
                            value: (**left).clone(),
                        });
                    }
                }
            }

            // Comparison operators: >, <, >=, <= → RangeCompare
            // Supports both literals and constant expressions like now()
            Expr::BinaryOp {
                left,
                op:
                    op @ (BinaryOperator::Gt
                    | BinaryOperator::GtEq
                    | BinaryOperator::Lt
                    | BinaryOperator::LtEq),
                right,
            } => {
                if let Some(comp_op) = ComparisonOp::from_binary_op(op) {
                    // Pattern: column OP value (e.g., created_at > now())
                    if let Expr::Column { table, column } = &left.expr {
                        // Check if right side is a constant expression
                        if self.is_constant_expr(right) {
                            return Some(CanonicalPredicate::RangeCompare {
                                table: table.clone(),
                                column: column.clone(),
                                op: comp_op,
                                value: (**right).clone(),
                            });
                        }
                    }

                    // Pattern: value OP column (e.g., now() < created_at) - reverse the operator
                    if let Expr::Column { table, column } = &right.expr {
                        if self.is_constant_expr(left) {
                            return Some(CanonicalPredicate::RangeCompare {
                                table: table.clone(),
                                column: column.clone(),
                                op: comp_op.reverse(),
                                value: (**left).clone(),
                            });
                        }
                    }
                }
            }

            // LIKE pattern: column LIKE 'prefix%'
            // This can be optimized to a prefix scan when the pattern is a prefix match
            Expr::Like {
                expr,
                pattern,
                negated,
            } => {
                // Only handle positive LIKE (not negated)
                if !negated {
                    if let Expr::Column { table, column } = &expr.expr {
                        if let Expr::Literal(Literal::Text(pattern_str)) = &pattern.expr {
                            // Check if this is a prefix pattern (ends with %)
                            if pattern_str.ends_with('%')
                                && !pattern_str[..pattern_str.len() - 1].contains(['%', '_'])
                            {
                                // This is a simple prefix pattern like 'value%'
                                let prefix = &pattern_str[..pattern_str.len() - 1];
                                let col_lower = column.to_lowercase();

                                // For path columns, use PrefixRange (path index)
                                if col_lower == "path" {
                                    return Some(CanonicalPredicate::PrefixRange {
                                        table: table.clone(),
                                        path_col: column.clone(),
                                        prefix: prefix.to_string(),
                                    });
                                }

                                // For indexed property columns, use PropertyPrefixRange
                                // This includes: node_type, and any JSON property
                                if col_lower == "node_type" {
                                    return Some(CanonicalPredicate::PropertyPrefixRange {
                                        table: table.clone(),
                                        column: column.clone(),
                                        prefix: prefix.to_string(),
                                    });
                                }
                            }
                        }
                    }
                }

                // Also handle JSON property LIKE: properties->>'key' LIKE 'prefix%'
                if !negated {
                    if let Expr::JsonExtractText { object, key } = &expr.expr {
                        if let Expr::Column { table, column: _ } = &object.expr {
                            if let Expr::Literal(Literal::Text(key_str)) = &key.expr {
                                if let Expr::Literal(Literal::Text(pattern_str)) = &pattern.expr {
                                    // Check if this is a prefix pattern (ends with %)
                                    if pattern_str.ends_with('%')
                                        && !pattern_str[..pattern_str.len() - 1]
                                            .contains(['%', '_'])
                                    {
                                        let prefix = &pattern_str[..pattern_str.len() - 1];
                                        // Use the JSON key as the column for property prefix scan
                                        return Some(CanonicalPredicate::PropertyPrefixRange {
                                            table: table.clone(),
                                            column: key_str.clone(),
                                            prefix: prefix.to_string(),
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }

            _ => {}
        }

        None
    }

    /// Flatten AND operations
    #[allow(clippy::only_used_in_recursion)]
    pub(super) fn flatten_ands(&self, expr: &TypedExpr) -> Vec<TypedExpr> {
        match &expr.expr {
            Expr::BinaryOp {
                left,
                op: BinaryOperator::And,
                right,
            } => {
                let mut result = self.flatten_ands(left);
                result.extend(self.flatten_ands(right));
                result
            }
            _ => vec![expr.clone()],
        }
    }
}
