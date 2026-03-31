//! Property index, compound index, and unique index key functions
//!
//! Keys for property-based lookups, multi-column compound indexes,
//! and unique constraint enforcement.

use super::KeyBuilder;
use raisin_hlc::HLC;

// Re-export CompoundColumnValue from raisin_storage for key encoding
pub use raisin_storage::CompoundColumnValue;

/// Property index key: {tenant}\0{repo}\0{branch}\0{workspace}\0prop{_pub}\0{property_name}\0{value_hash}\0{node_id}
#[deprecated(
    since = "0.1.0",
    note = "Use property_index_key_versioned for revision-aware storage"
)]
pub fn property_index_key(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    property_name: &str,
    value_hash: &str,
    node_id: &str,
    published: bool,
) -> Vec<u8> {
    let tag = if published { "prop_pub" } else { "prop" };
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push(workspace)
        .push(tag)
        .push(property_name)
        .push(value_hash)
        .push(node_id)
        .build()
}

/// Revision-aware property index key
pub fn property_index_key_versioned(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    property_name: &str,
    value_hash: &str,
    revision: &HLC,
    node_id: &str,
    published: bool,
) -> Vec<u8> {
    let tag = if published { "prop_pub" } else { "prop" };
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push(workspace)
        .push(tag)
        .push(property_name)
        .push(value_hash)
        .push_revision(revision)
        .push(node_id)
        .build()
}

/// Property index key with timestamp value (big-endian i64 microseconds for correct sorting)
pub fn property_index_key_versioned_timestamp(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    property_name: &str,
    timestamp_micros: i64,
    revision: &HLC,
    node_id: &str,
    published: bool,
) -> Vec<u8> {
    let tag = if published { "prop_pub" } else { "prop" };
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push(workspace)
        .push(tag)
        .push(property_name)
        .push_bytes(&timestamp_micros.to_be_bytes())
        .push_revision(revision)
        .push(node_id)
        .build()
}

// --- Compound Index Keys ---

/// Compound index tag for key encoding
pub const COMPOUND_INDEX_TAG: &str = "cidx";
/// Compound index tag for published nodes
pub const COMPOUND_INDEX_TAG_PUB: &str = "cidx_pub";

/// Build a compound index key builder with column values encoded
fn build_compound_key(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    index_name: &str,
    column_values: &[CompoundColumnValue],
    published: bool,
) -> KeyBuilder {
    let tag = if published {
        COMPOUND_INDEX_TAG_PUB
    } else {
        COMPOUND_INDEX_TAG
    };
    let mut builder = KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push(workspace)
        .push(tag)
        .push(index_name);

    for col_val in column_values {
        builder = match col_val {
            CompoundColumnValue::String(s) => builder.push(s),
            CompoundColumnValue::Integer(i) => builder.push_bytes(&i.to_be_bytes()),
            CompoundColumnValue::TimestampDesc(ts) => builder.push_bytes(&(!ts).to_be_bytes()),
            CompoundColumnValue::TimestampAsc(ts) => builder.push_bytes(&ts.to_be_bytes()),
            CompoundColumnValue::Boolean(b) => builder.push_bytes(&[if *b { 1 } else { 0 }]),
        };
    }

    builder
}

/// Compound index key versioned
pub fn compound_index_key_versioned(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    index_name: &str,
    column_values: &[CompoundColumnValue],
    revision: &HLC,
    node_id: &str,
    published: bool,
) -> Vec<u8> {
    build_compound_key(
        tenant_id,
        repo_id,
        branch,
        workspace,
        index_name,
        column_values,
        published,
    )
    .push_revision(revision)
    .push(node_id)
    .build()
}

/// Compound index prefix for scanning with specific leading column values
pub fn compound_index_prefix(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    index_name: &str,
    column_values: &[CompoundColumnValue],
    published: bool,
) -> Vec<u8> {
    build_compound_key(
        tenant_id,
        repo_id,
        branch,
        workspace,
        index_name,
        column_values,
        published,
    )
    .build_prefix()
}

/// Compound index workspace prefix: {tenant}\0{repo}\0{branch}\0{workspace}\0cidx{_pub}\0
pub fn compound_index_workspace_prefix(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    published: bool,
) -> Vec<u8> {
    let tag = if published {
        COMPOUND_INDEX_TAG_PUB
    } else {
        COMPOUND_INDEX_TAG
    };
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push(workspace)
        .push(tag)
        .build_prefix()
}

// --- Unique Index Keys ---

/// Unique index tag for key encoding
pub const UNIQUE_INDEX_TAG: &str = "uniq";

/// Unique index key: {tenant}\0{repo}\0{branch}\0{workspace}\0uniq\0{node_type}\0{property_name}\0{value_hash}\0{~revision}
pub fn unique_index_key_versioned(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_type: &str,
    property_name: &str,
    value_hash: &str,
    revision: &HLC,
) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push(workspace)
        .push(UNIQUE_INDEX_TAG)
        .push(node_type)
        .push(property_name)
        .push(value_hash)
        .push_revision(revision)
        .build()
}

/// Unique index value prefix
pub fn unique_index_value_prefix(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_type: &str,
    property_name: &str,
    value_hash: &str,
) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push(workspace)
        .push(UNIQUE_INDEX_TAG)
        .push(node_type)
        .push(property_name)
        .push(value_hash)
        .build_prefix()
}

/// Unique index property prefix
pub fn unique_index_property_prefix(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_type: &str,
    property_name: &str,
) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push(workspace)
        .push(UNIQUE_INDEX_TAG)
        .push(node_type)
        .push(property_name)
        .build_prefix()
}

/// Unique index workspace prefix
pub fn unique_index_workspace_prefix(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push(workspace)
        .push(UNIQUE_INDEX_TAG)
        .build_prefix()
}
