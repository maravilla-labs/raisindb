//! Property index scan operations
//!
//! NOTE: File slightly exceeds 300 lines (~365) because each scan function
//! contains both ascending and descending iteration paths with shared MVCC logic.
//! Splitting further would fragment the closely-coupled scan implementations.
//!
//! Provides ordered scans and bounded range scans over the property index.

use super::helpers::{is_tombstone, parse_entry_components};
use crate::repositories::nodes::hash_property_value;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::PropertyScanEntry;
use rocksdb::{Direction, IteratorMode, DB};
use std::sync::Arc;

pub(super) async fn scan_property(
    db: &Arc<DB>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    property_name: &str,
    published_only: bool,
    ascending: bool,
    limit: Option<usize>,
) -> Result<Vec<PropertyScanEntry>> {
    let tag = if published_only { "prop_pub" } else { "prop" };

    let prefix = keys::KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push(workspace)
        .push(tag)
        .push(property_name)
        .build_prefix();

    tracing::debug!(
        "🔍 scan_property: tenant={}, repo={}, branch={}, workspace={}, property={}, tag={}, ascending={}, limit={:?}",
        tenant_id, repo_id, branch, workspace, property_name, tag, ascending, limit
    );
    tracing::debug!(
        "🔍 scan_property: prefix={:?} (len={})",
        String::from_utf8_lossy(&prefix),
        prefix.len()
    );

    let cf = cf_handle(db, cf::PROPERTY_INDEX)?;
    let prefix_clone = prefix.clone();
    let mut seen_nodes = std::collections::HashSet::new();
    // Track tombstoned node_ids for MVCC
    let mut tombstoned_node_ids = std::collections::HashSet::new();
    let mut results = Vec::new();
    let mut keys_iterated = 0usize;
    let mut keys_parsed = 0usize;
    let mut keys_skipped_tombstone = 0usize;
    let mut keys_skipped_prefix_mismatch = 0usize;

    if ascending {
        let iter = db.prefix_iterator_cf(cf, prefix);

        for item in iter {
            keys_iterated += 1;

            // Early termination if limit reached
            if let Some(lim) = limit {
                if results.len() >= lim {
                    tracing::debug!("🔍 scan_property: limit {} reached, stopping", lim);
                    break;
                }
            }

            let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            if !key.starts_with(&prefix_clone) {
                keys_skipped_prefix_mismatch += 1;
                tracing::trace!(
                    "🔍 scan_property: key doesn't match prefix, stopping. key={:?}",
                    String::from_utf8_lossy(&key)
                );
                break;
            }

            if let Some((node_id, property_value)) = parse_entry_components(&key) {
                keys_parsed += 1;

                // Skip tombstones and track them for MVCC
                if value.is_empty() || is_tombstone(&value) {
                    keys_skipped_tombstone += 1;
                    tombstoned_node_ids.insert(node_id);
                    continue;
                }

                // Skip entries for node_ids that have been tombstoned at a newer revision
                if tombstoned_node_ids.contains(&node_id) {
                    continue;
                }

                if seen_nodes.insert(node_id.clone()) {
                    tracing::trace!(
                        "🔍 scan_property: found entry node_id={}, value={}",
                        node_id,
                        property_value
                    );
                    results.push(PropertyScanEntry {
                        node_id,
                        property_value,
                    });
                }
            }
        }
    } else {
        let mut upper_bound = prefix.clone();
        upper_bound.push(0xFF);
        let iter = db.iterator_cf(cf, IteratorMode::From(&upper_bound, Direction::Reverse));

        for item in iter {
            keys_iterated += 1;

            // Early termination if limit reached
            if let Some(lim) = limit {
                if results.len() >= lim {
                    tracing::debug!("🔍 scan_property: limit {} reached, stopping", lim);
                    break;
                }
            }

            let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            if !key.starts_with(&prefix_clone) {
                keys_skipped_prefix_mismatch += 1;
                if key.as_ref() < prefix_clone.as_slice() {
                    tracing::trace!(
                        "🔍 scan_property: key < prefix, stopping reverse scan. key={:?}",
                        String::from_utf8_lossy(&key)
                    );
                    break;
                } else {
                    continue;
                }
            }

            if let Some((node_id, property_value)) = parse_entry_components(&key) {
                keys_parsed += 1;

                // Skip tombstones and track them for MVCC
                if value.is_empty() || is_tombstone(&value) {
                    keys_skipped_tombstone += 1;
                    tombstoned_node_ids.insert(node_id);
                    continue;
                }

                // Skip entries for node_ids that have been tombstoned at a newer revision
                if tombstoned_node_ids.contains(&node_id) {
                    continue;
                }

                if seen_nodes.insert(node_id.clone()) {
                    tracing::trace!(
                        "🔍 scan_property: found entry node_id={}, value={}",
                        node_id,
                        property_value
                    );
                    results.push(PropertyScanEntry {
                        node_id,
                        property_value,
                    });
                }
            }
        }
    }

    tracing::debug!(
        "🔍 scan_property: COMPLETE - {} results, iterated={}, parsed={}, tombstones={}, prefix_mismatch={}",
        results.len(),
        keys_iterated,
        keys_parsed,
        keys_skipped_tombstone,
        keys_skipped_prefix_mismatch
    );

    Ok(results)
}

