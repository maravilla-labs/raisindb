//! The streaming build: estimate → pre-flight → write → seal → terminal.
//!
//! Nothing here holds the selection or the archive in memory. The selection
//! is walked a page at a time (pre-order DFS, 500 nodes per page), each node
//! filtered through the requester's permissions (E3) and written straight
//! into the archive, which streams into the blob store through a
//! [`BlobWriter`] while its sha256 and size are computed in the same pass
//! (E23). Binaries are copied chunk by chunk. Inventory lines go to an
//! external sort; the central directory spills to a file. Peak memory is one
//! page, one chunk and one sort run, whatever the archive's size (E21).
//!
//! Every exit path leaves no object behind: an error or cancel aborts every
//! open writer and deletes anything already committed; a panic or an
//! aborted task drops the writers, whose guards remove their partials; a
//! killed process leaves partials the boot sweep removes.

use super::archive;
use super::estimate;
use super::inventory::{InventorySorter, DEFAULT_RUN_BYTES};
use super::manifest_types::{ManifestInput, PackageManifest};
use super::mirror::{blob_resource, estimate_value, Mirror, RecordSignal, WsCount};
use super::progress::{blob, check, insufficient, io, Progress, Stop};
use super::request::{self, SelectionRoot};
use super::types::{PackageCreateFromSelectionResult, SelectedPath};
use super::PackageCreateFromSelectionHandler;
use raisin_binary::zip_stream::{Compression, ZipStreamWriter};
use raisin_binary::{BlobWriter, DynBinaryStorage, HashingWriter, StoredObject, WriteHint};
use raisin_core::package_registration;
use raisin_core::services::package_export_access;
use raisin_error::{Error, Result};
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_packages::Manifest;
use raisin_storage::jobs::{JobContext, JobInfo, JobStatus, JobType};
use raisin_storage::{
    CreateNodeOptions, ListOptions, NodeRepository, Storage, StorageScope, UpdateNodeOptions,
};
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

/// Nodes per page of the selection walk.
const PAGE_SIZE: usize = 500;
/// How often the record is polled for a caller's `cancel_requested`.
const CANCEL_POLL: Duration = Duration::from_secs(1);

/// The parsed job.
struct Spec {
    name: String,
    version: String,
    include_node_types: bool,
    roots: Vec<SelectionRoot>,
    register_package: bool,
    requester: AuthContext,
    title: Option<String>,
    description: Option<String>,
    author: Option<String>,
}

/// What a finished build produced.
struct Built {
    archive: StoredObject,
    archive_sha256: String,
    manifest: StoredObject,
    manifest_sha256: String,
    inventory: StoredObject,
    inventory_sha256: String,
    items: u64,
    node_types: u64,
}

/// Blobs this build has committed, deleted again if the build fails later.
#[derive(Default)]
struct Committed(Vec<String>);

