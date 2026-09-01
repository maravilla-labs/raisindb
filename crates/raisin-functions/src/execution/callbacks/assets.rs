// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The extraction WRITEBACK: how text produced above this process gets indexed.
//!
//! # Why a primitive exists at all
//!
//! Core's extraction vocabulary is one mimetype. `is_extractable_mime` matches
//! `application/pdf` and nothing else, because the readers for Word,
//! PowerPoint and Excel are LibreOffice, which must not live inside the
//! raisindb process. Those formats are converted by a media plugin, called from
//! a function — the only layer that can reach a plugin.
//!
//! So the text exists in JavaScript and has to reach a node property. A
//! function cannot simply write one: `__extracted_text` and its siblings are
//! engine-owned (`is_engine_owned_property_key`), and the write-path shield
//! refuses them to every actor but the two that own them. That refusal is
//! correct and is exactly why this primitive is needed — without it the only
//! ways to land plugin-produced text were to open the shield to tenant code, or
//! to give up and let Studio keep a parallel chunking path of its own.
//!
//! # What it deliberately does NOT take
//!
//! The FINGERPRINT. It is computed here, server-side, from the node as it
//! stands. Handing the grammar to JavaScript would put a second producer of the
//! stamp on the other side of a wire, and a stamp that disagreed by one byte
//! would never match the enqueue gate — so every writeback would re-enqueue
//! extraction, which would hand off again, which would write back again. An
//! unbounded loop, each turn minting a node revision, a fulltext reindex and an
//! embedding call. `raisin_models::nodes::asset_fingerprint` is the one
//! implementation and both sides call it.
//!
//! It also does not take the CHUNKING or the EMBEDDING. Writing the property is
//! the whole job: a node write emits `node:updated`, and the ordinary indexing
//! path chunks and embeds from there, under the spec hash that makes a
//! steady-state run write nothing. A second chunker in JS would have to
//! reproduce the `{node}#doc#{chunk}` id grammar, and an index built with ids
//! the live path never produces is a search that finds nothing and reports no
//! fault.
//!
//! # Why NodeService and not the repository
//!
//! Node events come from `NodeService` and the flow callbacks, never from the
//! raw repository: `storage.nodes().update(...)` is SILENT. Writing the text
//! that way would store it and index NOTHING — no `node:updated`, so no
//! fulltext, no embedding, no subscription — which is the precise opposite of
//! the point.

use std::sync::Arc;

use raisin_binary::BinaryStorage;
use raisin_core::NodeService;
use raisin_error::{Error, Result};
use raisin_models::nodes::{asset_fingerprint, ExtractStatus, ExtractionArtifact};
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::Storage;
use serde_json::{json, Value};

use crate::api::AssetSetExtractionCallback;

/// The `source` recorded when the caller does not name one.
///
/// Free-form on purpose (`ExtractionArtifact::source` is a string so an
/// out-of-tree plugin can name itself), but never empty: `__extract_source` is
/// what tells an operator whether text came from core's PDF reader or from a
/// converter, and an empty one makes the two indistinguishable.
const DEFAULT_SOURCE: &str = "plugin";

/// Create the `raisin.assets.setExtractedText` callback.
///
/// `(workspace, nodeIdOrPath, text, options) -> { status, chars, stored, source }`
///
/// `options.source` names the extractor (`plugin-libreoffice`). `options.store`
/// mirrors the rule's `store_extracted_text`: false records the STATUS and
/// discards the text, which is a real configuration — the status is bookkeeping
/// about the binary, and losing it is what made a skip invisible to begin with.
pub fn create_asset_set_extraction<S, B>(
    storage: Arc<S>,
    tenant_id: String,
    repo_id: String,
    branch: String,
) -> AssetSetExtractionCallback
where
    S: Storage + TransactionalStorage + 'static,
    B: BinaryStorage + 'static,
{
    Arc::new(
        move |workspace: String, node_ref: String, text: String, options: Value| {
            let storage = storage.clone();
            let tenant_id = tenant_id.clone();
            let repo_id = repo_id.clone();
            let branch = branch.clone();

            Box::pin(async move {
                let source = options
                    .get("source")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or(DEFAULT_SOURCE)
                    .to_string();
                let store_text = options
                    .get("store")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);

                // The NAMED engine actor, not plain `system`. The write-path
                // shield opens only for the actors that own `__` properties,
                // and `AuthContext::system()` is what half the internal
                // services and every test harness present — so `system` would
                // silently drop the artifact and report success.
                let svc: NodeService<S> = NodeService::new_with_context(
                    storage,
                    tenant_id,
                    repo_id,
                    branch,
                    workspace.clone(),
                )
                .with_auth(raisin_models::auth::AuthContext::system_as(
                    raisin_models::nodes::EXTRACTION_ACTOR,
                ));

                // A path is accepted as well as an id because the caller is a
                // trigger function, which is handed whichever the event
                // carried. Requiring one spelling would put the lookup in every
                // function instead of here.
                let node: Option<raisin_models::nodes::Node> = if node_ref.starts_with('/') {
                    svc.get_by_path(&node_ref).await?
                } else {
                    svc.get(&node_ref).await?
                };
                let mut node = node.ok_or_else(|| {
                    Error::NotFound(format!(
                        "Asset not found for extraction writeback: {node_ref}"
                    ))
                })?;

                // Computed from the node as it stands NOW, never taken from the
                // caller — see the module docs. A file replaced while the
                // conversion was running produces a stamp for the NEW bytes,
                // which does not match the text just produced; the gate then
                // re-extracts, which is the correct outcome.
                let fingerprint = asset_fingerprint(&node);

                let artifact = ExtractionArtifact::extracted(fingerprint, source.clone(), text);
                artifact.apply(&mut node.properties, store_text);

                let status = artifact.status;
                let chars = raisin_models::nodes::extracted_text(&node.properties)
                    .map(|t| t.chars().count())
                    .unwrap_or(0);

                svc.update_node(node).await?;

                tracing::info!(
                    workspace = %workspace,
                    node_ref = %node_ref,
                    source = %source,
                    status = %status.as_str(),
                    stored_chars = chars,
                    "Extraction writeback stored; indexing follows from node:updated"
                );

                Ok(json!({
                    "status": status.as_str(),
                    "source": source,
                    "chars": chars,
                    "stored": store_text && status == ExtractStatus::Ok,
                }))
            })
        },
    )
}
