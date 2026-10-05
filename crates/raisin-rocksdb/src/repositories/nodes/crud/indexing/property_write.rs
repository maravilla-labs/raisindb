//! How a node write's PROPERTY_INDEX entries are written (plan Phase 7):
//! the repository's door to the one delta writer.

use super::super::super::NodeRepositoryImpl;
use crate::indexing::{Baseline, InPlace, IndexCtx, PropertyIndexTarget};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use rocksdb::WriteBatch;

/// How a node write's PROPERTY_INDEX entries are written: against which
/// baseline, and whether the revision is a reused (`versionable=false`) one
/// (with the in-place targets resolved up front when the flag is on).
/// See `crate::indexing::property_delta`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PropertyWrite<'a> {
    pub(crate) baseline: Baseline<'a>,
    pub(crate) in_place: InPlace<'a>,
}

impl<'a> PropertyWrite<'a> {
    /// A create (copy, deep create, import): no prior version on the branch.
    pub(crate) const CREATE: PropertyWrite<'static> = PropertyWrite {
        baseline: Baseline::NoPrior,
        in_place: InPlace::No,
    };

    /// A full put against `old` at a fresh revision.
    pub(crate) fn full(old: Option<&'a raisin_models::nodes::Node>) -> Self {
        Self {
            baseline: Baseline::Full(old),
            in_place: InPlace::No,
        }
    }
}

impl NodeRepositoryImpl {
    /// Stage `node`'s PROPERTY_INDEX entries through the ONE writer.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn write_property_entries(
        &self,
        batch: &mut WriteBatch,
        node: &Node,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        revision: &HLC,
        write: PropertyWrite<'_>,
    ) -> Result<crate::indexing::DeltaCounts> {
        crate::indexing::write_property_index_delta(
            batch,
            PropertyIndexTarget::from_db(&self.db)?,
            &IndexCtx::new(tenant_id, repo_id, branch, workspace),
            write.baseline,
            node,
            revision,
            write.in_place,
        )
    }
}
