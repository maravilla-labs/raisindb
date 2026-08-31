// SPDX-License-Identifier: BSL-1.1

//! Utility functions for the Tantivy indexing engine.

use raisin_error::{Error, Result};
use std::path::Path;

/// Tantivy's writer lock file. Never copied into a snapshot: the lock itself
/// is an advisory file lock, so the copy carries no meaning, and leaving it
/// out keeps a snapshot from looking like it has a writer attached.
const WRITER_LOCK_FILE: &str = ".tantivy-writer.lock";
const META_LOCK_FILE: &str = ".tantivy-meta.lock";

/// How many times to re-take a snapshot that came out unreadable.
///
/// A retry is only needed when a merge landed mid-copy and garbage-collected
/// a segment the snapshot had already recorded in `meta.json`. That can happen
/// twice in a row, but it converges: each merge leaves fewer, larger segments
/// and the source is not accepting new commits while we hold its writer.
const SNAPSHOT_ATTEMPTS: usize = 3;

pub(crate) fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)
        .map_err(|e| Error::storage(format!("Failed to create target directory: {}", e)))?;

    for entry in std::fs::read_dir(src)
        .map_err(|e| Error::storage(format!("Failed to read source directory: {}", e)))?
    {
        let entry =
            entry.map_err(|e| Error::storage(format!("Failed to read directory entry: {}", e)))?;
        let path = entry.path();
        let file_name = entry.file_name();

        if file_name == WRITER_LOCK_FILE || file_name == META_LOCK_FILE {
            continue;
        }

        let target_path = dst.join(&file_name);

        if path.is_dir() {
            copy_dir_recursive(&path, &target_path)?;
        } else {
            match std::fs::copy(&path, &target_path) {
                Ok(_) => {}
                // A merge that completes mid-copy garbage-collects the
                // segment files it superseded, and it writes its output
                // through `.tmp*` scratch files that are renamed away. Both
                // disappear under us. Skipping them is safe because the
                // snapshot is validated afterwards: if what vanished was
                // still referenced by `meta.json`, the open fails and the
                // whole snapshot is retaken.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    tracing::debug!(
                        file = %path.display(),
                        "Source file vanished mid-copy (merge garbage collection); skipping"
                    );
                }
                Err(e) => {
                    return Err(Error::storage(format!("Failed to copy file: {}", e)));
                }
            }
        }
    }

    Ok(())
}

/// Copy a live Tantivy index directory to `dst`, atomically and readably.
///
/// A plain recursive copy of an index that has a writer attached is a race on
/// two counts. The directory is a moving target — a merge landing mid-copy
/// deletes the segments it superseded, so the copy can end up holding a
/// `meta.json` that references files it never got. And the copy is performed
/// in place, so a partially copied (or wholly broken) index is visible at the
/// branch's path the entire time it is being written.
///
/// This closes both: the copy is staged in a sibling temporary directory and
/// validated by actually opening it and reading a searcher, and only a
/// snapshot that survives that becomes visible, via a single `rename`.
pub(crate) fn snapshot_index_dir(src: &Path, dst: &Path) -> Result<()> {
    let parent = dst.parent().ok_or_else(|| {
        Error::storage(format!(
            "Index target has no parent directory: {}",
            dst.display()
        ))
    })?;
    std::fs::create_dir_all(parent)
        .map_err(|e| Error::storage(format!("Failed to create index parent: {}", e)))?;

    let staging = parent.join(format!(".snapshot-{}", uuid::Uuid::new_v4()));

    let mut last_error = None;
    for attempt in 1..=SNAPSHOT_ATTEMPTS {
        let _ = std::fs::remove_dir_all(&staging);
        copy_dir_recursive(src, &staging)?;

        match validate_index_dir(&staging) {
            Ok(()) => {
                last_error = None;
                break;
            }
            Err(e) => {
                tracing::warn!(
                    attempt,
                    source = %src.display(),
                    error = %e,
                    "Index snapshot came out unreadable (a merge probably landed \
                     mid-copy); retaking it"
                );
                last_error = Some(e);
            }
        }
    }

    if let Some(e) = last_error {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(Error::storage(format!(
            "Could not take a readable snapshot of {} after {} attempts: {}",
            src.display(),
            SNAPSHOT_ATTEMPTS,
            e
        )));
    }

    // Replace the target only now that a good snapshot exists — never delete
    // first and copy second.
    if dst.exists() {
        tracing::warn!(
            target = %dst.display(),
            "Branch index target already existed; replacing it with the snapshot"
        );
        std::fs::remove_dir_all(dst).map_err(|e| {
            let _ = std::fs::remove_dir_all(&staging);
            Error::storage(format!("Failed to clear existing index target: {}", e))
        })?;
    }

    std::fs::rename(&staging, dst).map_err(|e| {
        let _ = std::fs::remove_dir_all(&staging);
        Error::storage(format!("Failed to publish index snapshot: {}", e))
    })?;

    Ok(())
}

/// Prove a copied index is usable before anything is allowed to see it.
///
/// Opening resolves `meta.json` against the files actually present, and taking
/// a searcher forces the segment readers open, so a segment that was collected
/// out from under the copy surfaces here rather than on someone's first query.
fn validate_index_dir(path: &Path) -> Result<()> {
    let index =
        tantivy::Index::open_in_dir(path).map_err(|e| Error::storage(format!("open: {}", e)))?;
    let reader = index
        .reader()
        .map_err(|e| Error::storage(format!("reader: {}", e)))?;
    let _ = reader.searcher().num_docs();
    Ok(())
}
