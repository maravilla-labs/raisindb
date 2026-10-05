//! Which keys hold revision history, and where their revision sits.
//!
//! Every MVCC column family stores one key per `(entity, revision)`, with the
//! revision as a 16-byte descending HLC. A read at revision `R` takes the
//! newest version `<= R` of the entity, and a version whose value is the
//! tombstone marker `T` means "deleted as of this revision".
//!
//! GC therefore needs, for each key: the *group* it belongs to (the key with
//! the revision cut out) and the revision itself. This table is an
//! ALLOW-LIST: a key whose column family, tag or shape is not recognised here
//! is never touched. That matters because several of these column families
//! also hold unversioned rows (metadata, legacy shapes) whose last 16 bytes are
//! not a revision — reading those as "older versions" would delete live data.
//!
//! The descending HLC may itself contain `0x00` bytes, so nothing here splits
//! a key on nulls past the fixed, null-free leading segments; the revision is
//! located from the END of the key, exactly as the branch copier does
//! (`repositories::branches::copy::locate_revision`).

use crate::cf;

/// Where the revision sits in a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Locator {
    /// `…\0{~rev}` — the key ends with the revision.
    Tail,
    /// `…\0{~rev}\0{seg}…` — the revision is followed by `n` null-free
    /// segments (node ids, workspace names).
    BeforeTrailing(usize),
}

/// One tag (the literal segment naming the key shape) and its layout.
#[derive(Debug, Clone, Copy)]
pub(super) struct TagLayout {
    /// Segment index of the tag (0-based, over the null-free leading segments).
    pub tag_index: usize,
    /// The literal tag, or `None` to accept any value at that position.
    pub tag: Option<&'static str>,
    pub locator: Locator,
}

/// How GC treats one column family.
#[derive(Debug, Clone, Copy)]
pub(super) struct GcTarget {
    pub cf: &'static str,
    /// Whether keys are scoped `{tenant}\0{repo}\0{branch}\0…` (true) or
    /// repository-wide `{tenant}\0{repo}\0…` (false).
    pub branch_scoped: bool,
    pub layouts: &'static [TagLayout],
    /// Whether an entity whose only remaining version is a tombstone may be
    /// removed entirely.
    ///
    /// Only safe where the group IS the unit a reader resolves (a node id, a
    /// path). Index families can have a reader that merges several groups —
    /// e.g. a child whose order label changed lives in two `ordered` groups —
    /// and there a tombstone in one group may be what hides an older live row
    /// in another. Keeping the tombstone costs one tiny key; dropping it there
    /// could resurrect a deleted row.
    pub drop_orphan_tombstones: bool,
}

const fn t(tag_index: usize, tag: &'static str, locator: Locator) -> TagLayout {
    TagLayout {
        tag_index,
        tag: Some(tag),
        locator,
    }
}

use Locator::{BeforeTrailing, Tail};

