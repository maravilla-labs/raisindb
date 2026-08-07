//! Commit phase logic for RocksDB transactions
//!
//! This module contains all the logic for committing transactions, including:
//! - Metadata extraction and batch handling
//! - Revision metadata creation
//! - Branch HEAD updates
//! - Snapshot job enqueueing
//! - Event emission
//! - Replication peer synchronization

mod events;
mod extract;
mod replication;
mod revision;

use super::RocksDBTransaction;
use raisin_error::Result;
use tracing::debug;

/// Main commit implementation for RocksDBTransaction
///
/// This is the actual commit logic called by the Transaction trait implementation in core.rs.
/// All commit phases are orchestrated here for better separation of concerns.
///
/// # Commit Phases
///
/// 1. **Conflict Check**: Verify no conflicts with other transactions
/// 2. **Metadata Extraction**: Extract tenant, repo, branch, actor, message
/// 3. **Data Collection**: Extract changed nodes and translations
/// 4. **RevisionMeta Creation**: Create revision metadata for the commit
/// 5. **Atomic Write**: Write everything to RocksDB in a single batch
/// 6. **Replication**: Capture and push operations to peers
/// 7. **Background Jobs**: Enqueue snapshot creation job
/// 8. **Event Emission**: Emit NodeEvent for each changed node
pub(super) async fn commit_impl(tx: &RocksDBTransaction) -> Result<()> {
    // Validate auth context is set (required for RLS enforcement)
    {
        let metadata = tx
            .metadata
            .lock()
            .map_err(|e| raisin_error::Error::storage(format!("Failed to lock metadata: {}", e)))?;

        if metadata.auth_context.is_none() {
            return Err(raisin_error::Error::invalid_state(
                "AuthContext required for transaction commit. Call tx.set_auth_context(auth) before commit. \
                For system operations, use AuthContext::system().",
            ));
        }
    }

    // Check for conflicts before committing
    tx.check_conflicts()?;

    // PHASE 1: Extract metadata and changed data
    let commit_meta = tx.extract_commit_metadata()?;
    let tenant_id = commit_meta.tenant_id;
    let repo_id = commit_meta.repo_id;
    let branch = commit_meta.branch;
    let max_revision = commit_meta.transaction_revision;
    let actor = commit_meta.actor;
    let message = commit_meta.message;
    let is_system = commit_meta.is_system;

    tracing::warn!(
        "COMMIT DEBUG: max_revision={:?}, branch={:?}, actor={:?}, message={:?}, is_system={}",
        max_revision,
        branch,
        actor.as_deref(),
        message.as_deref(),
        is_system
    );

    let mut batch_to_write = tx.extract_batch()?;

    // PHASE 2: Extract changed nodes and translations for async snapshot creation
    let changed_nodes = tx.extract_changed_nodes()?;

    // Diagnostic for runaway mount-node commit loops: name the writer whenever a
    // VirtualMount node is part of a commit. One line per COMMIT (a transaction
    // batches its writes and commits once), aggregating the affected mounts.
    let mount_changes: Vec<String> = changed_nodes
        .iter()
        .filter(|(_, change)| change.node_type.as_deref() == Some("raisin:VirtualMount"))
        .map(|(node_id, change)| format!("{node_id}:{:?}", change.operation))
        .collect();
    if !mount_changes.is_empty() {
        tracing::warn!(
            mounts = ?mount_changes,
            branch = ?branch.as_deref(),
            actor = ?actor.as_deref(),
            message = ?message.as_deref(),
            is_system,
            "VirtualMount node committed"
        );
    }
    let changed_translations = tx.extract_changed_translations()?;

    tracing::debug!(
        "Transaction has {} changed nodes and {} changed translations for async snapshot creation",
        changed_nodes.len(),
        changed_translations.len()
    );

    // PHASE 3-4: Create RevisionMeta and update branch HEAD in the batch
    let mut created_revision_meta: Option<raisin_storage::RevisionMeta> = None;
    let mut branch_updates = Vec::new();

    // ALWAYS update branch head when there's a revision (even for relation-only changes)
    // This ensures replicated data becomes visible immediately
    if let (Some(branch_name), Some(new_revision)) = (branch.as_deref(), max_revision.as_ref()) {
        debug!(branch = %branch_name, revision = %new_revision, "Updating branch head");

        // Create RevisionMeta with defaults if actor/message not set
        let actor_str = actor.as_deref().map(|s| s.as_str()).unwrap_or("system");
        let message_str = message
            .as_deref()
            .map(|s| s.as_str())
            .unwrap_or("auto-commit");

        debug!(
            actor = %actor_str,
            message = %message_str,
            "Creating RevisionMeta"
        );
        // Build NodeChangeInfo list
        let changed_node_infos = tx.build_node_change_infos(&changed_nodes, &changed_translations);

        // Create and add RevisionMeta to batch
        let revision_meta = tx
            .create_revision_meta(
                &mut batch_to_write,
                &tenant_id,
                &repo_id,
                branch_name,
                new_revision,
                actor_str,
                message_str,
                is_system,
                changed_node_infos,
            )
            .await?;

        created_revision_meta = Some(revision_meta);

        // Always update branch HEAD when we have a revision
        let updated_branch = tx
            .update_branch_head(
                &mut batch_to_write,
                &tenant_id,
                &repo_id,
                branch_name,
                new_revision,
            )
            .await?;

        debug!("Adding branch update to branch_updates vec");
        branch_updates.push((tenant_id.clone(), repo_id.clone(), updated_branch));
    } else {
        debug!(
            branch = ?branch,
            max_revision = ?max_revision,
            "No branch update (missing branch or revision)"
        );
    }

    // PHASE 5: Write EVERYTHING atomically
    tracing::debug!(
        "Writing atomic batch with {} operations",
        "all changes" // WriteBatch doesn't expose count easily
    );

    // Run the atomic RocksDB write on Tokio's blocking pool. `rocksdb::DB::write`
    // is a synchronous FFI call that can park for a long time on RocksDB's internal
    // write-stall/write-stop backpressure. Executing it directly on a job-pool
    // worker thread means a burst of concurrent commits (e.g. many package installs
    // at once) can park every worker in this same blocking call simultaneously and
    // wedge the whole pool — the deadlock that froze the job queue. Offloading to the
    // elastic blocking pool keeps the async worker threads free to make progress;
    // the commit is still awaited here, so ordering and consistency are unchanged.
    let db = tx.db.clone();
    tokio::task::spawn_blocking(move || db.write(batch_to_write))
        .await
        .map_err(|e| {
            raisin_error::Error::storage(format!("Commit write task failed to join: {}", e))
        })?
        .map_err(|e| raisin_error::Error::storage(format!("Transaction commit failed: {}", e)))?;

    tracing::debug!("Atomic commit successful");

    // The created rows are durable now, so the committed-storage existence
    // check protects path uniqueness from here on; release the in-flight
    // CREATE path reservations taken by add_node.
    tx.release_create_path_reservations();

    // PHASE 5.3: Capture RevisionMeta and branch update operations for replication
    tx.capture_metadata_operations(
        (*tenant_id).clone(),
        (*repo_id).clone(),
        created_revision_meta,
        branch_updates
            .into_iter()
            .map(|(t, r, b)| ((*t).clone(), (*r).clone(), b))
            .collect(),
        &actor.as_ref().map(|a| (**a).clone()),
        &message.as_ref().map(|m| (**m).clone()),
        is_system,
    )
    .await;

    // PHASE 5.4: Capture operations for replication using ChangeTracker
    if let (Some(branch_name), Some(new_revision)) = (branch.as_deref(), max_revision.as_ref()) {
        let actor_str = actor
            .as_deref()
            .map(|s| s.to_string())
            .unwrap_or_else(|| "system".to_string());
        let message_str = message
            .as_deref()
            .map(|s| s.to_string())
            .unwrap_or_else(|| "relation update".to_string());

        tx.capture_tracked_changes(
            (*tenant_id).clone(),
            (*repo_id).clone(),
            branch_name.to_string(),
            Some(*new_revision),
            actor_str,
            message_str,
            is_system,
        )
        .await?;
    }

    // PHASE 5.5: Enqueue async snapshot creation job
    if let (Some(branch_name), Some(new_revision)) = (branch.as_deref(), max_revision.as_ref()) {
        tx.enqueue_snapshot_job(
            &tenant_id,
            &repo_id,
            branch_name,
            new_revision,
            &changed_nodes,
            &changed_translations,
        )
        .await?;
    }

    // PHASE 6: Emit NodeEvent for each changed node
    if let Some(branch_name) = branch.as_deref() {
        tx.emit_node_events(&tenant_id, &repo_id, branch_name, &changed_nodes)
            .await;
    }

    // PHASE 5.6: Push captured operations to replication peers
    tx.push_to_replication_peers().await?;

    Ok(())
}
