//! The work items of a `compound_builds` link (`compound_builds.rs`): a
//! build through the shared per-index build, and the drop of an undeclared
//! workspace index.

use super::compound_detect::Work;
use super::{RepairOptions, RepairReport};
use crate::RocksDBStorage;
use raisin_error::{Error, Result};
use std::sync::Arc;

/// One work item of a link.
pub(super) async fn run_item(
    storage: &RocksDBStorage,
    handler: &crate::jobs::handlers::CompoundIndexJobHandler,
    (tenant_id, repo_id, branch): (&str, &str, &str),
    item: &Work,
    options: &RepairOptions,
    report: &mut RepairReport,
) -> Result<()> {
    match item {
        Work::Build {
            workspace,
            index,
            owner,
        } => {
            build(
                storage,
                handler,
                (tenant_id, repo_id, branch, workspace),
                index,
                owner.as_deref(),
                options,
                report,
            )
            .await
        }
        Work::Drop { workspace, index } => {
            drop_index(
                storage.db(),
                (tenant_id, repo_id, branch, workspace),
                index,
                report,
            )
            .await
        }
    }
}

type BranchWs<'a> = (&'a str, &'a str, &'a str, &'a str);

async fn build(
    storage: &RocksDBStorage,
    handler: &crate::jobs::handlers::CompoundIndexJobHandler,
    (tenant_id, repo_id, branch, workspace): BranchWs<'_>,
    index: &str,
    owner: Option<&str>,
    options: &RepairOptions,
    report: &mut RepairReport,
) -> Result<()> {
    let owner = match owner {
        Some(owner) => owner.to_string(),
        // A node-type index: any type that carries the name (inheritance
        // resolved) finds the same declaration and the same carriers.
        None => {
            let scope = raisin_storage::BranchScope::new(tenant_id, repo_id, branch);
            let fresh = crate::indexing::compound::defs::fresh_branch(
                storage.db(),
                &storage.node_types,
                scope,
                &[],
            )
            .await?;
            match fresh
                .iter()
                .find(|(_, defs)| defs.compound.iter().any(|d| d.name == index))
            {
                Some((name, _)) => name.clone(),
                None => {
                    report.compound.undeclared += 1;
                    return Ok(());
                }
            }
        }
    };
    let label = format!("compound_builds:{branch}");
    let built = handler
        .build_index(
            &label,
            tenant_id,
            repo_id,
            branch,
            workspace,
            &owner,
            index,
            options.max_bytes_per_sec,
        )
        .await?;
    use crate::jobs::handlers::CompoundBuildResult;
    match built {
        CompoundBuildResult::Ready => {
            report.compound.built += 1;
            Ok(())
        }
        CompoundBuildResult::MissingOrderValues(nodes) => {
            // Expected, not a failure (plan Phase 13g): recorded in the
            // repair state, one WARN naming the fix, no error and no retry.
            report.compound.refused += 1;
            report.compound.refused_missing_order_values += nodes;
            tracing::warn!(
                index,
                "{}",
                crate::indexing::compound::build::missing_order_values_message(
                    &crate::indexing::IndexCtx::new(tenant_id, repo_id, branch, workspace),
                    nodes as usize,
                )
            );
            Ok(())
        }
        CompoundBuildResult::NotReady => Err(Error::storage(format!(
            "compound index {index} on {workspace} did not reach Ready (kept being marked)"
        ))),
    }
}

async fn drop_index(
    db: &Arc<rocksdb::DB>,
    (tenant_id, repo_id, branch, workspace): BranchWs<'_>,
    index: &str,
    report: &mut RepairReport,
) -> Result<()> {
    use crate::indexing::compound::keyspace;
    let _keyspace = keyspace::lock(db, tenant_id, repo_id, branch, workspace, index).await;
    if !crate::compound_state::forget_undeclared_workspace_index(
        db, tenant_id, repo_id, branch, workspace, index,
    )? {
        return Ok(()); // declared again meanwhile
    }
    let _inserting = crate::management::cf_exclusion::enter_inserter_async(
        db,
        tenant_id,
        repo_id,
        branch,
        crate::cf::COMPOUND_INDEX,
    )
    .await;
    report.compound.dropped_entries +=
        keyspace::clear(db, tenant_id, repo_id, branch, workspace, index)?;
    report.compound.dropped += 1;
    tracing::info!(
        tenant_id,
        repo_id,
        branch,
        workspace,
        index,
        "compound_builds: dropped an undeclared workspace index"
    );
    Ok(())
}