/// The column families GC prunes. Anything not listed keeps all its history:
/// schema families (tiny, and versioned through their VALUE), secrets
/// (rotation must stay readable), the audit log (history is its purpose).
pub(super) const GC_TARGETS: &[GcTarget] = &[
    // {t}\0{r}\0{b}\0{ws}\0nodes\0{node_id}\0{~rev}  and  …\0nodes\0{id}\0adj\0{~rev}
    GcTarget {
        cf: cf::NODES,
        branch_scoped: true,
        layouts: &[t(4, "nodes", Tail)],
        drop_orphan_tombstones: true,
    },
    // …\0path\0{path}\0{~rev}
    GcTarget {
        cf: cf::PATH_INDEX,
        branch_scoped: true,
        layouts: &[t(4, "path", Tail)],
        drop_orphan_tombstones: true,
    },
    // …\0node_path\0{node_id}\0{~rev}
    GcTarget {
        cf: cf::NODE_PATH,
        branch_scoped: true,
        layouts: &[t(4, "node_path", Tail)],
        drop_orphan_tombstones: true,
    },
    // …\0prop{_pub}\0{name}\0{value}\0{~rev}\0{node_id}
    GcTarget {
        cf: cf::PROPERTY_INDEX,
        branch_scoped: true,
        layouts: &[
            t(4, "prop", BeforeTrailing(1)),
            t(4, "prop_pub", BeforeTrailing(1)),
        ],
        drop_orphan_tombstones: false,
    },
    // fwd: …\0ref{_pub}\0{node_id}\0{prop_path}\0{~rev}
    // rev: …\0ref_rev{_pub}\0{ws}\0{path}\0{src_id}\0{prop_path}\0{~rev}
    GcTarget {
        cf: cf::REFERENCE_INDEX,
        branch_scoped: true,
        layouts: &[
            t(4, "ref", Tail),
            t(4, "ref_pub", Tail),
            t(4, "ref_rev", Tail),
            t(4, "ref_rev_pub", Tail),
        ],
        drop_orphan_tombstones: false,
    },
    // …\0rel{_rev}\0{id}\0{type}\0{~rev}\0{id}
    // {t}\0{r}\0{b}\0rel_global\0{type}\0{~rev}\0{ws}\0{id}\0{ws}\0{id}
    GcTarget {
        cf: cf::RELATION_INDEX,
        branch_scoped: true,
        layouts: &[
            t(4, "rel", BeforeTrailing(1)),
            t(4, "rel_rev", BeforeTrailing(1)),
            t(3, "rel_global", BeforeTrailing(4)),
        ],
        drop_orphan_tombstones: false,
    },
    // …\0ordered\0{parent_id}\0{order_label}\0{~rev}\0{child_id}
    GcTarget {
        cf: cf::ORDERED_CHILDREN,
        branch_scoped: true,
        layouts: &[t(4, "ordered", BeforeTrailing(1))],
        drop_orphan_tombstones: false,
    },
    // …\0translations|trans_meta\0{node_id}\0{locale}\0{~rev}
    GcTarget {
        cf: cf::TRANSLATION_DATA,
        branch_scoped: true,
        layouts: &[t(4, "translations", Tail), t(4, "trans_meta", Tail)],
        drop_orphan_tombstones: false,
    },
    // …\0block_trans\0{node_id}\0{block_uuid}\0{locale}\0{~rev}
    GcTarget {
        cf: cf::BLOCK_TRANSLATIONS,
        branch_scoped: true,
        layouts: &[t(4, "block_trans", Tail)],
        drop_orphan_tombstones: false,
    },
    // …\0geo\0{property}\0{geohash}\0{~rev}\0{node_id}
    GcTarget {
        cf: cf::SPATIAL_INDEX,
        branch_scoped: true,
        layouts: &[t(4, "geo", BeforeTrailing(1))],
        drop_orphan_tombstones: false,
    },
    // …\0cidx{_pub}\0{index}\0{col}…\0{~rev}\0{node_id}
    GcTarget {
        cf: cf::COMPOUND_INDEX,
        branch_scoped: true,
        layouts: &[
            t(4, "cidx", BeforeTrailing(1)),
            t(4, "cidx_pub", BeforeTrailing(1)),
        ],
        drop_orphan_tombstones: false,
    },
    // …\0uniq\0{node_type}\0{property}\0{value_hash}\0{~rev}
    GcTarget {
        cf: cf::UNIQUE_INDEX,
        branch_scoped: true,
        layouts: &[t(4, "uniq", Tail)],
        drop_orphan_tombstones: false,
    },
    // fwd: …\0lname\0{locale}\0{parent_id}\0{name}\0{node_id}\0{~rev}
    // rev: …\0lname_of\0{node_id}\0{locale}\0{~rev}
    // One group per node (forward) and per (node, locale) (reverse), so
    // retention never lets one node's tombstone outlive another's claim.
    GcTarget {
        cf: cf::LOCALIZED_NAME_INDEX,
        branch_scoped: true,
        layouts: &[t(4, "lname", Tail), t(4, "lname_of", Tail)],
        drop_orphan_tombstones: false,
    },
    // v2:     …\0{embedder_hash}\0{kind}\0{source_id}\0{chunk_idx}\0{~rev}
    // legacy: …\0{node_id}\0{~rev}
    GcTarget {
        cf: cf::EMBEDDINGS,
        branch_scoped: true,
        layouts: &[TagLayout {
            tag_index: 4,
            tag: None,
            locator: Tail,
        }],
        drop_orphan_tombstones: false,
    },
    // {t}\0{r}\0snapshots\0{node_id}\0{~rev}
    // {t}\0{r}\0trans_snapshots\0{node_id}\0{locale}\0{~rev}
    // (`revisions\0{~rev}` — the commit log itself — is deliberately NOT here.)
    GcTarget {
        cf: cf::REVISIONS,
        branch_scoped: false,
        layouts: &[t(2, "snapshots", Tail), t(2, "trans_snapshots", Tail)],
        drop_orphan_tombstones: false,
    },
];

/// Positions of the parts of a recognised versioned key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Located {
    /// End (exclusive) of the null-free scope segments: `{t}\0{r}` for a
    /// repository-wide key, `{t}\0{r}\0{b}` for a branch-scoped one.
    pub scope_end: usize,
    /// Start of the 16-byte revision.
    pub rev_start: usize,
}

impl Located {
    /// Bytes before the revision separator — keys sharing it are contiguous.
    pub fn chunk<'k>(&self, key: &'k [u8]) -> &'k [u8] {
        &key[..self.rev_start - 1]
    }

    /// Bytes after the revision (empty for `Tail`), which together with
    /// [`Self::chunk`] identify the group.
    pub fn tail<'k>(&self, key: &'k [u8]) -> &'k [u8] {
        // Skip the separator after the revision, when there is anything after it.
        &key[(self.rev_start + 17).min(key.len())..]
    }

    pub fn revision<'k>(&self, key: &'k [u8]) -> &'k [u8] {
        &key[self.rev_start..self.rev_start + 16]
    }
}

/// Offsets of the first `n` null separators, or `None` if the key has fewer.
fn leading_separators(key: &[u8], n: usize) -> Option<Vec<usize>> {
    let mut out = Vec::with_capacity(n);
    let mut from = 0;
    while out.len() < n {
        let pos = from + key[from..].iter().position(|&b| b == 0)?;
        out.push(pos);
        from = pos + 1;
    }
    Some(out)
}

