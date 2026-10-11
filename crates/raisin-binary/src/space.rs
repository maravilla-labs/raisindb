//! Free-space policy for large writes.
//!
//! When the blob store and the database share a volume, filling it does more
//! than fail one write: RocksDB hits ENOSPC and the whole server goes
//! read-only. So a large writer must leave a **floor** free — the larger of
//! 5 GiB and 5 % of the volume — and must check it both before it opens a
//! writer and again while it is writing.

use crate::writer::StorageSpace;

/// Absolute part of the floor.
pub const FLOOR_MIN_BYTES: u64 = 5 * 1024 * 1024 * 1024;
/// Relative part of the floor, in percent of the volume.
pub const FLOOR_PERCENT: u64 = 5;

/// Bytes that must remain free on a volume of `total` bytes.
pub fn floor_bytes(total: u64) -> u64 {
    FLOOR_MIN_BYTES.max((total as u128 * FLOOR_PERCENT as u128 / 100) as u64)
}

/// Free and total space of the volume holding `path` (or its nearest
/// existing ancestor), `None` if it cannot be read.
pub fn volume_space(path: &std::path::Path) -> Option<StorageSpace> {
    let mut probe = path;
    while !probe.exists() {
        probe = probe.parent()?;
    }
    fs4::statvfs(probe).ok().map(|s| StorageSpace {
        available: s.available_space(),
        total: s.total_space(),
    })
}

/// Why a write may not go ahead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InsufficientStorage {
    /// Bytes the write needs, floor included.
    pub need: u64,
    /// Bytes available on the volume.
    pub have: u64,
}

impl std::fmt::Display for InsufficientStorage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "insufficient storage: need {} bytes free (including the safety floor), have {}",
            self.need, self.have
        )
    }
}

impl std::error::Error for InsufficientStorage {}

/// May `bytes` more be written to a volume in state `space`?
///
/// `None` space means the backend cannot say (object storage); that is the
/// caller's quota to enforce, and this check passes.
pub fn check_room(space: Option<StorageSpace>, bytes: u64) -> Result<(), InsufficientStorage> {
    let Some(space) = space else {
        return Ok(());
    };
    let need = bytes.saturating_add(floor_bytes(space.total));
    if space.available >= need {
        Ok(())
    } else {
        Err(InsufficientStorage {
            need,
            have: space.available,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1024 * 1024 * 1024;

    #[test]
    fn the_floor_is_the_larger_of_5_gib_and_5_percent() {
        assert_eq!(floor_bytes(10 * GIB), 5 * GIB);
        assert_eq!(floor_bytes(100 * GIB), 5 * GIB);
        assert_eq!(floor_bytes(1000 * GIB), 50 * GIB);
        assert_eq!(floor_bytes(0), 5 * GIB);
    }

    #[test]
    fn room_counts_the_floor() {
        let space = StorageSpace {
            available: 11 * GIB,
            total: 460 * GIB,
        };
        // 460 GiB volume → floor 23 GiB; 11 GiB free is already below it.
        let err = check_room(Some(space), 1).unwrap_err();
        assert_eq!(err.have, 11 * GIB);
        assert_eq!(err.need, 23 * GIB + 1);

        let roomy = StorageSpace {
            available: 40 * GIB,
            total: 100 * GIB,
        };
        assert!(check_room(Some(roomy), 35 * GIB).is_ok());
        assert!(check_room(Some(roomy), 35 * GIB + 1).is_err());
    }

    #[test]
    fn unknown_space_passes() {
        assert!(check_room(None, u64::MAX).is_ok());
    }
}
