//! Hash Aggregate Executor
//!
//! Implements hash-based aggregation with support for GROUP BY and aggregate functions.
//!
//! Algorithm:
//! 1. Build hash table: group key → accumulators
//! 2. For each input row:
//!    - Evaluate GROUP BY expressions to compute group key
//!    - Update accumulators for this group
//! 3. Finalize: convert accumulators to output rows

use super::eval::eval_expr;
use super::executor::{ExecutionContext, ExecutionError, Row, RowStream};
use super::operators::PhysicalPlan;
use futures::stream::{self, StreamExt};
use indexmap::IndexMap;
use raisin_models::nodes::properties::PropertyValue;
use raisin_sql::analyzer::{Literal, TypedExpr};
use raisin_sql::logical_plan::{AggregateExpr, AggregateFunction};
use raisin_storage::Storage;
use std::collections::HashMap;

/// Execute a hash aggregate operation
pub async fn execute_hash_aggregate<
    S: Storage + raisin_storage::transactional::TransactionalStorage + 'static,
>(
    plan: &PhysicalPlan,
    ctx: &ExecutionContext<S>,
) -> Result<RowStream, ExecutionError> {
    let (input, group_by, aggregates) = match plan {
        PhysicalPlan::HashAggregate {
            input,
            group_by,
            aggregates,
        } => (input, group_by, aggregates),
        _ => {
            return Err(ExecutionError::Backend(
                "Invalid plan passed to execute_hash_aggregate".to_string(),
            ))
        }
    };

    // Execute input
    let input_stream = super::executor::execute_plan(input.as_ref(), ctx).await?;

    // Materialize all input rows and build aggregation hash table
    // Store both the group key and the actual PropertyValues for GROUP BY columns
    let mut groups: HashMap<GroupKey, (Vec<PropertyValue>, Vec<Accumulator>)> = HashMap::new();

    let input_rows: Vec<_> = input_stream.collect().await;
    for row_result in input_rows {
        let row = row_result?;

        // Evaluate GROUP BY expressions to get group key and values
        let (group_key, group_values) = evaluate_group_key(group_by, &row)?;

        // Get or create accumulators for this group
        let (stored_values, accumulators) = groups.entry(group_key.clone()).or_insert_with(|| {
            let new_accumulators = aggregates
                .iter()
                .map(|agg| Accumulator::new(&agg.func))
                .collect();
            (group_values.clone(), new_accumulators)
        });

        // Update each accumulator with values from this row
        for (i, agg_expr) in aggregates.iter().enumerate() {
            // Check FILTER clause if present
            let should_include = if let Some(filter_expr) = &agg_expr.filter {
                let filter_result = eval_expr(filter_expr, &row)?;
                match filter_result {
                    Literal::Boolean(true) => true,
                    Literal::Boolean(false) | Literal::Null => false,
                    other => {
                        return Err(ExecutionError::Validation(format!(
                            "FILTER condition must evaluate to BOOLEAN, got {:?}",
                            other
                        )))
                    }
                }
            } else {
                true // No filter, include all rows
            };

            // Only update accumulator if filter passes
            if should_include {
                // Evaluate aggregate argument
                let value = if agg_expr.args.is_empty() {
                    // COUNT(*) - no argument
                    Literal::Int(1)
                } else {
                    eval_expr(&agg_expr.args[0], &row)?
                };

                accumulators[i].update(value)?;
            }
        }
    }

    // Finalize: convert groups to output rows
    let mut output_rows = Vec::new();
    for (_group_key, (group_values, accumulators)) in groups {
        let mut columns = IndexMap::new();

        // Add GROUP BY columns to output
        for (i, expr) in group_by.iter().enumerate() {
            if i < group_values.len() {
                let col_name = extract_column_name(expr).unwrap_or_else(|| format!("group_{}", i));
                tracing::debug!(
                    "HashAggregate: Storing GROUP BY column '{}' with value {:?}",
                    col_name,
                    group_values[i]
                );
                columns.insert(col_name, group_values[i].clone());
            }
        }

        // Add aggregate results
        for (i, accumulator) in accumulators.iter().enumerate() {
            let agg_expr = &aggregates[i];
            let result = accumulator.finalize()?;

            // Store with the user-provided alias
            columns.insert(agg_expr.alias.clone(), result.clone());

            // Also store with canonical name for lookup during projection evaluation
            let canonical_name = generate_canonical_aggregate_name(agg_expr);
            tracing::debug!(
                "HashAggregate: Storing aggregate '{}' (canonical: '{}') with value {:?}",
                agg_expr.alias,
                canonical_name,
                result
            );
            columns.insert(canonical_name, result);
        }

        output_rows.push(Row::from_map(columns));
    }

    // Convert to stream
    Ok(Box::pin(stream::iter(output_rows.into_iter().map(Ok))))
}

/// Group key for hash aggregation
/// We use a wrapper around PropertyValue strings for hashing
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct GroupKey {
    values: Vec<String>,
}

