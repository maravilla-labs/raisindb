// SPDX-License-Identifier: BSL-1.1

//! THE index path builder. There is exactly one, and everything that needs to
//! name an index file on disk goes through it.
//!
//! This used to be two. `engine/mod.rs::get_index_path` split a `/`-joined key
//! and called `.with_extension("hnsw")` on the last segment, while
//! `raisin-rocksdb`'s `hnsw_transfer` built
//! `base.join(tenant).join(repo).join(format!("{branch}.hnsw"))`
//! independently. They had already drifted in two ways, one of them a live
//! data-loss bug:
//!
//! * `.with_extension` REPLACES an existing extension, so branches `release.2`
//!   and `release.3` both resolved to `release.hnsw` and silently shared one
//!   index; the transfer path's `format!` did not;
//! * a grep for "meta" across the whole transfer module returned only
//!   `fs::metadata` — the `.hnsw.meta` sidecar was never shipped, and a peer
//!   receiving a bare `.hnsw` routed it into the bincode format migration,
//!   which fails. The index was gone, AND the receiver had already renamed the
//!   healthy local one out of the way.
//!
//! Both are structural consequences of a second implementation, so the fix is
//! to delete the second implementation rather than to patch it.
//!
//! # Layout
//!
//! ```text
//! <base>/<tenant>/<repo>/<branch>/<partition>.hnsw
//! <base>/<tenant>/<repo>/<branch>/<partition>.hnsw.meta
//! ```
//!
//! The branch is a DIRECTORY component (which is what removes the
//! `release.2` / `release.3` collision — a directory name keeps its dots) and
//! the partition token is the file stem, appended with `format!` so no
//! extension is ever replaced.

use crate::partition::PartitionId;
use std::fmt;
use std::path::{Path, PathBuf};

/// Identifies one index: a branch's slice of one embedding space.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IndexKey {
    pub tenant_id: String,
    pub repo_id: String,
    pub branch: String,
    pub partition: PartitionId,
}

impl IndexKey {
    /// Build a key from its components.
    pub fn new(tenant_id: &str, repo_id: &str, branch: &str, partition: &PartitionId) -> Self {
        Self {
            tenant_id: tenant_id.to_string(),
            repo_id: repo_id.to_string(),
            branch: branch.to_string(),
            partition: partition.clone(),
        }
    }

    /// The directory holding every partition of this branch.
    pub fn branch_dir(&self, base: &Path) -> PathBuf {
        base.join(&self.tenant_id)
            .join(&self.repo_id)
            .join(&self.branch)
    }

    /// The `.hnsw` graph file.
    pub fn index_path(&self, base: &Path) -> PathBuf {
        self.branch_dir(base)
            .join(format!("{}.hnsw", self.partition))
    }

    /// The `.hnsw.meta` sidecar. Always shipped, copied and deleted with the
    /// graph file — an index without its sidecar is not an index.
    pub fn meta_path(&self, base: &Path) -> PathBuf {
        crate::persistence::meta_path_for(&self.index_path(base))
    }

    /// Where this index lived BEFORE partitioning: `<base>/<t>/<r>/<branch>.hnsw`.
    ///
    /// Used by the one-time lazy rename in `get_or_load_index`. See
    /// [`crate::engine::migrate_legacy_layout`].
    pub fn legacy_index_path(&self, base: &Path) -> PathBuf {
        base.join(&self.tenant_id)
            .join(&self.repo_id)
            .join(format!("{}.hnsw", self.branch))
    }
}

impl fmt::Display for IndexKey {
    /// The form that appears in logs and in `SHOW VECTOR INDEX HEALTH`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}/{}/{}/{}",
            self.tenant_id, self.repo_id, self.branch, self.partition
        )
    }
}

/// The `.hnsw` path for one index. Free function for callers that hold a base
/// directory but no engine — the replication transfer path, chiefly.
pub fn index_path(
    base: &Path,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    partition: &PartitionId,
) -> PathBuf {
    IndexKey::new(tenant_id, repo_id, branch, partition).index_path(base)
}

/// The `.hnsw.meta` path beside a given `.hnsw` path.
pub fn meta_path(index_path: &Path) -> PathBuf {
    crate::persistence::meta_path_for(index_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dotted_branch_no_longer_collides() {
        // `.with_extension("hnsw")` on `release.2` produced `release.hnsw`,
        // which is also what `release.3` produced. The two branches shared one
        // index file and neither knew.
        let base = Path::new("/idx");
        let p = PartitionId::new("hashT");
        let a = index_path(base, "t", "r", "release.2", &p);
        let b = index_path(base, "t", "r", "release.3", &p);
        assert_ne!(a, b);
        assert_eq!(a, Path::new("/idx/t/r/release.2/hashT.hnsw"));
        assert_eq!(b, Path::new("/idx/t/r/release.3/hashT.hnsw"));
    }

    #[test]
    fn two_partitions_of_one_branch_are_separate_files_in_one_directory() {
        let base = Path::new("/idx");
        let text = index_path(base, "t", "r", "main", &PartitionId::new("aaaaaaaaaaaT"));
        let image = index_path(base, "t", "r", "main", &PartitionId::new("bbbbbbbbbbbI"));
        assert_ne!(text, image);
        assert_eq!(text.parent(), image.parent());
    }

    #[test]
    fn the_sidecar_sits_beside_the_graph() {
        let key = IndexKey::new("t", "r", "main", &PartitionId::new("hashT"));
        let base = Path::new("/idx");
        assert_eq!(
            key.meta_path(base),
            Path::new("/idx/t/r/main/hashT.hnsw.meta")
        );
    }

    #[test]
    fn the_legacy_path_is_the_pre_partition_layout() {
        let key = IndexKey::new("t", "r", "main", &PartitionId::new("hashT"));
        assert_eq!(
            key.legacy_index_path(Path::new("/idx")),
            Path::new("/idx/t/r/main.hnsw")
        );
    }
}
