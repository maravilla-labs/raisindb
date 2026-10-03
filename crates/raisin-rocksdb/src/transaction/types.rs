//! Common types, constants, and pure helper functions for transaction module

/// Tombstone marker for deleted entries
pub(crate) const TOMBSTONE: &[u8] = b"T";

/// Check if a value is a tombstone marker
#[inline]
pub(crate) fn is_tombstone(value: &[u8]) -> bool {
    crate::keys::is_tombstone_value(value)
}
