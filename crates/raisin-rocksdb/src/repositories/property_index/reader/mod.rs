//! The one revision-bounded reader of `PROPERTY_INDEX`.
//!
//! Key: `{tenant}\0{repo}\0{branch}\0{ws}\0prop[_pub]\0{property}\0{value}\0{~rev:16}\0{node_id}`.
//!
//! Every equality lookup, existence scan, ordered scan, range scan and
//! `list_by_type` reads the index through here. There used to be five
//! hand-written loops, and they disagreed: some treated an empty value as a
//! tombstone and some as live, none was revision-bounded, and all of them kept
//! a NODE-WIDE tombstone set, so a node's tombstone under its OLD value
//! (written when the value changed) hid its live entry under the NEW value —
//! which is how `ORDER BY updated_at ASC` dropped every node ever updated.
//!
//! # The rule
//!
//! Decisions are per `(value, node)`. Within one value's key range, entries
//! run newest revision first, so the first entry seen for a node at or below
//! the bound `at` is that pair's state at `at`: a live value (anything but
//! `T`, empty included) is a match, `T` is not. `at` is the caller's revision,
//! else the branch HEAD read once — which also hides entries stranded above
//! HEAD. A value group is entered with ONE seek to `{value}\0{~at}`.
//!
//! What this does NOT do: seek past one node's remaining revisions. Within a
//! value the key orders by revision BEFORE node id, so nodes interleave and
//! such a seek would skip other nodes' live entries.
//!
//! # Candidates, not answers
//!
//! The result is the set of nodes the index says match. Orphan live entries
//! left by historically buggy writers survive until a rebuild, so callers that
//! must be exact re-check the decoded node (see `orphans.rs`).

mod walk;

use crate::repositories::nodes::hash_property_value;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::PropertyScanEntry;
use rocksdb::DB;
use std::collections::HashSet;

pub(crate) use walk::ValueBound;

/// Pseudo-properties whose index value is the 8-byte big-endian i64 of the
/// timestamp in microseconds, not `hash_property_value`. The writers are
/// `keys::property_index_key_versioned_timestamp` callers.
const TIMESTAMP_PROPERTIES: [&str; 2] = ["__created_at", "__updated_at"];

pub(crate) fn is_timestamp_property(property_name: &str) -> bool {
    TIMESTAMP_PROPERTIES.contains(&property_name)
}

/// The value bytes a key carries for `value` under `property_name` — the ONE
/// encoder for both an equality probe and a range bound, matching the writers.
pub(crate) fn encode_value(property_name: &str, value: &PropertyValue) -> Vec<u8> {
    if is_timestamp_property(property_name) {
        match value {
            PropertyValue::Date(date) => return date.timestamp_micros().to_be_bytes().to_vec(),
            PropertyValue::Integer(micros) => return micros.to_be_bytes().to_vec(),
            other => tracing::warn!(
                property = property_name,
                "timestamp property compared with a non-timestamp value {:?}; it cannot match",
                other
            ),
        }
    }
    hash_property_value(value).into_bytes()
}

/// How a stored value is reported in a [`PropertyScanEntry`]: timestamp
/// properties as their microseconds in decimal, everything else as text.
pub(crate) fn display_value(property_name: &str, value: &[u8]) -> String {
    if is_timestamp_property(property_name) {
        if let Ok(bytes) = <[u8; 8]>::try_from(value) {
            return i64::from_be_bytes(bytes).to_string();
        }
    }
    String::from_utf8_lossy(value).into_owned()
}

/// The branch HEAD, read once per scan. `None` when the branch record does not
/// exist (a tag, or a branch being created): the scan is then unbounded, as
/// every reader was before.
fn branch_head(db: &DB, tenant_id: &str, repo_id: &str, branch: &str) -> Result<Option<HLC>> {
    let cf = cf_handle(db, cf::BRANCHES)?;
    let bytes = db
        .get_cf(cf, keys::branch_key(tenant_id, repo_id, branch))
        .map_err(|e| raisin_error::Error::storage(e.to_string()))?;
    match bytes {
        Some(bytes) => rmp_serde::from_slice::<raisin_context::Branch>(&bytes)
            .map(|b| Some(b.head))
            .map_err(|e| raisin_error::Error::storage(format!("Branch decode error: {}", e))),
        None => Ok(None),
    }
}

