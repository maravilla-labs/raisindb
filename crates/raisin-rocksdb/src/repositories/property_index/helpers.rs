//! Shared helpers for property index operations

/// Check if a value is a tombstone marker. An EMPTY value is a live entry (the
/// rebuild writes custom properties that way), not a tombstone.
#[inline]
pub(super) fn is_tombstone(value: &[u8]) -> bool {
    crate::keys::is_tombstone_value(value)
}