impl PackageCreateFromSelectionHandler {
    /// Handle a package build job.
    pub async fn handle(
        &self,
        job: &JobInfo,
        context: &JobContext,
    ) -> Result<Option<serde_json::Value>> {
        let record = request::record_from_metadata(&context.metadata)?;
        let mut mirror = Mirror::new(
            self.storage.clone(),
            self.job_registry.clone(),
            job.id.clone(),
            &context.tenant_id,
            &context.repo_id,
            &context.branch,
            record,
        );
        let cancel = self
            .job_registry
            .get_cancel_token(&job.id)
            .await
            .map(|t| t.child_token())
            .unwrap_or_default();
        let _watch = mirror
            .record()
            .cloned()
            .map(|r| self.watch_record_for_cancel(context, r, cancel.clone()));

        // A restored job whose record already says this very job was running
        // is the second attempt of a build the server lost mid-write (the
        // job system re-queues what was running at a crash). The record has
        // already shown that attempt's progress, so it is reported as `lost`
        // rather than silently restarted, and the attempt's partial objects —
        // named after the job, so findable — are discarded.
        if mirror.was_running_as(&job.id).await {
            if let Some(store) = self.storage.blob_store() {
                let _ = store.discard_partial(&archive_partial_id(&job.id)).await;
                let _ = store.discard_partial(&inventory_partial_id(&job.id)).await;
            }
            let message = "The server stopped while this build was running; the partial \
                           archive was discarded"
                .to_string();
            mirror.log("error", format!("lost: {message}")).await;
            let mut err = HashMap::new();
            err.insert("code".to_string(), PropertyValue::String("lost".into()));
            err.insert(
                "message".to_string(),
                PropertyValue::String(message.clone()),
            );
            let mut extra = HashMap::new();
            extra.insert("error".to_string(), PropertyValue::Object(err));
            mirror.terminal("failed", extra).await;
            return Err(Error::Validation(format!("lost: {message}")));
        }

        let mut committed = Committed::default();
        let outcome = self
            .build(job, context, &mut mirror, &cancel, &mut committed)
            .await;
        let store = self.storage.blob_store();

        match outcome {
            Ok((spec, built, log_resource)) => {
                let mut extra = HashMap::new();
                extra.insert(
                    "artifact".to_string(),
                    blob_resource(
                        &built.archive,
                        &built.archive_sha256,
                        &format!("{}-{}.rap", spec.name, spec.version),
                        "application/zip",
                    ),
                );
                extra.insert(
                    "manifest".to_string(),
                    blob_resource(
                        &built.manifest,
                        &built.manifest_sha256,
                        "manifest.yaml",
                        "application/yaml",
                    ),
                );
                extra.insert(
                    "inventory".to_string(),
                    blob_resource(
                        &built.inventory,
                        &built.inventory_sha256,
                        "inventory.jsonl",
                        "application/jsonl",
                    ),
                );
                if let Some((stored, sha)) = &log_resource {
                    extra.insert(
                        "log".to_string(),
                        blob_resource(stored, sha, "build.log", "text/plain"),
                    );
                }
                extra.insert(
                    "size_bytes".to_string(),
                    PropertyValue::Integer(built.archive.size),
                );
                extra.insert(
                    "sha256".to_string(),
                    PropertyValue::String(built.archive_sha256.clone()),
                );
                extra.insert("fidelity".to_string(), fidelity(&spec));
                mirror.state.phase = "done";
                mirror.terminal("completed", extra).await;

                let result = PackageCreateFromSelectionResult {
                    package_name: spec.name.clone(),
                    package_version: spec.version.clone(),
                    nodes_included: built.items as usize,
                    node_types_included: built.node_types as usize,
                    blob_key: built.archive.key.clone(),
                    download_url: built.archive.url.clone(),
                    created_at: chrono::Utc::now().to_rfc3339(),
                    size_bytes: built.archive.size.max(0) as u64,
                    sha256: built.archive_sha256.clone(),
                    manifest_key: Some(built.manifest.key.clone()),
                    inventory_key: Some(built.inventory.key.clone()),
                };
                let mut value = serde_json::to_value(result).unwrap_or_default();
                package_export_access::stamp_result_repo(&mut value, &context.repo_id);
                Ok(Some(value))
            }
            Err(stop) => {
                if let Some(store) = &store {
                    for key in &committed.0 {
                        let _ = store.delete(key).await;
                    }
                }
                let (status, code, message, detail) = match stop {
                    Stop::Cancelled => ("cancelled", None, "Cancelled".to_string(), None),
                    Stop::Refused {
                        code,
                        message,
                        detail,
                    } => ("failed", Some(code), message, detail),
                    Stop::Failed(e) => ("failed", Some("other"), e.to_string(), None),
                };
                let mut extra = HashMap::new();
                if let Some(code) = code {
                    mirror
                        .log(
                            "error",
                            format!("{code}: {message}. The partial archive was deleted."),
                        )
                        .await;
                    let mut err = HashMap::new();
                    err.insert("code".to_string(), PropertyValue::String(code.to_string()));
                    err.insert(
                        "message".to_string(),
                        PropertyValue::String(message.clone()),
                    );
                    if let Some(d) = detail {
                        err.insert("detail".to_string(), PropertyValue::String(d.to_string()));
                    }
                    extra.insert("error".to_string(), PropertyValue::Object(err));
                } else {
                    mirror
                        .log("warn", "Cancelled. The partial archive was deleted.")
                        .await;
                }
                mirror.terminal(status, extra).await;
                if status == "cancelled" {
                    let _ = self
                        .job_registry
                        .update_status(&job.id, JobStatus::Cancelled)
                        .await;
                    return Err(Error::InvalidState("package build cancelled".to_string()));
                }
                Err(match code {
                    Some("forbidden") => Error::Forbidden(message),
                    Some(c) if c != "other" => Error::Validation(format!("{c}: {message}")),
                    _ => Error::storage(message),
                })
            }
        }
    }

