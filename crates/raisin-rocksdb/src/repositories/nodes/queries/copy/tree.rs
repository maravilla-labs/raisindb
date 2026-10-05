//! Tree copy operation.
//!
//! Recursively copies a node and all its descendants to a new location.
//! Generates new IDs for all copied nodes while preserving the tree structure,
//! fractional index ordering, and translations (node-level and block-level).

use super::super::super::NodeRepositoryImpl;
use crate::translation_write::OverlayTarget;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_models::nodes::Node;
use raisin_models::translations::TranslationMeta;
use raisin_models::tree::ChangeOperation;
use raisin_storage::{BranchRepository, BranchScope, NodeRepository, RevisionRepository};
use rocksdb::WriteBatch;
use std::collections::HashMap;

impl NodeRepositoryImpl {
    /// Copy node tree recursively
    ///
    /// Recursively copies a node and all its descendants to a new location.
    /// Generates new IDs for all copied nodes while preserving the tree structure
    /// and fractional index ordering.
    ///
    /// # Arguments
    /// * `source_path` - Path to the node to copy
    /// * `target_parent` - Path to the parent where the copy will be placed
    /// * `new_name` - Optional new name for the root of the copied tree
    ///
    /// # Returns
    /// The root node of the copied tree
    pub(in crate::repositories::nodes) async fn copy_node_tree_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        source_path: &str,
        target_parent: &str,
        new_name: Option<&str>,
        operation_meta: Option<raisin_models::operations::OperationMeta>,
    ) -> Result<Node> {
        // VALIDATION 1: Cannot copy root node
        self.validate_not_root_node(source_path)?;

        // VALIDATION 2: Source must exist
        let source = self
            .get_by_path_impl(tenant_id, repo_id, branch, workspace, source_path, None)
            .await?
            .ok_or_else(|| raisin_error::Error::NotFound("Source node not found".to_string()))?;

        // VALIDATION 3: Check for circular reference (cannot copy into own descendant)
        // target_parent cannot be equal to or start with source_path
        if target_parent == source_path || target_parent.starts_with(&format!("{}/", source_path)) {
            return Err(raisin_error::Error::Validation(format!(
                "Cannot copy '{}' into its own descendant '{}'",
                source_path, target_parent
            )));
        }

        // VALIDATION 4 (MINIMAL): Check target doesn't exist
        let name = new_name.unwrap_or(&source.name);
        // At the workspace root `target_parent` is already "/", so joining
        // naively would produce "//name".
        let new_path = if target_parent == "/" {
            format!("/{}", name)
        } else {
            format!("{}/{}", target_parent, name)
        };

        // Reserve the destination ROOT before the existence check below
        // (reserve-then-check, same registry contract as create). The guard
        // holds the reservation until this function's single WriteBatch is
        // durable (or it errors / is cancelled).
        //
        // Why the ROOT only, not every descendant destination path:
        // - All copied descendants land strictly under `new_path`, and the
        //   whole tree is committed in ONE atomic WriteBatch — descendant
        //   paths only become visible together with the reserved root, so
        //   reserving the root owns the destination prefix for the duration
        //   of the copy.
        // - A competing creator cannot legitimately produce a descendant path
        //   first: parent-validated creates fail with NotFound while the root
        //   doesn't exist yet, and a competing copy/create AT the root
        //   conflicts on this reservation.
        // - Per-descendant reservation would take the registry mutex O(tree)
        //   times for keys nobody else can contend, for no added safety.
        //   (A parentless transactional `add_node` writing a descendant path
        //   blind is already an orphan-tolerated escape hatch today; the
        //   registry is not the layer that makes orphans impossible.)
        let mut reservation_guard = crate::repositories::nodes::PathReservationGuard::new(self);
        reservation_guard.reserve(tenant_id, repo_id, branch, workspace, &new_path)?;

        if self
            .get_by_path_impl(tenant_id, repo_id, branch, workspace, &new_path, None)
            .await?
            .is_some()
        {
            return Err(raisin_error::Error::Conflict(format!(
                "Target path '{}' already exists",
                new_path
            )));
        }

        // VALIDATION 5 (MINIMAL): Check parent allows child type
        // Only validate the root of the copied tree - assume source tree is internally valid
        //
        // `None` means the WORKSPACE ROOT, which is a legitimate destination
        // that stores no node of its own — there is then no parent schema to
        // consult, so the child check is skipped (see validate_parent_exists_opt).
        let target_parent_node = self
            .validate_parent_exists_opt(tenant_id, repo_id, branch, workspace, target_parent)
            .await?;

        if let Some(parent) = &target_parent_node {
            self.validate_parent_allows_child(
                BranchScope::new(tenant_id, repo_id, branch),
                &parent.node_type,
                &source.node_type,
            )
            .await?;
        }

        // Localized node name uniqueness of the copied root at its
        // destination, carrying the source's overlays (plan Phase 12; a no-op
        // unless the repository enforces it).
        // Checked now, and again at the commit step under the branch lock
        // (`localized_name::unique::deferred`).
        let name_check = {
            let names =
                crate::localized_name::keys::NameScope::new(tenant_id, repo_id, branch, workspace);
            let mut copy = source.clone();
            copy.id = String::new();
            copy.path = new_path.clone();
            copy.name = name.to_string();
            crate::localized_name::unique::NameCheck::staged(
                &self.db,
                names,
                &copy,
                Some(target_parent_node.as_ref().map_or("/", |p| p.id.as_str())),
                &crate::mvcc_read::NEWEST,
                crate::localized_name::unique::source_overrides(&self.db, names, &source.id)?,
            )?
        };

        // Overlays are read as of the branch HEAD (the one reader's bound),
        // never a version written above it.
        let source_head = self
            .branch_repo
            .get_branch(tenant_id, repo_id, branch)
            .await?
            .map(|b| b.head);

        // STEP 1: Allocate SINGLE revision for entire tree copy operation
        let revision = self.revision_repo.allocate_revision();

        tracing::info!(
            "copy_node_tree_impl: source_path={}, target={}, revision={}, using atomic single-revision approach",
            source_path,
            new_path,
            revision
        );

        // STEP 2: Use prefix scan to collect all descendants (no recursion!)
        let descendants = self.scan_descendants_ordered_impl(
            tenant_id, repo_id, branch, workspace, &source.id, None,
        )?;

        tracing::debug!(
            "copy_node_tree_impl: collected {} nodes to copy",
            descendants.len()
        );

        // STEP 3: Build ID mapping and prepare nodes iteratively
        let mut id_mapping: HashMap<String, String> = HashMap::new();
        let mut path_mapping: HashMap<String, String> = HashMap::new();
        let mut order_label_mapping: HashMap<String, String> = HashMap::new();
        let mut path_to_old_id: HashMap<String, String> = HashMap::new();

        // Build path_to_old_id mapping for later parent ID lookups
        for (node, _) in &descendants {
            path_to_old_id.insert(node.path.clone(), node.id.clone());
        }

        // Get fractional index labels for all nodes to preserve order
        for (node, _depth) in &descendants {
            if let Some(parent_name) = &node.parent {
                if parent_name != "/" {
                    let parent_path = node.path.rsplit_once('/').map(|x| x.0).unwrap_or("/");

                    if let Some(parent_node) = self
                        .get_by_path_impl(tenant_id, repo_id, branch, workspace, parent_path, None)
                        .await?
                    {
                        if let Some(label) = self.get_order_label_for_child(
                            tenant_id,
                            repo_id,
                            branch,
                            workspace,
                            &parent_node.id,
                            &node.id,
                        )? {
                            order_label_mapping.insert(node.id.clone(), label);
                        }
                    }
                }
            }
        }

        // STEP 4: Create WriteBatch for atomic operation
        let mut batch = WriteBatch::default();
        let mut copied_node_ids = Vec::new();
        let mut translation_change_infos: Vec<raisin_storage::NodeChangeInfo> = Vec::new();
        let now = chrono::Utc::now();

        // Translation versions the copy writes, captured after the nodes.
        let mut translation_ops: Vec<raisin_replication::OpType> = Vec::new();

        let (translation_actor, translation_message, translation_is_system) =
            if let Some(meta) = operation_meta.as_ref() {
                (meta.actor.clone(), meta.message.clone(), meta.is_system)
            } else {
                (
                    "system".to_string(),
                    format!("Copy tree {} -> {}", source_path, new_path),
                    true,
                )
            };

        // STEP 5: Process nodes in breadth-first order (parents before children)
        // Collect operation capture data for replication
        let mut nodes_for_replication: Vec<(Node, Option<String>, Option<String>)> = Vec::new();

        for (source_node, depth) in descendants {
            let new_id = nanoid::nanoid!();
            id_mapping.insert(source_node.id.clone(), new_id.clone());
            copied_node_ids.push(new_id.clone());

            // Calculate new path based on depth
            let new_node_path = if depth == 0 {
                new_path.clone()
            } else {
                let relative_path = source_node
                    .path
                    .strip_prefix(&format!("{}/", source.path))
                    .unwrap_or(&source_node.path);
                format!("{}/{}", new_path, relative_path)
            };

            path_mapping.insert(source_node.path.clone(), new_node_path.clone());

            // Construct new node
            let mut new_node = source_node.clone();
            new_node.id = new_id.clone();
            new_node.path = new_node_path.clone();
            new_node.name = if depth == 0 {
                name.to_string()
            } else {
                source_node.name.clone()
            };
            new_node.created_at = Some(now);
            new_node.updated_at = Some(now);
            new_node.has_children = None; // Never store computed field
            new_node.children = vec![]; // Clear children list

            // Update parent reference
            new_node.parent = Node::extract_parent_name_from_path(&new_node_path);

            // Every node in the tree gets a NEW id, so every one of them must
            // get its own secrets — same reasoning as the single-node copy; see
            // `revault.rs`.
            let minted_secrets = self
                .revault_copied_node(
                    tenant_id,
                    repo_id,
                    branch,
                    &source_node.id,
                    &mut new_node,
                    &translation_actor,
                )
                .await?;
            self.capture_secret_versions(
                tenant_id,
                repo_id,
                branch,
                &translation_actor,
                &minted_secrets,
            )
            .await;

            // Determine the NEW parent ID for ORDERED_CHILDREN index
            let new_parent_id: Option<String> = if depth == 0 {
                // Root-level nodes are indexed with parent_id = "/" (listing.rs).
                Some(
                    target_parent_node
                        .as_ref()
                        .map_or_else(|| "/".to_string(), |p| p.id.clone()),
                )
            } else if let Some(_source_parent_name) = &source_node.parent {
                let source_parent_path = source_node
                    .path
                    .rsplit_once('/')
                    .map(|x| x.0)
                    .unwrap_or("/");
                if source_parent_path != "/" {
                    if let Some(old_parent_id) = path_to_old_id.get(source_parent_path) {
                        id_mapping.get(old_parent_id).cloned()
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            };

            // The order label. A DESCENDANT keeps its source label: its parent is
            // a fresh copy, so labels cannot collide and editorial order is
            // preserved. The copied ROOT is appended to the target parent like
            // any create, through the one append minter. It used to reuse the
            // source's label (landing at the source's position, possibly on a
            // label a target sibling already holds) or, under the workspace
            // root, pass a FULL label to `inc` — which fails on the `::HLC`
            // suffix and fell back to a duplicate `first()`.
            //
            // The label is the node's `order_key` (`Node.order_key ==
            // ORDERED_CHILDREN label`): the clone still carried the SOURCE's.
            let mut order_label_owned: Option<String> = None;
            if let Some(ref parent_id) = new_parent_id {
                let label = match order_label_mapping.get(&source_node.id) {
                    Some(existing) if depth > 0 => existing.clone(),
                    _ => self.next_append_label(
                        tenant_id, repo_id, branch, workspace, parent_id, &revision,
                    )?,
                };
                order_label_mapping.insert(source_node.id.clone(), label.clone());
                new_node.order_key = label.clone();
                order_label_owned = Some(label);
            }

            // Add node to batch with SAME revision and parent ID override
            self.add_node_to_batch_with_parent_id(
                &mut batch,
                &new_node,
                tenant_id,
                repo_id,
                branch,
                workspace,
                &revision,
                order_label_owned.as_deref(),
                new_parent_id.as_deref(),
                // A copy mints a fresh id: no prior version on the branch.
                crate::repositories::nodes::PropertyWrite::CREATE,
            )?;

            // Compound indexes are the one family the batch indexer cannot
            // write (they need an async NodeType load), so every write path
            // adds them itself — the copy never did, and a copied node was
            // invisible to every typed folder listing.
            self.add_compound_delta_to_batch(
                &mut batch,
                &crate::indexing::IndexCtx::new(tenant_id, repo_id, branch, workspace),
                crate::indexing::Baseline::NoPrior,
                &new_node,
                &revision,
            )
            .await?;

            // Copy latest node-level translations (if any)
            let node_translations = self.collect_node_translations_for_copy(
                tenant_id,
                repo_id,
                branch,
                workspace,
                &source_node.id,
                source_head.as_ref(),
            )?;

            let mut staged_overlays = crate::localized_name::sync::Overrides::new();
            for (locale, overlay, parent_translation_revision) in node_translations {
                staged_overlays.insert(locale.as_str().to_string(), Some(overlay.clone()));
                let target = OverlayTarget {
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    node_id: &new_id,
                    block_uuid: None,
                    locale: locale.as_str(),
                };
                let translation_meta = TranslationMeta {
                    locale: locale.clone(),
                    revision,
                    parent_revision: parent_translation_revision,
                    timestamp: now,
                    actor: translation_actor.clone(),
                    message: translation_message.clone(),
                    is_system: translation_is_system,
                };
                translation_ops.push(self.stage_copied_translation(
                    &mut batch,
                    &target,
                    Some(&overlay),
                    &translation_meta,
                )?);

                translation_change_infos.push(raisin_storage::NodeChangeInfo {
                    node_id: new_id.clone(),
                    workspace: workspace.to_string(),
                    operation: ChangeOperation::Added,
                    translation_locale: Some(locale.as_str().to_string()),
                });
            }

            // The copy and its overlays share this unreadable batch: sync the
            // localized name index against both (plan Phase 12).
            if !staged_overlays.is_empty() {
                crate::localized_name::sync::sync_node_final(
                    &self.db,
                    &mut batch,
                    crate::localized_name::keys::NameScope::new(
                        tenant_id, repo_id, branch, workspace,
                    ),
                    &new_node,
                    new_parent_id.as_deref(),
                    &revision,
                    &staged_overlays,
                )?;
            }

            // Copy block-level translations (if any)
            let block_translations = self.collect_block_translations_for_copy(
                tenant_id,
                repo_id,
                branch,
                workspace,
                &source_node.id,
                source_head.as_ref(),
            )?;

            for (block_uuid, locale, overlay, parent_revision) in block_translations {
                let target = OverlayTarget {
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    node_id: &new_id,
                    block_uuid: Some(&block_uuid),
                    locale: locale.as_str(),
                };
                let translation_meta = TranslationMeta {
                    locale: locale.clone(),
                    revision,
                    parent_revision,
                    timestamp: now,
                    actor: translation_actor.clone(),
                    message: translation_message.clone(),
                    is_system: translation_is_system,
                };
                translation_ops.push(self.stage_copied_translation(
                    &mut batch,
                    &target,
                    Some(&overlay),
                    &translation_meta,
                )?);

                translation_change_infos.push(raisin_storage::NodeChangeInfo {
                    node_id: new_id.clone(),
                    workspace: workspace.to_string(),
                    operation: ChangeOperation::Added,
                    translation_locale: Some(format!("{}::{}", locale.as_str(), block_uuid)),
                });
            }

            if let (Some(parent_id), Some(label)) =
                (new_parent_id.as_ref(), order_label_owned.as_ref())
            {
                let cf_ordered = cf_handle(&self.db, cf::ORDERED_CHILDREN)?;
                let metadata_key =
                    keys::last_child_metadata_key(tenant_id, repo_id, branch, workspace, parent_id);
                batch.put_cf(cf_ordered, metadata_key, label.as_bytes());
            }

            // Collect node information for operation capture
            nodes_for_replication.push((new_node, new_parent_id, order_label_owned));
        }

        // STEP 6: Atomic commit - all nodes created in single WriteBatch, as one
        // node commit step (plan Phase 7b): creates re-validate nothing, but
        // every copied node is locked like any write of it.
        let mut commit = crate::indexing::NodeCommit::new(tenant_id, repo_id, branch);
        for node_id in &copied_node_ids {
            commit.touch(node_id);
        }
        commit.check_name(name_check);
        commit.write(&self.db, batch).await.map_err(|e| match e {
            // A localized name refused under the branch lock stays a
            // conflict.
            raisin_error::Error::Conflict(_) => e,
            e => raisin_error::Error::storage(format!("Atomic copy_tree failed: {}", e)),
        })?;

        // Destination tree is durable — release the root reservation.
        drop(reservation_guard);

        tracing::info!(
            "copy_node_tree_impl: successfully copied {} nodes with single revision {}",
            copied_node_ids.len(),
            revision
        );

        // STEP 7: Index all node changes for this revision
        for node_id in &copied_node_ids {
            self.revision_repo
                .index_node_change(tenant_id, repo_id, &revision, node_id)
                .await?;
        }

        // STEP 7.5: Capture one ApplyRevision snapshot for replication
        self.capture_tree_copy_operations(
            tenant_id,
            repo_id,
            branch,
            workspace,
            &operation_meta,
            &nodes_for_replication,
            revision,
        )
        .await;

        // STEP 7.6: The copied translation versions, after the nodes (same lane).
        let translation_actor_for_capture = operation_meta
            .as_ref()
            .map(|m| m.actor.clone())
            .unwrap_or_else(|| "system".to_string());
        for op_type in translation_ops {
            let _ = self
                .operation_capture
                .capture_operation_with_revision(
                    tenant_id.to_string(),
                    repo_id.to_string(),
                    branch.to_string(),
                    op_type,
                    translation_actor_for_capture.clone(),
                    None,
                    false,
                    Some(revision),
                )
                .await;
        }

        // STEP 8: Store operation metadata with ALL copied node IDs
        if let Some(mut op_meta) = operation_meta {
            op_meta.revision = revision;
            op_meta.node_id = id_mapping
                .get(&source.id)
                .ok_or_else(|| {
                    raisin_error::Error::internal("Source node ID not found in mapping after copy")
                })?
                .clone();

            // Create NodeChangeInfo for each copied node
            let mut changed_nodes: Vec<raisin_storage::NodeChangeInfo> = copied_node_ids
                .iter()
                .map(|node_id| raisin_storage::NodeChangeInfo {
                    node_id: node_id.clone(),
                    workspace: workspace.to_string(),
                    translation_locale: None,
                    operation: ChangeOperation::Added,
                })
                .collect();

            changed_nodes.extend(translation_change_infos.into_iter());

            let rev_meta = raisin_storage::RevisionMeta {
                revision,
                parent: op_meta.parent_revision,
                merge_parent: None,
                branch: branch.to_string(),
                timestamp: op_meta.timestamp,
                actor: op_meta.actor.clone(),
                message: op_meta.message.clone(),
                is_system: op_meta.is_system,
                changed_nodes,
                changed_node_types: Vec::new(),
                changed_archetypes: Vec::new(),
                changed_element_types: Vec::new(),
                operation: Some(op_meta),
            };

            self.revision_repo
                .store_revision_meta(tenant_id, repo_id, rev_meta)
                .await?;
        }

        // STEP 9: Update branch HEAD to the new revision
        self.branch_repo
            .update_head(tenant_id, repo_id, branch, revision)
            .await?;

        // STEP 10: Return the copied root node
        let root_new_id = id_mapping.get(&source.id).ok_or_else(|| {
            raisin_error::Error::internal("Source node ID not found in mapping after copy")
        })?;
        self.get_impl(tenant_id, repo_id, branch, workspace, root_new_id, false)
            .await?
            .ok_or_else(|| raisin_error::Error::storage("Failed to retrieve copied root node"))
    }

    /// Capture one ApplyRevision snapshot covering all copied nodes.
    ///
    /// Full node snapshots (same shape as transaction commits) instead of
    /// granular CreateNode ops, so peers apply identical state.
    #[allow(clippy::too_many_arguments)]
    async fn capture_tree_copy_operations(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        operation_meta: &Option<raisin_models::operations::OperationMeta>,
        nodes_for_replication: &[(Node, Option<String>, Option<String>)],
        revision: raisin_hlc::HLC,
    ) {
        use raisin_replication::operation::{ReplicatedNodeChange, ReplicatedNodeChangeKind};

        let actor = operation_meta
            .as_ref()
            .map(|m| m.actor.clone())
            .unwrap_or_else(|| "system".to_string());

        let node_changes = nodes_for_replication
            .iter()
            .map(|(node, parent_id, order_label)| {
                let mut node = node.clone();
                if node.workspace.is_none() {
                    node.workspace = Some(workspace.to_string());
                }
                ReplicatedNodeChange {
                    node,
                    parent_id: parent_id.clone(),
                    kind: ReplicatedNodeChangeKind::Upsert,
                    cf_order_key: order_label.clone().unwrap_or_default(),
                }
            })
            .collect();

        self.capture_apply_revision_prepared(
            tenant_id,
            repo_id,
            branch,
            node_changes,
            revision,
            crate::repositories::nodes::WriteAttribution {
                actor: Some(&actor),
                agent: operation_meta.as_ref().and_then(|m| m.agent.as_deref()),
            },
        )
        .await;
    }
}