/// One property of one workspace, read as of one revision.
pub(crate) struct PropertyIndexReader<'a> {
    db: &'a DB,
    property_name: &'a str,
    /// `{..}\0prop[_pub]\0{property}\0` — every value of the property.
    base: Vec<u8>,
    at: Option<HLC>,
}

impl<'a> PropertyIndexReader<'a> {
    /// `max_revision` bounds the read; `None` reads at the branch HEAD.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        db: &'a DB,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        property_name: &'a str,
        published: bool,
        max_revision: Option<&HLC>,
    ) -> Result<Self> {
        let tag = if published { "prop_pub" } else { "prop" };
        let base = keys::KeyBuilder::new()
            .push(tenant_id)
            .push(repo_id)
            .push(branch)
            .push(workspace)
            .push(tag)
            .push(property_name)
            .build_prefix();
        let at = match max_revision {
            Some(revision) => Some(*revision),
            None => branch_head(db, tenant_id, repo_id, branch)?,
        };
        Ok(Self {
            db,
            property_name,
            base,
            at,
        })
    }

    /// Nodes whose `property_name` equals `value`, at most `limit` of them.
    pub(crate) fn find(&self, value: &PropertyValue, limit: Option<usize>) -> Result<Vec<String>> {
        let encoded = encode_value(self.property_name, value);
        let mut nodes = Vec::new();
        if limit == Some(0) {
            return Ok(nodes);
        }
        walk::visit_value(self.db, &self.base, &encoded, self.at.as_ref(), |node_id| {
            nodes.push(node_id.to_string());
            Ok(!limit.is_some_and(|limit| nodes.len() >= limit))
        })?;
        Ok(nodes)
    }

    /// How many nodes have `property_name == value`.
    pub(crate) fn count(&self, value: &PropertyValue) -> Result<usize> {
        Ok(self.find(value, None)?.len())
    }

    /// Nodes that have the property at all, any value.
    pub(crate) fn nodes_with_property(&self) -> Result<Vec<String>> {
        Ok(self
            .scan_encoded(ValueBound::Unbounded, ValueBound::Unbounded, true, None)?
            .into_iter()
            .map(|entry| entry.node_id)
            .collect())
    }

    /// Nodes in value order, each once, at most `limit`.
    pub(crate) fn scan(
        &self,
        lower: Option<(&PropertyValue, bool)>,
        upper: Option<(&PropertyValue, bool)>,
        ascending: bool,
        limit: Option<usize>,
    ) -> Result<Vec<PropertyScanEntry>> {
        let bound = |b: Option<(&PropertyValue, bool)>| match b {
            Some((value, inclusive)) => ValueBound::Value {
                bytes: encode_value(self.property_name, value),
                inclusive,
            },
            None => ValueBound::Unbounded,
        };
        self.scan_encoded(bound(lower), bound(upper), ascending, limit)
    }

    /// Every live `(value, node)` pair in value order, NOT deduplicated per
    /// node — what the orphan detector needs, since an orphan is precisely a
    /// node live under a value it no longer has.
    pub(crate) fn for_each_live_pair(
        &self,
        mut visit: impl FnMut(&[u8], &str) -> Result<bool>,
    ) -> Result<()> {
        walk::visit_values(
            self.db,
            &self.base,
            &ValueBound::Unbounded,
            &ValueBound::Unbounded,
            true,
            self.at.as_ref(),
            |value, node_id| visit(value, node_id),
        )
    }

    fn scan_encoded(
        &self,
        lower: ValueBound,
        upper: ValueBound,
        ascending: bool,
        limit: Option<usize>,
    ) -> Result<Vec<PropertyScanEntry>> {
        let mut seen: HashSet<String> = HashSet::new();
        let mut out = Vec::new();
        if limit == Some(0) {
            return Ok(out);
        }
        walk::visit_values(
            self.db,
            &self.base,
            &lower,
            &upper,
            ascending,
            self.at.as_ref(),
            |value, node_id| {
                // An orphan can leave one node live under two values; it is
                // reported once, at the first value in scan order.
                if seen.insert(node_id.to_string()) {
                    out.push(PropertyScanEntry {
                        node_id: node_id.to_string(),
                        property_value: display_value(self.property_name, value),
                    });
                }
                Ok(!limit.is_some_and(|limit| out.len() >= limit))
            },
        )?;
        Ok(out)
    }
}