/// Evaluate GROUP BY expressions to compute group key and values
fn evaluate_group_key(
    group_by: &[TypedExpr],
    row: &Row,
) -> Result<(GroupKey, Vec<PropertyValue>), ExecutionError> {
    let mut key_strings = Vec::new();
    let mut prop_values = Vec::new();

    for expr in group_by {
        let literal = eval_expr(expr, row)?;
        // Convert to string for hashing
        let key_string = format!("{:?}", literal);
        key_strings.push(key_string);

        // Also store the PropertyValue for output
        let prop_value = literal_to_property_value(literal)?;
        prop_values.push(prop_value);
    }

    Ok((
        GroupKey {
            values: key_strings,
        },
        prop_values,
    ))
}

/// Convert Literal to PropertyValue
fn literal_to_property_value(lit: Literal) -> Result<PropertyValue, ExecutionError> {
    use super::types::to_property_value;
    to_property_value(&lit).map_err(ExecutionError::Backend)
}

/// Extract column name from expression (for GROUP BY columns in output)
fn extract_column_name(expr: &TypedExpr) -> Option<String> {
    use raisin_sql::analyzer::Expr;
    match &expr.expr {
        Expr::Column { table, column } => {
            // Use qualified name to avoid conflicts
            Some(format!("{}.{}", table, column))
        }
        Expr::JsonExtractText { object, key } => {
            // For JSON operators, create a synthetic name
            // e.g., properties->>'description' becomes properties_description
            if let Expr::Column { table, column } = &object.expr {
                if let Expr::Literal(raisin_sql::analyzer::Literal::Text(key_str)) = &key.expr {
                    return Some(format!("{}.{}_{}", table, column, key_str));
                }
            }
            None
        }
        Expr::Function { name, args, .. } => {
            // For function calls, generate a canonical name
            // e.g., DEPTH(default.path) becomes "DEPTH(default.path)"
            let func_name_upper = name.to_uppercase();

            if args.is_empty() {
                // No arguments (e.g., NOW())
                Some(format!("{}()", func_name_upper))
            } else if args.len() == 1 {
                // Single argument - try to extract column name
                if let Some(arg_name) = extract_column_name(&args[0]) {
                    Some(format!("{}({})", func_name_upper, arg_name))
                } else {
                    // Argument is a complex expression
                    Some(format!("{}(...)", func_name_upper))
                }
            } else {
                // Multiple arguments
                Some(format!("{}(...)", func_name_upper))
            }
        }
        _ => {
            // For other complex expressions, use a generic name
            // In production, we'd want to use the original SQL text or a hash
            None
        }
    }
}

/// Generate canonical name for aggregate function result
/// Must match the logic in eval.rs generate_function_column_name
fn generate_canonical_aggregate_name(agg_expr: &AggregateExpr) -> String {
    use raisin_sql::analyzer::Expr;
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let func_name_upper = match agg_expr.func {
        AggregateFunction::Count => "COUNT",
        AggregateFunction::CountDistinct => "COUNT",
        AggregateFunction::Sum => "SUM",
        AggregateFunction::Avg => "AVG",
        AggregateFunction::Min => "MIN",
        AggregateFunction::Max => "MAX",
        AggregateFunction::ArrayAgg => "ARRAY_AGG",
    };

    // Generate base name matching eval.rs format: FUNCTION_NAME(arg) or FUNCTION_NAME()
    let base_name = if agg_expr.args.is_empty() {
        // No arguments (e.g., COUNT(*) which is represented as empty args in some contexts)
        format!("{}()", func_name_upper)
    } else if agg_expr.args.len() == 1 {
        // Single argument - extract argument name
        let arg_name = match &agg_expr.args[0].expr {
            Expr::Column { table, column } => format!("{}.{}", table, column),
            _ => "...".to_string(),
        };
        format!("{}({})", func_name_upper, arg_name)
    } else {
        // Multiple arguments
        format!("{}(...)", func_name_upper)
    };

    // Include FILTER clause in canonical name to distinguish filtered aggregates
    if let Some(ref filter) = agg_expr.filter {
        // Hash the filter expression to create a unique suffix
        let mut hasher = DefaultHasher::new();
        format!("{:?}", filter).hash(&mut hasher);
        let filter_hash = hasher.finish();
        format!("{}_filter_{:x}", base_name, filter_hash)
    } else {
        base_name
    }
}

/// Accumulator for aggregate functions
#[derive(Debug, Clone)]
enum Accumulator {
    Count { count: usize },
    Sum { sum: f64, has_value: bool },
    Avg { sum: f64, count: usize },
    Min { min: Option<PropertyValue> },
    Max { max: Option<PropertyValue> },
    ArrayAgg { values: Vec<PropertyValue> },
}

impl Accumulator {
    /// Create new accumulator for the given aggregate function
    fn new(func: &AggregateFunction) -> Self {
        match func {
            AggregateFunction::Count | AggregateFunction::CountDistinct => {
                Accumulator::Count { count: 0 }
            }
            AggregateFunction::Sum => Accumulator::Sum {
                sum: 0.0,
                has_value: false,
            },
            AggregateFunction::Avg => Accumulator::Avg { sum: 0.0, count: 0 },
            AggregateFunction::Min => Accumulator::Min { min: None },
            AggregateFunction::Max => Accumulator::Max { max: None },
            AggregateFunction::ArrayAgg => Accumulator::ArrayAgg { values: Vec::new() },
        }
    }