    /// Cancel `cancel` when the record's status becomes `cancel_requested`.
    fn watch_record_for_cancel(
        &self,
        context: &JobContext,
        record: super::mirror::RecordRef,
        cancel: CancellationToken,
    ) -> AbortOnDrop {
        let storage = self.storage.clone();
        let (tenant, repo, branch) = (
            context.tenant_id.clone(),
            context.repo_id.clone(),
            context.branch.clone(),
        );
        AbortOnDrop(tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => return,
                    _ = tokio::time::sleep(CANCEL_POLL) => {}
                }
                let scope = StorageScope::new(&tenant, &repo, &branch, &record.workspace);
                if let Ok(Some(node)) = storage.nodes().get_by_path(scope, &record.path, None).await
                {
                    if matches!(
                        node.properties.get("status"),
                        Some(PropertyValue::String(s)) if s == "cancel_requested"
                    ) {
                        cancel.cancel();
                        return;
                    }
                }
            }
        }))
    }

    async fn build(
        &self,
        job: &JobInfo,
        context: &JobContext,
        mirror: &mut Mirror,
        cancel: &CancellationToken,
        committed: &mut Committed,
    ) -> std::result::Result<(Spec, Built, Option<(StoredObject, String)>), Stop> {
        let spec = parse_spec(job, context)?;
        let store = self.storage.blob_store().ok_or_else(|| Stop::Refused {
            code: "engine_unsupported",
            message: "This server has no blob store configured for package builds".to_string(),
            detail: None,
        })?;
        let (tenant, repo, branch) = (
            context.tenant_id.as_str(),
            context.repo_id.as_str(),
            context.branch.as_str(),
        );
        mirror
            .log(
                "info",
                format!(
                    "Building {}-{} from {} selected root(s)",
                    spec.name,
                    spec.version,
                    spec.roots.len()
                ),
            )
            .await;

        // ---- estimate ------------------------------------------------------
        check(mirror.phase("estimate").await, cancel)?;
        let est = estimate::estimate(&self.storage, tenant, repo, branch, &spec.roots).await?;
        mirror
            .log(
                "info",
                format!(
                    "Estimate: {} items, {} bytes{}",
                    est.items,
                    est.bytes,
                    if est.partial {
                        " (partial: still counting)"
                    } else {
                        ""
                    }
                ),
            )
            .await;
        mirror.state.estimate = Some(est.clone());
        check(mirror.flush().await, cancel)?;

        // ---- pre-flight ----------------------------------------------------
        let spill_dir = spill_dir();
        let floor = self.space_floor.or_else(estimate::floor_override);
        let archive_space = store.available_space().await.map_err(blob)?;
        if let Err(short) = estimate::room_for(archive_space, estimate::archive_need(&est), floor) {
            return Err(insufficient(short, 0, est.bytes));
        }
        if let Err(short) = estimate::room_for(
            estimate::local_space(&spill_dir),
            estimate::spill_need(&est),
            floor,
        ) {
            return Err(insufficient(short, 0, est.bytes));
        }

        // ---- write ---------------------------------------------------------
        check(mirror.phase("write").await, cancel)?;
        let writer = store
            .begin_write(
                WriteHint::new()
                    .content_type("application/zip")
                    .ext("rap")
                    .original_name(format!("{}-{}.rap", spec.name, spec.version))
                    .tenant(Some(tenant))
                    .partial_id(archive_partial_id(&job.id)),
            )
            .await
            .map_err(blob)?;
        let mut zip = ZipStreamWriter::with_spill(HashingWriter::new(writer), &spill_dir)
            .await
            .map_err(io)?;
        let mut inventory = InventorySorter::new(&spill_dir, DEFAULT_RUN_BYTES);
        let mut node_types_used: BTreeSet<String> = BTreeSet::new();
        let mut progress = Progress::new(mirror, store.clone(), floor, cancel.clone(), est.bytes);

        let mut any_root = false;
        for (idx, root) in spec.roots.iter().enumerate() {
            if request::covered_by(&spec.roots, idx) {
                continue;
            }
            let scope = StorageScope::new(tenant, repo, branch, &root.workspace);
            let (start_id, single) = if root.whole_workspace() {
                (crate::constants::ROOT_PARENT_ID.to_string(), None)
            } else {
                match self
                    .storage
                    .nodes()
                    .get_by_path(scope, &root.path, None)
                    .await?
                {
                    Some(n) if root.recursive => (n.id, None),
                    Some(n) => (n.id.clone(), Some(n)),
                    None => {
                        progress
                            .mirror
                            .log(
                                "warn",
                                format!("{} does not exist; skipped", root.spelling()),
                            )
                            .await;
                        progress.counters.warnings += 1;
                        continue;
                    }
                }
            };
            any_root = true;
            progress
                .mirror
                .log("info", format!("Writing {}", root.spelling()))
                .await;

            let mut after: Option<String> = None;
            loop {
                progress.check_cancel()?;
                let page: Vec<(Node, String)> = match &single {
                    Some(n) => {
                        if after.is_some() {
                            Vec::new()
                        } else {
                            vec![(n.clone(), String::new())]
                        }
                    }
                    None => {
                        self.storage
                            .nodes()
                            .scan_descendants_ordered_page(
                                scope,
                                &start_id,
                                after.as_deref(),
                                Some(PAGE_SIZE),
                                ListOptions::default(),
                            )
                            .await?
                    }
                };
                let Some((_, last_order)) = page.last() else {
                    break;
                };
                let last_order = last_order.clone();
                let full_page = single.is_none() && page.len() >= PAGE_SIZE;

                for (node, _) in page {
                    progress.check_cancel()?;
                    let Some(node) = package_export_access::readable_by_requester(
                        node,
                        &spec.requester,
                        &root.workspace,
                        branch,
                    ) else {
                        progress.counters.skipped += 1;
                        continue;
                    };
                    let has_children = archive::embedded_file(&node).is_none()
                        && self
                            .storage
                            .nodes()
                            .has_children(scope, &node.id, None)
                            .await
                            .unwrap_or(false);
                    progress.mirror.state.current =
                        Some(format!("{}:{}", root.workspace, node.path));
                    let written = archive::write_content_node(
                        &mut zip,
                        store.as_ref(),
                        &root.workspace,
                        &node,
                        has_children,
                        &mut progress,
                    )
                    .await
                    .map_err(|e| progress.stop_for(e))?;
                    progress.check_cancel()?;
                    if let Some(key) = &written.missing_binary {
                        progress.counters.warnings += 1;
                        progress
                            .mirror
                            .log(
                                "warn",
                                format!(
                                    "source_unreadable: {}:{} has no stored file ({}); continued without it",
                                    root.workspace, node.path, key
                                ),
                            )
                            .await;
                    }
                    if !node.node_type.is_empty() {
                        node_types_used.insert(node.node_type.clone());
                    }
                    inventory.push(&written.line).await.map_err(io)?;
                    progress.node_done(&root.workspace, &written);
                    progress.revise_estimate();
                    progress.recheck_space().await?;
                    check(progress.mirror.tick().await, cancel)?;
                }

                if !full_page {
                    break;
                }
                after = Some(last_order);
            }
        }
        if !any_root {
            return Err(Stop::Refused {
                code: "missing_root",
                message: "None of the selected paths exists".to_string(),
                detail: Some(serde_json::json!(spec
                    .roots
                    .iter()
                    .map(|r| r.spelling())
                    .collect::<Vec<_>>())),
            });
        }

        // ---- node types ----------------------------------------------------
        let mut node_types_written = 0u64;
        if spec.include_node_types && !node_types_used.is_empty() {
            check(progress.mirror.phase("nodetypes").await, cancel)?;
            let nt_scope = StorageScope::new(tenant, repo, branch, "nodetypes");
            for nt in &node_types_used {
                progress.check_cancel()?;
                let Some(node) = self
                    .storage
                    .nodes()
                    .get_by_path(nt_scope, &format!("/{nt}"), None)
                    .await?
                else {
                    continue;
                };
                let Some(node) = package_export_access::readable_by_requester(
                    node,
                    &spec.requester,
                    "nodetypes",
                    branch,
                ) else {
                    continue;
                };
                let written = archive::write_node_type(&mut zip, &node).await?;
                inventory.push(&written.line).await.map_err(io)?;
                progress.node_done("nodetypes", &written);
                node_types_written += 1;
                progress.counters.node_types = node_types_written;
                check(progress.mirror.tick().await, cancel)?;
            }
        }

        // ---- seal ----------------------------------------------------------
        check(progress.mirror.phase("seal").await, cancel)?;
        let manifest = PackageManifest::build(ManifestInput {
            name: &spec.name,
            version: &spec.version,
            title: spec.title.clone(),
            description: spec.description.clone(),
            author: spec.author.clone(),
            content: spec.roots.iter().map(|r| r.spelling()).collect(),
            workspaces: progress
                .counters
                .per_ws
                .keys()
                .filter(|ws| ws.as_str() != "nodetypes")
                .cloned()
                .collect(),
            node_types: node_types_used.iter().cloned().collect(),
        });
        let manifest_yaml = serde_yaml::to_string(&manifest)
            .map_err(|e| Error::storage(format!("Failed to serialize manifest: {e}")))?;
        zip.write_entry(
            "manifest.yaml",
            Compression::Deflate(6),
            manifest_yaml.as_bytes(),
        )
        .await
        .map_err(io)?;

        // The inventory goes into the archive AND to its sidecar in one merge.
        let inv_writer = store
            .begin_write(
                WriteHint::new()
                    .content_type("application/jsonl")
                    .ext("jsonl")
                    .original_name("inventory.jsonl")
                    .tenant(Some(tenant))
                    .partial_id(inventory_partial_id(&job.id)),
            )
            .await
            .map_err(blob)?;
        let mut inv_sidecar = HashingWriter::new(inv_writer);
        let items = inventory.lines();
        {
            let mut entry = zip
                .start_entry("inventory.jsonl", Compression::Deflate(6))
                .await
                .map_err(io)?;
            let mut sorted = inventory.finish().await.map_err(io)?;
            while let Some(line) = sorted.next().await.map_err(io)? {
                if cancel.is_cancelled() {
                    return Err(Stop::Cancelled);
                }
                entry.write(line.as_bytes()).await.map_err(io)?;
                inv_sidecar.write_all(line.as_bytes()).await.map_err(io)?;
            }
            entry.finish().await.map_err(io)?;
        }
        progress.check_cancel()?;

        let hashing = zip.finish().await.map_err(io)?;
        let (archive_writer, archive_digest) = hashing.finish();
        let archive = archive_writer.commit().await.map_err(blob)?;
        committed.0.push(archive.key.clone());

        let (inv_writer, inv_digest) = inv_sidecar.finish();
        let inventory_obj = inv_writer.commit().await.map_err(blob)?;
        committed.0.push(inventory_obj.key.clone());

        let (manifest_obj, manifest_sha) = put_small(
            store.as_ref(),
            tenant,
            "manifest.yaml",
            "yaml",
            "application/yaml",
            manifest_yaml.as_bytes(),
        )
        .await?;
        committed.0.push(manifest_obj.key.clone());

        progress
            .mirror
            .log(
                "info",
                format!(
                    "Sealed: {} items, {} bytes, sha256 {}",
                    items, archive_digest.size, archive_digest.sha256
                ),
            )
            .await;

        // E4: only a legacy request registers `packages:/<name>`.
        if spec.register_package {
            self.create_or_update_package_node(
                context,
                &spec.name,
                &spec.version,
                &archive.key,
                &archive.url,
                archive_digest.size as usize,
            )
            .await?;
        }

        let log = put_small(
            store.as_ref(),
            tenant,
            "build.log",
            "log",
            "text/plain",
            progress.mirror.state.full_log().as_bytes(),
        )
        .await
        .ok();
        if let Some((stored, _)) = &log {
            committed.0.push(stored.key.clone());
        }

        let _ = self.job_registry.update_progress(&job.id, 1.0).await;
        let node_types = node_types_written;
        Ok((
            spec,
            Built {
                archive,
                archive_sha256: archive_digest.sha256,
                manifest: manifest_obj,
                manifest_sha256: manifest_sha,
                inventory: inventory_obj,
                inventory_sha256: inv_digest.sha256,
                items,
                node_types,
            },
            log,
        ))
    }
}

