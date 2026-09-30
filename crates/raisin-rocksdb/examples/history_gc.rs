//! Run history GC against the data directory of a STOPPED server.
//!
//! ```text
//! cargo run -p raisin-rocksdb --release --example history_gc -- <data_dir> \
//!     [--dry-run] [--keep-days N] [--keep-revisions N] [--keep-oplog] [--job-retention-hours N]
//!     [--blob-min-age-hours N] [--no-blob-sweep]
//! ```
//!
//! Opens `<data_dir>` (the directory holding the RocksDB files and `uploads/`),
//! prunes revision history under the given retention (default: 7 days / 100
//! revisions, overriding stored policies only when a flag is given), purges the
//! operation log unless `--keep-oplog`, deletes finished jobs older than the
//! job retention, bounds oversized stored job results, deletes the upload blobs
//! nothing in the database mentions (older than `--blob-min-age-hours`,
//! default 24), compacts, and prints the report as JSON.
//!
//! Never point this at a directory a running server has open: RocksDB's LOCK
//! file refuses a second writer, and that is the only thing standing between
//! this tool and a corrupted database.

use raisin_binary::FilesystemBinaryStorage;
use raisin_rocksdb::management::history_gc::{self, GcOptions, HistoryRetention};
use raisin_rocksdb::{RocksDBConfig, RocksDBStorage};
use std::sync::Arc;
use std::time::Duration;

fn arg_value<T: std::str::FromStr>(args: &[String], flag: &str) -> Option<T> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(data_dir) = args.first().filter(|a| !a.starts_with("--")).cloned() else {
        eprintln!("usage: history_gc <data_dir> [--dry-run] [--keep-days N] [--keep-revisions N] [--keep-oplog] [--job-retention-hours N] [--blob-min-age-hours N] [--no-blob-sweep]");
        std::process::exit(2);
    };
    let dry_run = args.iter().any(|a| a == "--dry-run");
    let keep_days: Option<u32> = arg_value(&args, "--keep-days");
    let keep_revisions: Option<u64> = arg_value(&args, "--keep-revisions");
    let job_hours: u64 = arg_value(&args, "--job-retention-hours").unwrap_or(24);
    let blob_hours: u64 = arg_value(&args, "--blob-min-age-hours").unwrap_or(24);

    let config = RocksDBConfig::production().with_path(&data_dir);
    let storage = Arc::new(RocksDBStorage::with_config(config)?);
    let bin = FilesystemBinaryStorage::new(std::path::Path::new(&data_dir).join("uploads"), None);

    let opts = GcOptions {
        dry_run,
        default_retention: HistoryRetention {
            keep_days: Some(7),
            keep_revisions: Some(100),
        },
        retention_override: (keep_days.is_some() || keep_revisions.is_some()).then_some(
            HistoryRetention {
                keep_days,
                keep_revisions,
            },
        ),
        purge_oplog: !args.iter().any(|a| a == "--keep-oplog"),
        job_retention: Some(Duration::from_secs(job_hours * 3600)),
        sweep_unreferenced_blobs: !args.iter().any(|a| a == "--no-blob-sweep"),
        blob_min_age: Duration::from_secs(blob_hours * 3600),
        ..GcOptions::default()
    };

    let outcome = history_gc::run_gc_and_sweep_blobs(storage, &bin, opts).await?;
    let mut json = serde_json::to_value(&outcome)?;
    // The full blob list can be long; the count is what matters here.
    if let Some(list) = json.get_mut("orphaned_blobs") {
        let n = list.as_array().map(|a| a.len()).unwrap_or(0);
        *list = serde_json::json!(n);
    }
    println!("{}", serde_json::to_string_pretty(&json)?);
    Ok(())
}
