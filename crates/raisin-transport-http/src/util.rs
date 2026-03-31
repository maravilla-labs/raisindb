// SPDX-License-Identifier: BSL-1.1

use raisin_core::NodeService;
use raisin_models::nodes::Node;
use raisin_storage::{transactional::TransactionalStorage, Storage};
use regex::Regex;

/// Populate has_children field for a single node
///
/// This queries the storage layer to check if the node has any children
/// and sets the `has_children` field accordingly.
pub async fn populate_has_children<S: Storage + TransactionalStorage>(
    node: &mut Node,
    node_service: &NodeService<S>,
) -> Result<(), raisin_error::Error> {
    let has_children = node_service.has_children(&node.id).await?;
    node.has_children = Some(has_children);
    Ok(())
}

/// Populate has_children field for multiple nodes
///
/// This is a convenience function that populates `has_children` for a vector of nodes.
pub async fn populate_has_children_batch<S: Storage + TransactionalStorage>(
    nodes: &mut [Node],
    node_service: &NodeService<S>,
) -> Result<(), raisin_error::Error> {
    for node in nodes.iter_mut() {
        populate_has_children(node, node_service).await?;
    }
    Ok(())
}

// Sanitize a single path segment (name) into a safe, URL-friendly slug.
// Rules:
// - trim whitespace
// - lowercase
// - replace any whitespace with '-'
// - keep only [a-z0-9-_.]
// - collapse multiple '-' into single
// - trim leading/trailing '-'
// - reject empty, '..', contains '/' or control chars
pub fn sanitize_name(input: &str) -> Result<String, &'static str> {
    let s = input.trim();
    if s.is_empty() {
        return Err("empty");
    }
    if s == ".." {
        return Err("dot");
    }
    if s.contains('/') {
        return Err("slash");
    }
    if s.chars().any(|c| c.is_control()) {
        return Err("control");
    }

    let lower = s.to_lowercase();
    // replace any whitespace with '-'
    let ws_re = Regex::new(r"\s+").expect("hardcoded regex pattern is valid");
    let mut slug = ws_re.replace_all(&lower, "-").to_string();
    // remove invalid chars
    let filtered: String = slug
        .chars()
        .filter(|c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-' || *c == '_' || *c == '.'
        })
        .collect();
    // collapse multiple '-'
    let dash_re = Regex::new(r"-+").expect("hardcoded regex pattern is valid");
    slug = dash_re.replace_all(&filtered, "-").to_string();
    // trim dashes
    slug = slug.trim_matches('-').to_string();

    if slug.is_empty() {
        return Err("empty");
    }
    Ok(slug)
}