/// Stable partial names, so a later attempt of the same job can find them.
fn archive_partial_id(job: &raisin_storage::jobs::JobId) -> String {
    format!("pkgbuild-{}", job)
}

fn inventory_partial_id(job: &raisin_storage::jobs::JobId) -> String {
    format!("pkgbuild-{}-inventory", job)
}

/// Aborts a spawned task when dropped.
struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

fn parse_spec(job: &JobInfo, context: &JobContext) -> std::result::Result<Spec, Stop> {
    let (name, version, include_node_types) = match &job.job_type {
        JobType::PackageCreateFromSelection {
            package_name,
            package_version,
            include_node_types,
        } => (
            package_name.clone(),
            package_version.clone(),
            *include_node_types,
        ),
        _ => {
            return Err(Stop::Failed(Error::Validation(
                "Expected PackageCreateFromSelection job type".to_string(),
            )))
        }
    };
    let selected: Vec<SelectedPath> = context
        .metadata
        .get(request::KEY_SELECTED_PATHS)
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    if selected.is_empty() {
        return Err(Stop::Failed(Error::Validation(
            "No paths selected for package creation".to_string(),
        )));
    }
    // E3: the selection is read AS THE REQUESTER; a job without one is refused.
    let requester =
        package_export_access::requester_from_metadata(&context.metadata).ok_or_else(|| {
            Stop::Refused {
                code: "forbidden",
                message: "Package export job carries no requester identity; refusing to read \
                      content without one"
                    .to_string(),
                detail: None,
            }
        })?;
    Ok(Spec {
        name,
        version,
        include_node_types,
        roots: request::selection_roots(&selected)?,
        register_package: request::register_package_from_metadata(&context.metadata),
        requester,
        title: request::string_from_metadata(&context.metadata, "title"),
        description: request::string_from_metadata(&context.metadata, "description"),
        author: request::string_from_metadata(&context.metadata, "author"),
    })
}

