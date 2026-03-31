//! Accumulator types for aggregate functions
//!
//! This module provides memory-efficient accumulators for various aggregate functions
//! like COUNT, SUM, AVG, MIN, MAX, and COLLECT.

use raisin_models::nodes::properties::PropertyValue;
use std::collections::{HashMap, HashSet};

use super::super::utils::{compare_property_values, compute_property_value_hash, extract_number};
use crate::physical_plan::executor::ExecutionError;

type Result<T> = std::result::Result<T, ExecutionError>;

/// Memory-efficient accumulator for aggregate functions
///
/// Each variant is optimized for its specific aggregate operation, minimizing
/// memory overhead and avoiding unnecessary cloning.
#[derive(Debug, Clone)]
pub(crate) enum Accumulator {
    /// COUNT aggregate - tracks number of values
    Count { count: usize },
    /// SUM aggregate - accumulates numeric sum
    Sum { sum: f64, has_value: bool },
    /// AVG aggregate - tracks sum and count for average calculation
    Avg { sum: f64, count: usize },
    /// MIN aggregate - tracks minimum value
    Min { min: Option<PropertyValue> },
    /// MAX aggregate - tracks maximum value
    Max { max: Option<PropertyValue> },
    /// COLLECT aggregate - collects values into array, optionally with DISTINCT
    Collect {
        values: Vec<PropertyValue>,
        distinct_set: Option<HashSet<u64>>, // Hash-only for DISTINCT
    },
    /// None - placeholder for non-aggregate expressions (zero-cost marker)
    None,
}

impl Accumulator {
    /// Create accumulator with estimated capacity
    ///
    /// # Arguments
    /// * `func_name` - Name of the aggregate function (count, sum, avg, min, max, collect)
    /// * `distinct` - Whether DISTINCT modifier is applied
    /// * `estimated_size` - Estimated number of values for pre-allocation
    pub(crate) fn new(func_name: &str, distinct: bool, estimated_size: usize) -> Self {
        match func_name.to_lowercase().as_str() {
            "collect" => Self::Collect {
                values: Vec::with_capacity(estimated_size),
                distinct_set: if distinct {
                    Some(HashSet::with_capacity(estimated_size))
                } else {
                    None
                },
            },
            "count" => Self::Count { count: 0 },
            "sum" => Self::Sum {
                sum: 0.0,
                has_value: false,
            },
            "avg" => Self::Avg { sum: 0.0, count: 0 },
            "min" => Self::Min { min: None },
            "max" => Self::Max { max: None },
            _ => Self::None,
        }
    }

    /// Update accumulator in-place (no cloning)
    ///
    /// Processes a new value according to the aggregate function's semantics.
    pub(crate) fn update(&mut self, value: PropertyValue) -> Result<()> {
        match self {
            Self::Collect {
                values,
                distinct_set,
            } => {
                if let Some(set) = distinct_set {
                    // DISTINCT: hash-based deduplication
                    let hash = compute_property_value_hash(&value);
                    if set.insert(hash) {
                        values.push(value);
                    }
                } else {
                    // Fast path: no deduplication
                    values.push(value);
                }
            }
            Self::Count { count } => {
                *count += 1;
            }
            Self::Sum { sum, has_value } => {
                if let Some(n) = extract_number(&value) {
                    *sum += n;
                    *has_value = true;
                }
            }
            Self::Avg { sum, count } => {
                if let Some(n) = extract_number(&value) {
                    *sum += n;
                    *count += 1;
                }
            }
            Self::Min { min } => match min {
                None => *min = Some(value),
                Some(current) => {
                    if compare_property_values(&value, current) == std::cmp::Ordering::Less {
                        *min = Some(value);
                    }
                }
            },
            Self::Max { max } => match max {
                None => *max = Some(value),
                Some(current) => {
                    if compare_property_values(&value, current) == std::cmp::Ordering::Greater {
                        *max = Some(value);
                    }
                }
            },
            Self::None => {}
        }
        Ok(())
    }

    /// Finalize accumulator to result value
    ///
    /// Converts the accumulated state into the final result PropertyValue.
    pub(crate) fn finalize(&self) -> Result<PropertyValue> {
        match self {
            Self::Collect { values, .. } => Ok(PropertyValue::Array(values.clone())),
            Self::Count { count } => Ok(PropertyValue::Integer(*count as i64)),
            Self::Sum { sum, has_value } => {
                if *has_value {
                    Ok(PropertyValue::Float(*sum))
                } else {
                    Ok(PropertyValue::Object(HashMap::new())) // NULL equivalent
                }
            }
            Self::Avg { sum, count } => {
                if *count > 0 {
                    Ok(PropertyValue::Float(sum / (*count as f64)))
                } else {
                    Ok(PropertyValue::Object(HashMap::new())) // NULL equivalent
                }
            }
            Self::Min { min } | Self::Max { max: min } => min.clone().ok_or_else(|| {
                ExecutionError::Backend("Aggregate min/max on empty set".to_string())
            }),
            Self::None => Ok(PropertyValue::Object(HashMap::new())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_count_accumulator() {
        let mut acc = Accumulator::new("count", false, 10);
        acc.update(PropertyValue::Integer(1)).unwrap();
        acc.update(PropertyValue::Integer(2)).unwrap();
        acc.update(PropertyValue::Integer(3)).unwrap();

        let result = acc.finalize().unwrap();
        assert_eq!(result, PropertyValue::Integer(3));
    }

    #[test]
    fn test_sum_accumulator() {
        let mut acc = Accumulator::new("sum", false, 10);
        acc.update(PropertyValue::Float(1.0)).unwrap();
        acc.update(PropertyValue::Float(2.0)).unwrap();
        acc.update(PropertyValue::Float(3.0)).unwrap();

        let result = acc.finalize().unwrap();
        assert_eq!(result, PropertyValue::Float(6.0));
    }

    #[test]
    fn test_avg_accumulator() {
        let mut acc = Accumulator::new("avg", false, 10);
        acc.update(PropertyValue::Float(2.0)).unwrap();
        acc.update(PropertyValue::Float(4.0)).unwrap();
        acc.update(PropertyValue::Float(6.0)).unwrap();

        let result = acc.finalize().unwrap();
        assert_eq!(result, PropertyValue::Float(4.0));
    }

    #[test]
    fn test_collect_distinct() {
        let mut acc = Accumulator::new("collect", true, 10);
        acc.update(PropertyValue::String("a".to_string())).unwrap();
        acc.update(PropertyValue::String("b".to_string())).unwrap();
        acc.update(PropertyValue::String("a".to_string())).unwrap(); // duplicate

        let result = acc.finalize().unwrap();
        if let PropertyValue::Array(arr) = result {
            assert_eq!(arr.len(), 2); // Only unique values
        } else {
            panic!("Expected Array");
        }
    }
}
