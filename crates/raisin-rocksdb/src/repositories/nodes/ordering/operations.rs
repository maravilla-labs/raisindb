//! Child ordering operations
//!
//! This module provides the public API for reordering children:
//! - reorder_child: Move child to a specific position
//! - move_child_before: Move child before another child
//! - move_child_after: Move child after another child

use super::super::NodeRepositoryImpl;
use raisin_error::Result;

impl NodeRepositoryImpl {
    /// Reorder a child to a new position
    ///
    /// This operation moves a child to a specific numeric position in the parent's
    /// child list using fractional indexing.
    pub(in crate::repositories::nodes) async fn reorder_child_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        parent_path: &str,
        child_name: &str,
        new_position: usize,
        message: Option<&str>,
        actor: Option<&str>,
    ) -> Result<()> {
        // 1. Get parent_id for locking and index operations
        // Special case: root-level children (parent_path="/") use "/" as parent_id
        // This matches the logic in create_impl where root-level nodes are indexed with parent_id="/"
        let parent_id = if parent_path == "/" {
            "/".to_string()
        } else {
            // For non-root parents, get parent node to find its ID
            self.get_by_path_impl(tenant_id, repo_id, branch, workspace, parent_path, None)
                .await?
                .ok_or_else(|| raisin_error::Error::NotFound("Parent node not found".to_string()))?
                .id
        };

        // 2. Acquire ordering lock for this parent to prevent concurrent modifications
        let lock_mutex = self
            .acquire_ordering_lock(tenant_id, repo_id, branch, workspace, &parent_id)
            .await;
        let _lock = lock_mutex.lock().await;

        // 3. Find target child_id by name (efficient - reads name from ordered index value)
        let target_child_id = self
            .find_child_id_by_name(
                tenant_id, repo_id, branch, workspace, &parent_id, child_name,
            )?
            .ok_or_else(|| {
                raisin_error::Error::NotFound(format!("Child '{}' not found", child_name))
            })?;

        // 4. Get ordered child IDs (lightweight - no full node objects)
        // Use None for max_revision since reorder operations always work on current HEAD
        let child_ids = self
            .get_ordered_child_ids(tenant_id, repo_id, branch, workspace, &parent_id, None)
            .await?;

        // 5. Calculate position bounds (before and after)
        let before_child_id = if new_position > 0 {
            child_ids.get(new_position - 1).cloned()
        } else {
            None
        };

        let after_child_id = child_ids
            .get(new_position)
            .filter(|id| *id != &target_child_id)
            .cloned();

        // 6. Get labels for neighbors (EFFICIENT - only 2 queries!)
        let (before_label, after_label) = self.get_adjacent_labels(
            tenant_id,
            repo_id,
            branch,
            workspace,
            &parent_id,
            before_child_id.as_deref(),
            after_child_id.as_deref(),
        )?;

        // 7. Calculate new order label using fractional indexing
        // Extract fractional parts from labels (strip ::HLC suffix)
        let before_fractional = before_label
            .as_ref()
            .map(|l| crate::fractional_index::extract_fractional(l));
        let after_fractional = after_label
            .as_ref()
            .map(|l| crate::fractional_index::extract_fractional(l));

        let new_label = crate::fractional_index::between(before_fractional, after_fractional)?;

        // 8. Use shared reorder logic
        let default_message = format!("Reorder {} to position {}", child_name, new_position);
        let final_message = message.unwrap_or(&default_message);

        self.reorder_child_with_label(
            tenant_id,
            repo_id,
            branch,
            workspace,
            &parent_id,
            &target_child_id,
            child_name,
            new_label,
            Some(final_message),
            actor,
        )
        .await
    }

    /// Move child before another child
    ///
    /// This operation repositions a child to be immediately before a specified sibling
    /// using fractional indexing.
    pub(in crate::repositories::nodes) async fn move_child_before_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        parent_path: &str,
        child_name: &str,
        before_child_name: &str,
        message: Option<&str>,
        actor: Option<&str>,
    ) -> Result<()> {
        // 0. Early validation: self-move is a no-op
        if child_name == before_child_name {
            return Ok(()); // Already in place, nothing to do
        }

        // 1. Get parent_id for locking and index operations
        // Special case: root-level children (parent_path="/") use "/" as parent_id
        // This matches the logic in create_impl where root-level nodes are indexed with parent_id="/"
        let parent_id = if parent_path == "/" {
            "/".to_string()
        } else {
            // For non-root parents, get parent node to find its ID
            self.get_by_path_impl(tenant_id, repo_id, branch, workspace, parent_path, None)
                .await?
                .ok_or_else(|| raisin_error::Error::NotFound("Parent node not found".to_string()))?
                .id
        };

        // 2. Acquire ordering lock for this parent to prevent concurrent modifications
        let lock_mutex = self
            .acquire_ordering_lock(tenant_id, repo_id, branch, workspace, &parent_id)
            .await;
        let _lock = lock_mutex.lock().await;

        // 3. Find both child IDs by name (efficient - reads names from ordered index values)
        let target_child_id = self
            .find_child_id_by_name(
                tenant_id, repo_id, branch, workspace, &parent_id, child_name,
            )?
            .ok_or_else(|| {
                raisin_error::Error::NotFound(format!("Child '{}' not found", child_name))
            })?;

        let before_child_id = self
            .find_child_id_by_name(
                tenant_id,
                repo_id,
                branch,
                workspace,
                &parent_id,
                before_child_name,
            )?
            .ok_or_else(|| {
                raisin_error::Error::NotFound(format!(
                    "Before child '{}' not found",
                    before_child_name
                ))
            })?;

        // 4. Get ordered child IDs
        // Use None for max_revision since move operations always work on current HEAD
        let child_ids = self
            .get_ordered_child_ids(tenant_id, repo_id, branch, workspace, &parent_id, None)
            .await?;

        // 5. Find position of before_child and get the one before it
        let before_pos = child_ids
            .iter()
            .position(|id| id == &before_child_id)
            .ok_or_else(|| {
                raisin_error::Error::NotFound("Before child position not found".to_string())
            })?;

        let prev_child_id = if before_pos > 0 {
            Some(&child_ids[before_pos - 1])
        } else {
            None
        };

        // 5b. No-op detection: if source is already immediately before target, skip
        if prev_child_id == Some(&target_child_id) {
            return Ok(()); // Already in correct position
        }

        // 6. Get labels for prev and before (EFFICIENT - only 2 queries!)
        let (prev_label, before_label) = self.get_adjacent_labels(
            tenant_id,
            repo_id,
            branch,
            workspace,
            &parent_id,
            prev_child_id.map(|s| s.as_str()),
            Some(&before_child_id),
        )?;

        // 7. Calculate new order label
        // Extract fractional parts from labels (strip ::HLC suffix)
        let prev_fractional = prev_label
            .as_ref()
            .map(|l| crate::fractional_index::extract_fractional(l));
        let before_fractional = before_label
            .as_ref()
            .map(|l| crate::fractional_index::extract_fractional(l));

        let new_label = crate::fractional_index::between(prev_fractional, before_fractional)?;

        // 8. Use shared reorder logic
        let default_message = format!("Move {} before {}", child_name, before_child_name);
        let final_message = message.unwrap_or(&default_message);

        self.reorder_child_with_label(
            tenant_id,
            repo_id,
            branch,
            workspace,
            &parent_id,
            &target_child_id,
            child_name,
            new_label,
            Some(final_message),
            actor,
        )
        .await
    }

    /// Move child after another child
    ///
    /// This operation repositions a child to be immediately after a specified sibling
    /// using fractional indexing.
    pub(in crate::repositories::nodes) async fn move_child_after_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        parent_path: &str,
        child_name: &str,
        after_child_name: &str,
        message: Option<&str>,
        actor: Option<&str>,
    ) -> Result<()> {
        // 0. Early validation: self-move is a no-op
        if child_name == after_child_name {
            return Ok(()); // Already in place, nothing to do
        }

        // 1. Get parent_id for locking and index operations
        // Special case: root-level children (parent_path="/") use "/" as parent_id
        // This matches the logic in create_impl where root-level nodes are indexed with parent_id="/"
        let parent_id = if parent_path == "/" {
            "/".to_string()
        } else {
            // For non-root parents, get parent node to find its ID
            self.get_by_path_impl(tenant_id, repo_id, branch, workspace, parent_path, None)
                .await?
                .ok_or_else(|| raisin_error::Error::NotFound("Parent node not found".to_string()))?
                .id
        };

        // 2. Acquire ordering lock for this parent to prevent concurrent modifications
        let lock_mutex = self
            .acquire_ordering_lock(tenant_id, repo_id, branch, workspace, &parent_id)
            .await;
        let _lock = lock_mutex.lock().await;

        // 3. Find both child IDs by name (efficient - reads names from ordered index values)
        let target_child_id = self
            .find_child_id_by_name(
                tenant_id, repo_id, branch, workspace, &parent_id, child_name,
            )?
            .ok_or_else(|| {
                raisin_error::Error::NotFound(format!("Child '{}' not found", child_name))
            })?;

        let after_child_id = self
            .find_child_id_by_name(
                tenant_id,
                repo_id,
                branch,
                workspace,
                &parent_id,
                after_child_name,
            )?
            .ok_or_else(|| {
                raisin_error::Error::NotFound(format!(
                    "After child '{}' not found",
                    after_child_name
                ))
            })?;

        // 4. Get ordered child IDs for position calculation
        // Use None for max_revision since move operations always work on current HEAD
        let child_ids = self
            .get_ordered_child_ids(tenant_id, repo_id, branch, workspace, &parent_id, None)
            .await?;

        // 5. Find position of after_child and get the one after it
        let after_pos = child_ids
            .iter()
            .position(|id| id == &after_child_id)
            .ok_or_else(|| {
                raisin_error::Error::NotFound("After child position not found".to_string())
            })?;

        let next_child_id = child_ids.get(after_pos + 1).map(|s| s.as_str());

        // 5b. No-op detection: if source is already immediately after target, skip
        if next_child_id == Some(target_child_id.as_str()) {
            return Ok(()); // Already in correct position
        }

        // 6. Get labels for after and next (EFFICIENT - only 2 queries!)
        let (after_label, next_label) = self.get_adjacent_labels(
            tenant_id,
            repo_id,
            branch,
            workspace,
            &parent_id,
            Some(&after_child_id),
            next_child_id,
        )?;

        // 7. Calculate new order label
        // Extract fractional parts from labels (strip ::HLC suffix)
        let after_fractional = after_label
            .as_ref()
            .map(|l| crate::fractional_index::extract_fractional(l));
        let next_fractional = next_label
            .as_ref()
            .map(|l| crate::fractional_index::extract_fractional(l));

        let new_label = crate::fractional_index::between(after_fractional, next_fractional)?;

        // 8. Use shared reorder logic
        let default_message = format!("Move {} after {}", child_name, after_child_name);
        let final_message = message.unwrap_or(&default_message);

        self.reorder_child_with_label(
            tenant_id,
            repo_id,
            branch,
            workspace,
            &parent_id,
            &target_child_id,
            child_name,
            new_label,
            Some(final_message),
            actor,
        )
        .await
    }
}