/// Recognise a versioned key of `target`, or `None` to leave it alone.
pub(super) fn locate(target: &GcTarget, key: &[u8]) -> Option<Located> {
    for layout in target.layouts {
        // The tag and every segment before it are null-free, so the first
        // `tag_index + 1` separators are real separators.
        let seps = match leading_separators(key, layout.tag_index + 1) {
            Some(s) => s,
            None => continue,
        };
        let tag_start = if layout.tag_index == 0 {
            0
        } else {
            seps[layout.tag_index - 1] + 1
        };
        let tag_end = seps[layout.tag_index];
        if let Some(tag) = layout.tag {
            if &key[tag_start..tag_end] != tag.as_bytes() {
                continue;
            }
        }

        let boundary = match layout.locator {
            Locator::Tail => key.len(),
            Locator::BeforeTrailing(n) => {
                let mut b = key.len();
                for _ in 0..n {
                    b = key[..b].iter().rposition(|&x| x == 0)?;
                }
                b
            }
        };
        let rev_start = boundary.checked_sub(16)?;
        // The revision must be its own segment, strictly after the tag: a
        // separator right before it, and at least one segment between the
        // tag and it (the entity id). Anything else is a different shape.
        if rev_start <= tag_end + 1 || key[rev_start - 1] != 0 {
            continue;
        }

        let scope_segments = if target.branch_scoped { 3 } else { 2 };
        let scope_end = *seps.get(scope_segments - 1)?;
        if scope_end >= rev_start {
            continue;
        }
        return Some(Located {
            scope_end,
            rev_start,
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys;
    use raisin_hlc::HLC;

    fn target(cf_name: &str) -> &'static GcTarget {
        GC_TARGETS.iter().find(|t| t.cf == cf_name).unwrap()
    }

    /// A revision whose descending encoding contains null bytes — the case
    /// that breaks any split-on-null parser.
    fn nully_rev() -> HLC {
        HLC::new(u64::MAX, u64::MAX)
    }

    #[test]
    fn node_keys_are_located_from_the_tail() {
        let rev = nully_rev();
        let key = keys::node_key_versioned("t", "r", "main", "ws", "node-1", &rev);
        let loc = locate(target(cf::NODES), &key).expect("versioned node key");
        assert_eq!(loc.revision(&key), &rev.encode_descending());
        assert_eq!(loc.tail(&key), b"");
        assert_eq!(&key[..loc.scope_end], b"t\0r\0main");
    }

    #[test]
    fn property_index_keys_keep_the_node_id_in_the_group() {
        let rev = nully_rev();
        let key =
            keys::property_index_key_versioned("t", "r", "main", "ws", "p", "h", &rev, "n1", false);
        let loc = locate(target(cf::PROPERTY_INDEX), &key).expect("versioned prop key");
        assert_eq!(loc.revision(&key), &rev.encode_descending());
        assert_eq!(loc.tail(&key), b"n1");
    }

    #[test]
    fn unversioned_and_unknown_shapes_are_left_alone() {
        // An unversioned node key: its last 16 bytes are part of the id.
        let key = keys::node_key("t", "r", "main", "ws", "abcdefghijklmnopqrstu");
        assert!(locate(target(cf::NODES), &key).is_none());
        // Unknown tag in a known CF.
        let key = keys::KeyBuilder::new()
            .push("t")
            .push("r")
            .push("main")
            .push("ws")
            .push("something_else")
            .push("x")
            .push_revision(&HLC::new(1, 1))
            .build();
        assert!(locate(target(cf::NODES), &key).is_none());
        // Too short to hold an entity and a revision.
        assert!(locate(target(cf::NODES), b"t\0r\0main\0ws\0nodes").is_none());
    }

    #[test]
    fn global_relations_are_located_before_four_segments() {
        let rev = nully_rev();
        let key = keys::relation_global_key_versioned(
            "t", "r", "main", "type", &rev, "ws1", "a", "ws2", "b",
        );
        let loc = locate(target(cf::RELATION_INDEX), &key).expect("global relation");
        assert_eq!(loc.revision(&key), &rev.encode_descending());
        assert_eq!(loc.tail(&key), b"ws1\0a\0ws2\0b");
    }

    #[test]
    fn snapshots_are_repository_scoped() {
        let rev = HLC::new(5, 0);
        let key = keys::node_snapshot_key("t", "r", "n1", &rev);
        let loc = locate(target(cf::REVISIONS), &key).expect("snapshot");
        assert_eq!(&key[..loc.scope_end], b"t\0r");
        // The commit log itself is never a GC target.
        let meta = keys::revision_meta_key("t", "r", &rev);
        assert!(locate(target(cf::REVISIONS), &meta).is_none());
    }

    #[test]
    fn every_target_is_a_real_column_family() {
        let real: std::collections::HashSet<&str> =
            crate::all_column_families().into_iter().collect();
        for t in GC_TARGETS {
            assert!(real.contains(t.cf), "{} is not a column family", t.cf);
        }
    }
}
