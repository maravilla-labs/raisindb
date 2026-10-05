//! The disk precheck every repair runs before it writes.

use crate::cf_handle;
use raisin_error::Result;
use rocksdb::DB;

/// Refuse to start without free space of at least twice `cf_name`'s on-disk
/// size on the data volume: the repair's writes are small, but the compaction
/// that reclaims the superseded entries afterwards rewrites the CF.
pub fn check_headroom(db: &DB, cf_name: &str) -> Result<()> {
    check_headroom_assuming(db, cf_name, None)
}

/// [`check_headroom`], with the free space taken from `free_bytes` when given
/// (a TEST hook: a test cannot shrink the volume it runs on).
pub fn check_headroom_assuming(db: &DB, cf_name: &str, free_bytes: Option<u64>) -> Result<()> {
    let cf = cf_handle(db, cf_name)?;
    let cf_bytes = db
        .property_int_value_cf(cf, "rocksdb.total-sst-files-size")
        .map_err(|e| raisin_error::Error::storage(e.to_string()))?
        .unwrap_or(0);
    let available = match free_bytes {
        Some(bytes) => bytes,
        None => available_bytes(db.path())?,
    };
    if available < cf_bytes.saturating_mul(2) {
        return Err(raisin_error::Error::Validation(format!(
            "repair refused: {available} bytes free on the data volume, need at least \
             {} (2x the {cf_name} column family)",
            cf_bytes.saturating_mul(2)
        )));
    }
    Ok(())
}

/// Refuse a write of about `estimated_bytes` NEW bytes unless the data volume
/// has twice that free and keeps `floor` free after it — the rule for a build
/// whose output the column family's current size says nothing about (an
/// index never built before). `free_bytes` as in [`check_headroom_assuming`].
pub fn check_output_headroom(
    db: &DB,
    estimated_bytes: u64,
    floor: u64,
    free_bytes: Option<u64>,
) -> Result<()> {
    let available = match free_bytes {
        Some(bytes) => bytes,
        None => available_bytes(db.path())?,
    };
    let needed = estimated_bytes
        .saturating_mul(2)
        .max(estimated_bytes.saturating_add(floor));
    if available < needed {
        return Err(raisin_error::Error::Validation(format!(
            "{available} bytes free on the data volume; this build writes about \
             {estimated_bytes} bytes and needs at least {needed} free (twice its output, \
             and {floor} left after it)"
        )));
    }
    Ok(())
}

/// Free bytes on the volume holding `path`, from `df -Pk` (POSIX output).
fn available_bytes(path: &std::path::Path) -> Result<u64> {
    let output = std::process::Command::new("df")
        .arg("-Pk")
        .arg(path)
        .output()
        .map_err(|e| raisin_error::Error::storage(format!("cannot run df: {e}")))?;
    let text = String::from_utf8_lossy(&output.stdout);
    // Header, then: filesystem, 1024-blocks, used, available, capacity, mount.
    text.lines()
        .nth(1)
        .and_then(|line| line.split_whitespace().nth(3))
        .and_then(|kb| kb.parse::<u64>().ok())
        .map(|kb| kb * 1024)
        .ok_or_else(|| {
            raisin_error::Error::storage(format!("cannot read free space from df: {text}"))
        })
}
