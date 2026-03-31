//! Grouping logic for aggregate queries
//!
//! This module provides the GroupKey type and related utilities for efficient
//! grouping in Cypher aggregation queries.

use raisin_models::nodes::properties::PropertyValue;

use super::super::utils::compute_property_value_hash;

/// Efficient group key with fast hashing
///
/// Used to group bindings in aggregate queries. The Empty variant represents
/// a single group (no GROUP BY), while Hashed contains hash values of grouping keys.
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub(crate) enum GroupKey {
    /// No grouping (single group)
    Empty,
    /// Hash values of grouping keys
    Hashed(Vec<u64>),
}

impl GroupKey {
    /// Compute a group key from a list of values
    ///
    /// Returns Empty if no values are provided, otherwise returns Hashed
    /// containing the hash of each value.
    pub(crate) fn compute(values: &[PropertyValue]) -> Self {
        if values.is_empty() {
            Self::Empty
        } else {
            let hashes: Vec<u64> = values.iter().map(compute_property_value_hash).collect();
            Self::Hashed(hashes)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_group_key() {
        let key = GroupKey::compute(&[]);
        assert_eq!(key, GroupKey::Empty);
    }

    #[test]
    fn test_hashed_group_key() {
        let values = vec![
            PropertyValue::String("test".to_string()),
            PropertyValue::Integer(42),
        ];
        let key = GroupKey::compute(&values);

        match key {
            GroupKey::Hashed(hashes) => {
                assert_eq!(hashes.len(), 2);
            }
            GroupKey::Empty => panic!("Expected Hashed, got Empty"),
        }
    }

    #[test]
    fn test_group_key_equality() {
        let values1 = vec![PropertyValue::String("test".to_string())];
        let values2 = vec![PropertyValue::String("test".to_string())];
        let values3 = vec![PropertyValue::String("other".to_string())];

        let key1 = GroupKey::compute(&values1);
        let key2 = GroupKey::compute(&values2);
        let key3 = GroupKey::compute(&values3);

        assert_eq!(key1, key2);
        assert_ne!(key1, key3);
    }
}