/// What the build honoured, for the record (E10 tracks the rest).
fn fidelity(spec: &Spec) -> PropertyValue {
    let mut m = HashMap::new();
    let s = |v: &str| PropertyValue::String(v.to_string());
    m.insert(
        "node_types".to_string(),
        s(if spec.include_node_types {
            "honoured"
        } else {
            "off"
        }),
    );
    m.insert("binaries".to_string(), s("honoured"));
    m.insert("schema".to_string(), s("unsupported"));
    m.insert("translations".to_string(), s("unsupported"));
    m.insert("acl".to_string(), s("unsupported"));
    m.insert("secrets".to_string(), s("unsupported"));
    PropertyValue::Object(m)
}

/// Where the inventory runs and the central directory spill.
fn spill_dir() -> std::path::PathBuf {
    std::env::temp_dir().join("raisin-package-build")
}

/// Write a small blob in one go; returns it with its sha256.
async fn put_small(
    store: &dyn DynBinaryStorage,
    tenant: &str,
    name: &str,
    ext: &str,
    mime: &str,
    bytes: &[u8],
) -> std::result::Result<(StoredObject, String), Stop> {
    let w = store
        .begin_write(
            WriteHint::new()
                .content_type(mime)
                .ext(ext)
                .original_name(name)
                .tenant(Some(tenant)),
        )
        .await
        .map_err(blob)?;
    let mut w = HashingWriter::new(w);
    if let Err(e) = w.write_all(bytes).await {
        let (w, _) = w.finish();
        let _ = w.abort().await;
        return Err(io(e));
    }
    let (w, digest) = w.finish();
    let stored = w.commit().await.map_err(blob)?;
    Ok((stored, digest.sha256))
}