pub(super) async fn scan_property_range(
    db: &Arc<DB>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    property_name: &str,
    lower_bound: Option<(&PropertyValue, bool)>,
    upper_bound: Option<(&PropertyValue, bool)>,
    published_only: bool,
    ascending: bool,
    limit: Option<usize>,
) -> Result<Vec<PropertyScanEntry>> {
    let tag = if published_only { "prop_pub" } else { "prop" };

    // Build the base prefix (without value)
    let base_prefix = keys::KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push(workspace)
        .push(tag)
        .push(property_name)
        .build_prefix();

    let cf = cf_handle(db, cf::PROPERTY_INDEX)?;
    let mut seen_nodes = std::collections::HashSet::new();
    let mut results = Vec::new();

    // Convert bounds to their string representations for comparison
    let lower_str = lower_bound.map(|(v, incl)| (hash_property_value(v), incl));
    let upper_str = upper_bound.map(|(v, incl)| (hash_property_value(v), incl));

    if ascending {
        // For ascending scan, we seek to the lower bound (if any) or start of prefix
        let start_key = if let Some((ref lower_val, _)) = lower_str {
            let mut key = base_prefix.clone();
            key.extend_from_slice(lower_val.as_bytes());
            key
        } else {
            base_prefix.clone()
        };

        let iter = db.iterator_cf(cf, IteratorMode::From(&start_key, Direction::Forward));

        for item in iter {
            if let Some(lim) = limit {
                if results.len() >= lim {
                    break;
                }
            }

            let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            // Check if we're still within our property's prefix
            if !key.starts_with(&base_prefix) {
                break;
            }

            // Skip tombstones (deleted entries marked with "T")
            // Note: Property index entries can have:
            //   - Empty value (user properties via PropertyIndexRepository)
            //   - Node ID bytes (system properties via add_system_property_indexes)
            //   - "T" (tombstone for deleted entries)
            if value.as_ref() == b"T" {
                continue;
            }

            if let Some((node_id, property_value)) = parse_entry_components(&key) {
                // Check lower bound
                if let Some((ref lower_val, inclusive)) = lower_str {
                    if inclusive {
                        if property_value < *lower_val {
                            continue;
                        }
                    } else if property_value <= *lower_val {
                        continue;
                    }
                }

                // Check upper bound
                if let Some((ref upper_val, inclusive)) = upper_str {
                    if inclusive {
                        if property_value > *upper_val {
                            break; // Past upper bound, stop iteration
                        }
                    } else if property_value >= *upper_val {
                        break;
                    }
                }

                if seen_nodes.insert(node_id.clone()) {
                    results.push(PropertyScanEntry {
                        node_id,
                        property_value,
                    });
                }
            }
        }
    } else {
        // For descending scan, we seek to the upper bound (if any) or end of prefix
        let start_key = if let Some((ref upper_val, _)) = upper_str {
            let mut key = base_prefix.clone();
            key.extend_from_slice(upper_val.as_bytes());
            // Add 0xFF to ensure we start past the upper bound value
            key.push(0xFF);
            key
        } else {
            let mut key = base_prefix.clone();
            key.push(0xFF);
            key
        };

        let iter = db.iterator_cf(cf, IteratorMode::From(&start_key, Direction::Reverse));

        for item in iter {
            if let Some(lim) = limit {
                if results.len() >= lim {
                    break;
                }
            }

            let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            // Check if we're still within our property's prefix
            if !key.starts_with(&base_prefix) {
                if key.as_ref() < base_prefix.as_slice() {
                    break;
                } else {
                    continue;
                }
            }

            // Skip tombstones (deleted entries marked with "T")
            // Note: Property index entries can have:
            //   - Empty value (user properties via PropertyIndexRepository)
            //   - Node ID bytes (system properties via add_system_property_indexes)
            //   - "T" (tombstone for deleted entries)
            if value.as_ref() == b"T" {
                continue;
            }

            if let Some((node_id, property_value)) = parse_entry_components(&key) {
                // Check upper bound
                if let Some((ref upper_val, inclusive)) = upper_str {
                    if inclusive {
                        if property_value > *upper_val {
                            continue;
                        }
                    } else if property_value >= *upper_val {
                        continue;
                    }
                }

                // Check lower bound
                if let Some((ref lower_val, inclusive)) = lower_str {
                    if inclusive {
                        if property_value < *lower_val {
                            break; // Past lower bound, stop iteration
                        }
                    } else if property_value <= *lower_val {
                        break;
                    }
                }

                if seen_nodes.insert(node_id.clone()) {
                    results.push(PropertyScanEntry {
                        node_id,
                        property_value,
                    });
                }
            }
        }
    }

    Ok(results)
}
