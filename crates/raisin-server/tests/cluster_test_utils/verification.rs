// Verification utilities for cluster consistency testing

use super::rest_client::RestClient;
use anyhow::{Context, Result};
use serde_json::Value;
use std::time::{Duration, Instant};
use tokio::time::sleep;

/// Verify that all nodes return children in the same natural order via REST API
///
/// This is the key test for child ordering consistency. The "natural order" comes from
/// the fragmented index, NOT from explicit ORDER BY clauses.
pub async fn verify_child_order_via_rest(
    client: &RestClient,
    tokens: &[String],
    repo: &str,
    branch: &str,
    workspace: &str,
    parent_path: &str,
) -> Result<()> {
    let mut all_children = Vec::new();
    let node_labels: Vec<String> = (0..tokens.len())
        .map(|i| format!("node{}", i + 1))
        .collect();

    // Fetch children from all nodes
    for (idx, (url, token)) in client.base_urls.iter().zip(tokens.iter()).enumerate() {
        let children = client
            .list_children(url, token, repo, branch, workspace, parent_path)
            .await
            .with_context(|| format!("Failed to list children from {}", node_labels[idx]))?;
        all_children.push(children);
    }

    // Extract ID sequences
    let id_sequences: Vec<Vec<String>> = all_children
        .iter()
        .map(|children| {
            children
                .iter()
                .filter_map(|child| child["id"].as_str().map(|s| s.to_string()))
                .collect()
        })
        .collect();

    // Verify all nodes return the same order
    let expected_count = tokens.len();
    if id_sequences.len() != expected_count {
        anyhow::bail!(
            "Expected {} node responses, got {}",
            expected_count,
            id_sequences.len()
        );
    }

    let reference_order = &id_sequences[0];

    for (idx, order) in id_sequences.iter().enumerate().skip(1) {
        if order != reference_order {
            // Provide detailed debug output
            println!("\n=== CHILD ORDER MISMATCH ===");
            let node_label_refs: Vec<&str> = node_labels.iter().map(|s| s.as_str()).collect();
            dump_children_order(&all_children, &node_label_refs);
            anyhow::bail!(
                "Child order mismatch between node1 and {}: {:?} vs {:?}",
                node_labels[idx],
                reference_order,
                order
            );
        }
    }

    Ok(())
}