    /// Update accumulator with a new value
    fn update(&mut self, value: Literal) -> Result<(), ExecutionError> {
        match self {
            Accumulator::Count { count } => {
                *count += 1;
            }
            Accumulator::Sum { sum, has_value } => {
                if let Some(num) = extract_number(&value) {
                    *sum += num;
                    *has_value = true;
                }
            }
            Accumulator::Avg { sum, count } => {
                if let Some(num) = extract_number(&value) {
                    *sum += num;
                    *count += 1;
                }
            }
            Accumulator::Min { min } => {
                let prop_value = literal_to_property_value(value)?;
                // Simple comparison - we compare the debug string representation
                let should_update = if let Some(current_min) = min {
                    format!("{:?}", prop_value) < format!("{:?}", current_min)
                } else {
                    true
                };
                if should_update {
                    *min = Some(prop_value);
                }
            }
            Accumulator::Max { max } => {
                let prop_value = literal_to_property_value(value)?;
                // Simple comparison - we compare the debug string representation
                let should_update = if let Some(current_max) = max {
                    format!("{:?}", prop_value) > format!("{:?}", current_max)
                } else {
                    true
                };
                if should_update {
                    *max = Some(prop_value);
                }
            }
            Accumulator::ArrayAgg { values } => {
                let prop_value = literal_to_property_value(value)?;
                values.push(prop_value);
            }
        }
        Ok(())
    }

    /// Finalize accumulator and return result
    fn finalize(&self) -> Result<PropertyValue, ExecutionError> {
        match self {
            Accumulator::Count { count } => Ok(PropertyValue::Integer(*count as i64)),
            Accumulator::Sum { sum, has_value } => {
                // Return 0 if no values, following PostgreSQL behavior
                Ok(PropertyValue::Float(if *has_value { *sum } else { 0.0 }))
            }
            Accumulator::Avg { sum, count } => {
                // Return 0 if no values
                if *count > 0 {
                    Ok(PropertyValue::Float(sum / (*count as f64)))
                } else {
                    Ok(PropertyValue::Float(0.0))
                }
            }
            Accumulator::Min { min } => {
                // Return Float(0.0) as a default for now
                // In production we'd want Option<PropertyValue> return type
                Ok(min.clone().unwrap_or(PropertyValue::Float(0.0)))
            }
            Accumulator::Max { max } => {
                // Return Float(0.0) as a default for now
                Ok(max.clone().unwrap_or(PropertyValue::Float(0.0)))
            }
            Accumulator::ArrayAgg { values } => {
                // Convert Vec<PropertyValue> to PropertyValue::Array
                Ok(PropertyValue::Array(values.clone()))
            }
        }
    }
}

/// Extract numeric value from literal
fn extract_number(lit: &Literal) -> Option<f64> {
    match lit {
        Literal::Int(i) => Some(*i as f64),
        Literal::BigInt(i) => Some(*i as f64),
        Literal::Double(f) => Some(*f),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_count_accumulator() {
        let mut acc = Accumulator::new(&AggregateFunction::Count);
        acc.update(Literal::Int(1)).unwrap();
        acc.update(Literal::Int(2)).unwrap();
        acc.update(Literal::Int(3)).unwrap();

        let result = acc.finalize().unwrap();
        assert_eq!(result, PropertyValue::Integer(3));
    }

    #[test]
    fn test_sum_accumulator() {
        let mut acc = Accumulator::new(&AggregateFunction::Sum);
        acc.update(Literal::Int(1)).unwrap();
        acc.update(Literal::Int(2)).unwrap();
        acc.update(Literal::Int(3)).unwrap();

        let result = acc.finalize().unwrap();
        assert_eq!(result, PropertyValue::Float(6.0));
    }

    #[test]
    fn test_avg_accumulator() {
        let mut acc = Accumulator::new(&AggregateFunction::Avg);
        acc.update(Literal::Int(1)).unwrap();
        acc.update(Literal::Int(2)).unwrap();
        acc.update(Literal::Int(3)).unwrap();

        let result = acc.finalize().unwrap();
        assert_eq!(result, PropertyValue::Float(2.0));
    }

    #[test]
    fn test_array_agg_accumulator() {
        let mut acc = Accumulator::new(&AggregateFunction::ArrayAgg);
        acc.update(Literal::Text("a".to_string())).unwrap();
        acc.update(Literal::Text("b".to_string())).unwrap();
        acc.update(Literal::Text("c".to_string())).unwrap();

        let result = acc.finalize().unwrap();
        match result {
            PropertyValue::Array(arr) => {
                assert_eq!(arr.len(), 3);
                assert_eq!(arr[0], PropertyValue::String("a".to_string()));
                assert_eq!(arr[1], PropertyValue::String("b".to_string()));
                assert_eq!(arr[2], PropertyValue::String("c".to_string()));
            }
            _ => panic!("Expected array"),
        }
    }
}
