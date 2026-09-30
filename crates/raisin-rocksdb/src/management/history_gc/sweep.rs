//! The retroactive blob sweep: every blob the binary store lists that nothing
//! in the database mentions.
//!
//! Pruning only reports the blobs the versions IT removes named. Blobs whose
//! last reference went some other way — history pruned by an earlier run or
//! before blob tracking existed, a package `.rap` replaced before replacement
//! deleted the old one, an upload whose node was never written — stay on disk
//! forever unless something walks the store itself.
//!
//! A listed blob is swept only when all of these hold:
//!
//! * its key has the blob shape (`[{prefix}/…]{yyyy}/{mm}/{dd}/{nanoid}[.ext]`)
//!   — anything else in the directory is not ours to judge;
//! * on a tenant-scoped run, its key sits under `{tenant}/`;
//! * it is older than [`GcOptions::blob_min_age`] — an upload is stored before
//!   the node that names it is written;
//! * no key or value in any column family but the derived indexes mentions
//!   its id, in any form (see [`super::blobs`] and `DERIVED_INDEXES`).

use super::{blobs, blobs_still_referenced, GcOptions};
use crate::RocksDBStorage;
use raisin_binary::ListedBlob;
use raisin_error::{Error, Result};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// The listed blobs eligible for the sweep, by id.
pub(super) fn candidates(
    listed: Vec<ListedBlob>,
    opts: &GcOptions,
    now: chrono::DateTime<chrono::Utc>,
) -> HashMap<String, ListedBlob> {
    let min_age =
        chrono::Duration::from_std(opts.blob_min_age).unwrap_or(chrono::Duration::days(36_500));
    let tenant_prefix = opts.tenant.as_ref().map(|t| format!("{t}/"));
    let mut out = HashMap::new();
    for blob in listed {
        if let Some(p) = &tenant_prefix {
            if !blob.key.starts_with(p.as_str()) {
                continue;
            }
        }
        // No modification time means no proof of age: keep it.
        let old_enough = blob
            .modified
            .is_some_and(|m| now.signed_duration_since(m) >= min_age);
        if !old_enough {
            continue;
        }
        if let Some(id) = blobs::identity_of_key(&blob.key) {
            out.insert(id.to_string(), blob);
        }
    }
    out
}

/// The listed blobs nothing references (the database scan runs off the async
/// runtime).
pub(super) async fn unreferenced(
    storage: Arc<RocksDBStorage>,
    listed: Vec<ListedBlob>,
    opts: &GcOptions,
) -> Result<Vec<ListedBlob>> {
    let candidates = candidates(listed, opts, chrono::Utc::now());
    if candidates.is_empty() {
        return Ok(Vec::new());
    }
    tokio::task::spawn_blocking(move || {
        let referenced = blobs_still_referenced(storage.db(), &candidates, &HashSet::new())?;
        let mut out: Vec<ListedBlob> = candidates
            .into_iter()
            .filter(|(id, _)| !referenced.contains(id))
            .map(|(_, blob)| blob)
            .collect();
        out.sort_by(|a, b| a.key.cmp(&b.key));
        Ok(out)
    })
    .await
    .map_err(|e| Error::storage(format!("blob sweep task failed: {e}")))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn blob(key: &str, age_hours: i64, now: chrono::DateTime<chrono::Utc>) -> ListedBlob {
        ListedBlob {
            key: key.to_string(),
            size: 1,
            modified: Some(now - chrono::Duration::hours(age_hours)),
        }
    }

    #[test]
    fn only_old_blob_shaped_keys_in_scope_are_candidates() {
        let now = chrono::Utc::now();
        let listed = vec![
            blob("t1/2026/09/29/aaaaaaaaaaaaaaaaaaaaa.rap", 48, now),
            blob("2026/09/29/bbbbbbbbbbbbbbbbbbbbb.jpg", 48, now),
            blob("t1/2026/09/29/ccccccccccccccccccccc.rap", 1, now),
            blob("t1/notes.txt", 48, now),
            ListedBlob {
                key: "t1/2026/09/29/ddddddddddddddddddddd.rap".into(),
                size: 1,
                modified: None,
            },
        ];
        let opts = GcOptions {
            blob_min_age: Duration::from_secs(24 * 3600),
            ..GcOptions::default()
        };
        let mut all: Vec<String> = candidates(listed.clone(), &opts, now)
            .into_values()
            .map(|b| b.key)
            .collect();
        all.sort();
        assert_eq!(
            all,
            vec![
                "2026/09/29/bbbbbbbbbbbbbbbbbbbbb.jpg".to_string(),
                "t1/2026/09/29/aaaaaaaaaaaaaaaaaaaaa.rap".to_string(),
            ]
        );

        // A tenant-scoped run only judges that tenant's namespace.
        let scoped = GcOptions {
            tenant: Some("t1".into()),
            ..opts
        };
        let keys: Vec<String> = candidates(listed, &scoped, now)
            .into_values()
            .map(|b| b.key)
            .collect();
        assert_eq!(
            keys,
            vec!["t1/2026/09/29/aaaaaaaaaaaaaaaaaaaaa.rap".to_string()]
        );
    }
}