/// Verify that SQL query without ORDER BY returns the expected natural order
///
/// This verifies that the fragmented index provides consistent ordering in SQL queries.
pub async fn verify_child_order_via_sql(
    client: &RestClient,
    token: &str,
    node_url: &str,
    repo: &str,
    workspace: &str,
    parent_path: &str,
    expected_ids: &[String],
) -> Result<()> {
    // Query without ORDER BY - should return natural order from fragmented index
    let sql = format!("SELECT id FROM {} WHERE parent_path = $1", workspace);

    let result = client
        .execute_sql(
            node_url,
            token,
            repo,
            &sql,
            vec![Value::String(parent_path.to_string())],
        )
        .await
        .context("SQL query failed")?;

    // Extract IDs from result
    let rows = result["rows"].as_array().context("No rows in SQL result")?;

    let actual_ids: Vec<String> = rows
        .iter()
        .filter_map(|row| {
            row.as_array()
                .and_then(|arr| arr.first())
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .collect();

    if actual_ids != expected_ids {
        anyhow::bail!(
            "SQL query order mismatch. Expected: {:?}, Got: {:?}",
            expected_ids,
            actual_ids
        );
    }

    Ok(())
}

/// Verify that a node exists on all three nodes
pub async fn verify_node_exists_on_all_nodes(
    client: &RestClient,
    tokens: &[String],
    repo: &str,
    branch: &str,
    workspace: &str,
    path: &str,
) -> Result<()> {
    for (idx, (url, token)) in client.base_urls.iter().zip(tokens.iter()).enumerate() {
        let node = client
            .get_node(url, token, repo, branch, workspace, path)
            .await
            .with_context(|| format!("Failed to get node from node{}", idx + 1))?;

        if node.is_none() {
            anyhow::bail!("Node at path '{}' not found on node{}", path, idx + 1);
        }
    }

    Ok(())
}

/// Verify that node properties match across all nodes
pub async fn verify_node_properties_match(
    client: &RestClient,
    tokens: &[String],
    repo: &str,
    branch: &str,
    workspace: &str,
    path: &str,
) -> Result<()> {
    let mut nodes = Vec::new();

    // Fetch node from all nodes
    for (url, token) in client.base_urls.iter().zip(tokens.iter()) {
        let node = client
            .get_node(url, token, repo, branch, workspace, path)
            .await?
            .context("Node not found")?;
        nodes.push(node);
    }

    // Compare properties (excluding timestamps and version fields)
    let reference = &nodes[0];
    let ref_props = reference["properties"].clone();

    for (idx, node) in nodes.iter().enumerate().skip(1) {
        let props = &node["properties"];
        if props != &ref_props {
            anyhow::bail!(
                "Properties mismatch between node1 and node{}: {:?} vs {:?}",
                idx + 1,
                ref_props,
                props
            );
        }
    }

    Ok(())
}

/// Verify that relations match across all nodes
pub async fn verify_relations_match(
    client: &RestClient,
    tokens: &[String],
    repo: &str,
    branch: &str,
    workspace: &str,
    source_path: &str,
    expected_count: usize,
) -> Result<()> {
    for (idx, (url, token)) in client.base_urls.iter().zip(tokens.iter()).enumerate() {
        let relations = client
            .get_relations(url, token, repo, branch, workspace, source_path)
            .await
            .with_context(|| format!("Failed to get relations from node{}", idx + 1))?;

        if relations.len() != expected_count {
            anyhow::bail!(
                "Relation count mismatch on node{}: expected {}, got {}",
                idx + 1,
                expected_count,
                relations.len()
            );
        }
    }

    Ok(())
}

/// Wait for a node to replicate to all nodes with timeout
pub async fn wait_for_replication(
    client: &RestClient,
    tokens: &[String],
    repo: &str,
    branch: &str,
    workspace: &str,
    path: &str,
    timeout: Duration,
) -> Result<()> {
    let start = std::time::Instant::now();

    loop {
        if start.elapsed() > timeout {
            anyhow::bail!(
                "Node at path '{}' did not replicate to all nodes within {:?}",
                path,
                timeout
            );
        }

        // Try to get node from all nodes
        let mut all_present = true;
        for (url, token) in client.base_urls.iter().zip(tokens.iter()) {
            match client
                .get_node(url, token, repo, branch, workspace, path)
                .await
            {
                Ok(Some(_)) => continue,
                _ => {
                    all_present = false;
                    break;
                }
            }
        }

        if all_present {
            return Ok(());
        }

        sleep(Duration::from_millis(100)).await;
    }
}

/// Wait for a node to replicate by ID to all nodes with timeout
pub async fn wait_for_replication_by_id(
    client: &RestClient,
    tokens: &[String],
    repo: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    timeout: Duration,
) -> Result<()> {
    let start = std::time::Instant::now();

    loop {
        if start.elapsed() > timeout {
            anyhow::bail!(
                "Node with ID '{}' did not replicate to all nodes within {:?}",
                node_id,
                timeout
            );
        }

        // Try to get node by ID from all nodes
        let mut all_present = true;
        for (url, token) in client.base_urls.iter().zip(tokens.iter()) {
            match client
                .get_node_by_id(url, token, repo, branch, workspace, node_id)
                .await
            {
                Ok(Some(_)) => continue,
                _ => {
                    all_present = false;
                    break;
                }
            }
        }

        if all_present {
            return Ok(());
        }

        sleep(Duration::from_millis(100)).await;
    }
}

/// Wait for a NodeType to replicate to all nodes
pub async fn wait_for_nodetype_replication(
    client: &RestClient,
    tokens: &[String],
    repo: &str,
    branch: &str,
    node_type_name: &str,
    timeout: Duration,
) -> Result<()> {
    let start = Instant::now();
    let mut last_report = Instant::now();
    let mut last_missing: Vec<String> = Vec::new();

    loop {
        if start.elapsed() > timeout {
            anyhow::bail!(
                "NodeType '{}' did not replicate to all nodes within {:?}. Missing: {}",
                node_type_name,
                timeout,
                if last_missing.is_empty() {
                    "unknown".to_string()
                } else {
                    last_missing.join(", ")
                }
            );
        }

        let mut missing = Vec::new();
        for (idx, (url, token)) in client.base_urls.iter().zip(tokens.iter()).enumerate() {
            match client
                .get_node_type(url, token, repo, branch, node_type_name)
                .await
            {
                Ok(Some(_)) => continue,
                Ok(None) => missing.push(format!("node{} (not found)", idx + 1)),
                Err(err) => missing.push(format!("node{} error: {}", idx + 1, err)),
            }
        }

        if missing.is_empty() {
            return Ok(());
        }

        last_missing = missing.clone();

        if last_report.elapsed() >= Duration::from_secs(1) {
            println!(
                "    Waiting for NodeType '{}' to replicate... still missing {}",
                node_type_name,
                missing.join(", ")
            );
            last_report = Instant::now();
        }
        sleep(Duration::from_millis(100)).await;
    }
}

/// Verify that a post appears at the same position (index) on all nodes
pub async fn verify_post_at_same_position(
    client: &RestClient,
    tokens: &[String],
    repo: &str,
    branch: &str,
    workspace: &str,
    parent_path: &str,
    post_id: &str,
) -> Result<()> {
    let mut positions = Vec::new();
    let node_labels: Vec<String> = (0..tokens.len())
        .map(|i| format!("node{}", i + 1))
        .collect();

    // Fetch children from all nodes and find position of post_id
    for (idx, (url, token)) in client.base_urls.iter().zip(tokens.iter()).enumerate() {
        let children = client
            .list_children(url, token, repo, branch, workspace, parent_path)
            .await
            .with_context(|| format!("Failed to list children from {}", node_labels[idx]))?;

        let position = children
            .iter()
            .position(|child| child["id"].as_str() == Some(post_id));

        match position {
            Some(pos) => positions.push((node_labels[idx].clone(), Some(pos))),
            None => positions.push((node_labels[idx].clone(), None)),
        }
    }

    // Verify all positions match
    let reference_pos = positions[0].1;

    for (label, pos) in &positions[1..] {
        if pos != &reference_pos {
            anyhow::bail!(
                "Post position mismatch: {} at {:?}, {} at {:?}",
                positions[0].0,
                reference_pos,
                label,
                pos
            );
        }
    }

    if reference_pos.is_none() {
        anyhow::bail!("Post '{}' not found on any node", post_id);
    }

    Ok(())
}

/// Verify that a comment exists on all nodes
pub async fn verify_comment_exists_on_all_nodes(
    client: &RestClient,
    tokens: &[String],
    repo: &str,
    branch: &str,
    workspace: &str,
    comment_path: &str,
    timeout: Duration,
) -> Result<()> {
    wait_for_replication(
        client,
        tokens,
        repo,
        branch,
        workspace,
        comment_path,
        timeout,
    )
    .await
}

/// Verify that a specific relation exists on all nodes
pub async fn verify_relation_exists_on_all_nodes(
    client: &RestClient,
    tokens: &[String],
    repo: &str,
    branch: &str,
    workspace: &str,
    source_path: &str,
    target_path: &str,
    relation_type: &str,
    timeout: Duration,
) -> Result<()> {
    let start = Instant::now();

    // Resolve target_path to target_id (relations store IDs, not paths)
    // We only need to resolve once since the ID is consistent across nodes
    let target_node = client
        .get_node(
            &client.base_urls[0],
            &tokens[0],
            repo,
            branch,
            workspace,
            target_path,
        )
        .await
        .context("Failed to get target node")?;

    let target_id = target_node
        .as_ref()
        .and_then(|n| n["id"].as_str())
        .ok_or_else(|| anyhow::anyhow!("Target node '{}' not found", target_path))?;

    loop {
        if start.elapsed() > timeout {
            anyhow::bail!(
                "Relation from '{}' to '{}' (type: '{}') did not replicate to all nodes within {:?}",
                source_path,
                target_path,
                relation_type,
                timeout
            );
        }

        let mut all_have_relation = true;

        for (idx, (url, token)) in client.base_urls.iter().zip(tokens.iter()).enumerate() {
            let relations = client
                .get_relations(url, token, repo, branch, workspace, source_path)
                .await
                .with_context(|| format!("Failed to get relations from node{}", idx + 1))?;

            // Check if the specific relation exists
            // Note: API returns "target" (ID) and "relation_type" (snake_case), not "targetPath" or "relationType"
            let has_relation = relations.iter().any(|rel| {
                rel["target"].as_str() == Some(target_id)
                    && rel["relation_type"].as_str() == Some(relation_type)
            });

            if !has_relation {
                all_have_relation = false;
                break;
            }
        }

        if all_have_relation {
            return Ok(());
        }

        sleep(Duration::from_millis(100)).await;
    }
}

/// Verify that a relation has been deleted on all nodes
pub async fn verify_relation_deleted_on_all_nodes(
    client: &RestClient,
    tokens: &[String],
    repo: &str,
    branch: &str,
    workspace: &str,
    source_path: &str,
    target_path: &str,
    relation_type: &str,
    timeout: Duration,
) -> Result<()> {
    let start = Instant::now();

    // Resolve target_path to target_id (relations store IDs, not paths)
    // We only need to resolve once since the ID is consistent across nodes
    let target_node = client
        .get_node(
            &client.base_urls[0],
            &tokens[0],
            repo,
            branch,
            workspace,
            target_path,
        )
        .await
        .context("Failed to get target node")?;

    let target_id = target_node
        .as_ref()
        .and_then(|n| n["id"].as_str())
        .ok_or_else(|| anyhow::anyhow!("Target node '{}' not found", target_path))?;

    loop {
        if start.elapsed() > timeout {
            anyhow::bail!(
                "Relation from '{}' to '{}' (type: '{}') was not deleted on all nodes within {:?}",
                source_path,
                target_path,
                relation_type,
                timeout
            );
        }

        let mut all_deleted = true;

        for (idx, (url, token)) in client.base_urls.iter().zip(tokens.iter()).enumerate() {
            let relations = client
                .get_relations(url, token, repo, branch, workspace, source_path)
                .await
                .with_context(|| format!("Failed to get relations from node{}", idx + 1))?;

            // Check if the specific relation still exists
            // Note: API returns "target" (ID) and "relation_type" (snake_case), not "targetPath" or "relationType"
            let still_has_relation = relations.iter().any(|rel| {
                rel["target"].as_str() == Some(target_id)
                    && rel["relation_type"].as_str() == Some(relation_type)
            });

            if still_has_relation {
                all_deleted = false;
                break;
            }
        }

        if all_deleted {
            return Ok(());
        }

        sleep(Duration::from_millis(100)).await;
    }
}

/// Debug helper: dump children order from all nodes
pub fn dump_children_order(children_per_node: &[Vec<Value>], labels: &[&str]) {
    println!("\nChildren order on each node:");
    for (idx, (children, label)) in children_per_node.iter().zip(labels.iter()).enumerate() {
        println!("\n{}:", label);
        for (i, child) in children.iter().enumerate() {
            let id = child["id"].as_str().unwrap_or("unknown");
            let name = child["name"].as_str().unwrap_or("unknown");
            println!("  [{}] id={}, name={}", i, id, name);
        }
    }
    println!();
}
