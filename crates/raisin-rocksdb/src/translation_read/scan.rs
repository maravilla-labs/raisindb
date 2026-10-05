//! The grouped newest-at-or-before scan behind every translation listing.

use raisin_error::Result;
use raisin_hlc::HLC;
use rocksdb::DB;
use std::ops::ControlFlow;

/// Scan `prefix` and call `visit` once per group — the first `depth`
/// `\0`-terminated text segments after the prefix — with the group's newest
/// version at or before `max_revision` (its revision and value). A group's
/// versions are contiguous and newest first, so the first one within the
/// bound decides it. A key whose revision does not parse is skipped.
pub(crate) fn for_each_newest(
    db: &DB,
    cf: &'static str,
    prefix: &[u8],
    depth: usize,
    max_revision: Option<&HLC>,
    visit: impl FnMut(&[&str], HLC, &[u8]),
) -> Result<()> {
    for_each_newest_in(
        &mut crate::mvcc_read::DbRead(db),
        cf,
        prefix,
        depth,
        max_revision,
        visit,
    )
}

/// [`for_each_newest`] through a read source (a lookup's pinned iterators,
/// plan Phase 13d).
pub(crate) fn for_each_newest_in(
    src: &mut impl crate::mvcc_read::VersionedRead,
    cf: &'static str,
    prefix: &[u8],
    depth: usize,
    max_revision: Option<&HLC>,
    mut visit: impl FnMut(&[&str], HLC, &[u8]),
) -> Result<()> {
    let mut decided: Option<Vec<u8>> = None;
    src.scan(cf, prefix, prefix, &mut |key, value| {
        let Some(suffix) = key.strip_prefix(prefix) else {
            return ControlFlow::Break(());
        };
        // The group is the first `depth` segments, terminator included.
        let mut end = 0;
        let mut found = 0;
        while found < depth {
            match suffix[end..].iter().position(|b| *b == 0) {
                Some(i) => {
                    end += i + 1;
                    found += 1;
                }
                None => break,
            }
        }
        if found < depth {
            return ControlFlow::Continue(());
        }
        let group = &suffix[..end];
        if decided.as_deref() == Some(group) {
            return ControlFlow::Continue(());
        }
        let revision = match crate::keys::extract_revision_from_key(key) {
            Ok(revision) if max_revision.is_none_or(|max| &revision <= max) => revision,
            // Newer than the bound, or unreadable: not this version.
            _ => return ControlFlow::Continue(()),
        };
        decided = Some(group.to_vec());
        let segments: std::result::Result<Vec<&str>, _> = group[..end - 1]
            .split(|b| *b == 0)
            .map(std::str::from_utf8)
            .collect();
        match segments {
            Ok(segments) => visit(&segments, revision, value),
            Err(_) => tracing::warn!("Skipping a translation key with a non-UTF-8 segment"),
        }
        ControlFlow::Continue(())
    })
}
