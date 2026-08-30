//! `VECTOR_OF(...)`: search by a node's OWN stored vector.
//!
//! # Why read the stored vector instead of re-encoding the node
//!
//! "Find things similar to this one" could be answered by fetching the node,
//! rebuilding its embedding input and calling the encoder again. Reading what
//! is already in `cf::EMBEDDINGS` is better on four counts, and the last is the
//! one that matters:
//!
//! * no provider call, so no latency, no cost and no rate limit on a query;
//! * it works when the encoder is offline or the tenant's key has been rotated
//!   — which is exactly when an image tower, hosted outside the database, is
//!   most likely to be unavailable;
//! * for an IMAGE there is nothing to re-encode from SQL at all. The pixels went
//!   through a tower this process does not host. The stored vector is the only
//!   representation the database has;
//! * **no drift.** A re-encode is only comparable to the index if the query-time
//!   pipeline reproduces the index-time one exactly — same model, same
//!   revision, same field selection, same chunker, same normalisation. Every one
//!   of those can move independently. When they do, the re-encoded vector lands
//!   somewhere else in the same R^n: every distance is still finite, every
//!   ranking still plausible, and nothing anywhere logs a fault. The stored
//!   vector cannot drift from itself.
//!
//! # The reference grammar
//!
//! ```text
//! VECTOR_OF('workspace:/path/to/node')      -- by path
//! VECTOR_OF('workspace:0193f2...')          -- by node id
//! VECTOR_OF('workspace:/path#spec')         -- a NAMED embedding spec
//! VECTOR_OF('workspace:/path', 3)           -- chunk 3 of a chunked document
//! ```
//!
//! The `workspace:` prefix is REQUIRED, exactly as it is for
//! `REFERENCES('workspace:/path')`. Embeddings are keyed by the workspace the
//! NODE lives in, not by the workspaces being searched, so a reference without
//! one would have to be resolved by guessing — and `workspaces => 'ALL
//! READABLE'` can be a dozen candidates.
//!
//! # Two things this module refuses to do
//!
//! **It will not pick chunk 0.** An image has one vector and "similar to this
//! image" has an answer. A chunked document has N, and the three ways to
//! collapse them behave very differently: chunk 0 is whichever paragraph came
//! first, a centroid of a multi-topic document is a point resembling none of
//! its parts, and max-sim is the right answer but costs N×M and returns a
//! different shape. Picking one silently would make `KNN(VECTOR_OF(doc))` mean
//! something the caller did not ask for and could not see. So: exactly one
//! stored chunk resolves; more than one is an error naming the count and the
//! argument that resolves it.
//!
//! **It will not leak a vector the caller may not read.** The reference node
//! goes through the same RLS pass every search hit does. Without that,
//! `VECTOR_OF` is an oracle: a caller who cannot read a node could still obtain
//! its embedding's neighbourhood, which is a description of its content.

use std::sync::Arc;

use raisin_embeddings::EmbeddingStorage;
use raisin_hnsw::PartitionId;
use raisin_models::auth::AuthContext;
use raisin_storage::{NodeRepository, Storage, StorageScope};

use crate::physical_plan::executor::ExecutionError;

use super::emit::rls_filter_search_hit;

/// How the reference node is addressed within its workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeLocator {
    /// A path, recognised by its leading `/`.
    Path(String),
    /// A node id.
    Id(String),
}

/// A parsed `VECTOR_OF(...)` argument. Not yet resolved: nothing here has
/// touched storage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VectorOfRef {
    /// Exactly what the caller wrote, for error messages and EXPLAIN.
    pub raw: String,
    /// The workspace the reference node lives in — and therefore the workspace
    /// its embedding is keyed under. NOT the set being searched.
    pub workspace: String,
    pub locator: NodeLocator,
    /// A named embedding spec (`#spec`), or `None` for the node's default one.
    pub spec: Option<String>,
    /// `VECTOR_OF(ref, n)`. `None` means "the source must have exactly one
    /// stored vector", which is the unambiguous case this ships for.
    pub chunk: Option<usize>,
}

impl VectorOfRef {
    /// Parse the reference string. `chunk` comes from the optional second
    /// argument and is passed in already validated.
    pub fn parse(raw: &str, chunk: Option<usize>) -> Result<Self, ExecutionError> {
        let raw = raw.trim();
        let Some((workspace, rest)) = raw.split_once(':') else {
            return Err(ExecutionError::Validation(format!(
                "VECTOR_OF('{raw}'): the reference must name a workspace, as \
                 'workspace:/path' or 'workspace:<node-id>'. Embeddings are keyed \
                 by the workspace the node lives in, which is not necessarily one \
                 of the workspaces being searched, so it cannot be inferred."
            )));
        };
        let workspace = workspace.trim();
        if workspace.is_empty() {
            return Err(ExecutionError::Validation(format!(
                "VECTOR_OF('{raw}'): the workspace before ':' is empty."
            )));
        }

        // `#spec` splits off the END, so a path containing no '#' is unaffected
        // and the spec cannot swallow part of the path. Same separator
        // `raisin_hnsw::namespaced_source_id` uses to BUILD the id, which is why
        // it is spelled the same way here rather than invented.
        let (target, spec) = match rest.rsplit_once('#') {
            Some((before, spec)) if !before.is_empty() && !spec.is_empty() => {
                (before, Some(spec.to_string()))
            }
            _ => (rest, None),
        };
        let target = target.trim();
        if target.is_empty() {
            return Err(ExecutionError::Validation(format!(
                "VECTOR_OF('{raw}'): no node path or id after '{workspace}:'."
            )));
        }

        let locator = if target.starts_with('/') {
            NodeLocator::Path(target.to_string())
        } else {
            NodeLocator::Id(target.to_string())
        };

        Ok(Self {
            raw: raw.to_string(),
            workspace: workspace.to_string(),
            locator,
            spec,
            chunk,
        })
    }

    /// How this reference reads in an error message or a log line.
    pub fn describe(&self) -> String {
        format!("VECTOR_OF('{}')", self.raw)
    }
}

/// The reference node, once resolved and authorised.
///
/// Held separately from the vectors because SELF-EXCLUSION needs the identity
/// even when a partition had no vector for it: a `kind => 'all'` query must not
/// return the reference node from the text leg just because it was addressed
/// through its image vector.
pub struct ResolvedSource {
    pub workspace: String,
    pub node_id: String,
    /// The namespaced `cf::EMBEDDINGS` source id — `{node}` or `{node}#{spec}`.
    pub source_id: String,
}

/// Resolve the reference node and check the caller may read it.
///
/// Returns `Err` rather than an empty result for a node that does not exist or
/// is not readable. An empty vector leg reported as a search is
/// indistinguishable from a corpus that matched nothing — the same reasoning
/// the leg dispatch already applies.
#[allow(clippy::too_many_arguments)]
pub async fn resolve_source<S: Storage>(
    reference: &VectorOfRef,
    storage: &Arc<S>,
    auth_context: Option<&AuthContext>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    max_revision: Option<&raisin_hlc::HLC>,
) -> Result<ResolvedSource, ExecutionError> {
    let scope = StorageScope::new(tenant_id, repo_id, branch, &reference.workspace);

    let node = match &reference.locator {
        NodeLocator::Path(path) => {
            storage
                .nodes()
                .get_by_path(scope, path, max_revision)
                .await?
        }
        NodeLocator::Id(id) => storage.nodes().get(scope, id, max_revision).await?,
    };

    let Some(node) = node else {
        return Err(ExecutionError::Validation(format!(
            "{}: no such node in workspace '{}'.",
            reference.describe(),
            reference.workspace
        )));
    };

    // The SAME pass every search hit goes through. A separate check here would
    // be a second definition of "may this caller see this node", and the two
    // would drift — which in this direction leaks the neighbourhood of a node
    // the caller cannot read.
    let node = rls_filter_search_hit(
        &**storage,
        node,
        auth_context,
        &reference.workspace,
        tenant_id,
        repo_id,
        branch,
        max_revision,
    )
    .await
    .ok_or_else(|| {
        // Deliberately the same wording as "no such node": distinguishing the
        // two would confirm the node's existence to a caller who may not read
        // it.
        ExecutionError::Validation(format!(
            "{}: no such node in workspace '{}'.",
            reference.describe(),
            reference.workspace
        ))
    })?;

    let source_id = raisin_hnsw::namespaced_source_id(&node.id, reference.spec.as_deref());
    Ok(ResolvedSource {
        workspace: reference.workspace.clone(),
        node_id: node.id,
        source_id,
    })
}

/// Read the reference's stored vector in ONE embedding space.
///
/// `Ok(None)` means this partition holds no vector for this source, which is a
/// normal outcome under `kind => 'all'` (a text-only node has no image vector).
/// The caller decides whether that leaves it with nothing to search.
pub fn stored_vector_for_partition(
    reference: &VectorOfRef,
    source: &ResolvedSource,
    embedding_storage: &Arc<dyn EmbeddingStorage>,
    partition: &PartitionId,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
) -> Result<Option<Vec<f32>>, ExecutionError> {
    let (Some(embedder_hash), Some(kind)) = (partition.embedder_hash(), partition.kind_char())
    else {
        return Err(ExecutionError::Backend(format!(
            "{}: vector partition '{partition}' has no readable embedder/kind \
             halves, so the stored vector cannot be addressed.",
            reference.describe()
        )));
    };

    let chunks = embedding_storage
        .list_source_chunk_indexes(
            tenant_id,
            repo_id,
            branch,
            &source.workspace,
            embedder_hash,
            kind,
            &source.source_id,
        )
        .map_err(|e| {
            ExecutionError::Backend(format!(
                "{}: cannot list the stored chunks of '{}': {e}",
                reference.describe(),
                source.source_id
            ))
        })?;

    if chunks.is_empty() {
        return Ok(None);
    }

    let chunk_idx = match reference.chunk {
        Some(requested) => {
            if !chunks.contains(&requested) {
                return Err(ExecutionError::Validation(format!(
                    "{}: chunk {requested} is not stored for this source in \
                     partition '{partition}'. It has {} ({}).",
                    reference.describe(),
                    plural_chunks(chunks.len()),
                    render_indexes(&chunks)
                )));
            }
            requested
        }
        None if chunks.len() == 1 => chunks[0],
        None => {
            // THE ambiguity, refused. See the module docs: chunk 0 is an
            // arbitrary paragraph, a centroid resembles none of the parts, and
            // max-sim is a different query shape. The caller has to say.
            return Err(ExecutionError::Validation(format!(
                "{}: this source has {} in partition '{partition}' ({}), so \
                 'similar to it' is ambiguous — its chunks are different vectors \
                 and chunk 0 is merely the one that came first. Name the chunk, \
                 e.g. VECTOR_OF('{}', 0). Assets and images store a single \
                 vector and need no chunk argument.",
                reference.describe(),
                plural_chunks(chunks.len()),
                render_indexes(&chunks),
                reference.raw
            )));
        }
    };

    let data = embedding_storage
        .get_source_chunk(
            tenant_id,
            repo_id,
            branch,
            &source.workspace,
            embedder_hash,
            kind,
            &source.source_id,
            chunk_idx,
            None,
        )
        .map_err(|e| {
            ExecutionError::Backend(format!(
                "{}: cannot read chunk {chunk_idx} of '{}': {e}",
                reference.describe(),
                source.source_id
            ))
        })?;

    // `list_source_chunk_indexes` just said this chunk exists, so a miss here is
    // a torn read between the two calls, not an absent vector. Reported as
    // absent (the caller's own "no vector in this partition" path) rather than
    // fabricated.
    Ok(data.map(|d| raisin_hnsw::normalize_vector(&d.vector)))
}

fn plural_chunks(n: usize) -> String {
    if n == 1 {
        "1 stored chunk".to_string()
    } else {
        format!("{n} stored chunks")
    }
}

fn render_indexes(indexes: &[usize]) -> String {
    let shown: Vec<String> = indexes.iter().take(8).map(|i| i.to_string()).collect();
    if indexes.len() > 8 {
        format!("indexes {}, ...", shown.join(", "))
    } else {
        format!("indexes {}", shown.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_reference_parses() {
        let r = VectorOfRef::parse("assets:/photos/cat.jpg", None).unwrap();
        assert_eq!(r.workspace, "assets");
        assert_eq!(r.locator, NodeLocator::Path("/photos/cat.jpg".into()));
        assert_eq!(r.spec, None);
        assert_eq!(r.chunk, None);
    }

    /// A node id has no leading slash, and that is the whole discriminator —
    /// the same one the rest of the SQL surface uses.
    #[test]
    fn an_id_reference_parses() {
        let r = VectorOfRef::parse("assets:0193f2ab-cafe", None).unwrap();
        assert_eq!(r.locator, NodeLocator::Id("0193f2ab-cafe".into()));
    }

    /// `#spec` is the SAME separator `namespaced_source_id` builds the stored
    /// key with. Spelling it differently here would address a row that the
    /// writer never wrote.
    #[test]
    fn a_named_spec_splits_off_the_end() {
        let r = VectorOfRef::parse("library:/manuals/boiler#doc", None).unwrap();
        assert_eq!(r.locator, NodeLocator::Path("/manuals/boiler".into()));
        assert_eq!(r.spec, Some("doc".into()));
    }

    /// The workspace prefix is not optional, because the embedding key needs it
    /// and `workspaces => 'ALL READABLE'` gives nothing to guess from.
    #[test]
    fn a_reference_without_a_workspace_is_refused() {
        let err = VectorOfRef::parse("/photos/cat.jpg", None).unwrap_err();
        assert!(format!("{err:?}").contains("workspace"), "{err:?}");
        assert!(VectorOfRef::parse(":/photos/cat.jpg", None).is_err());
        assert!(VectorOfRef::parse("assets:", None).is_err());
    }

    #[test]
    fn the_chunk_argument_is_carried() {
        let r = VectorOfRef::parse("library:/manuals/boiler", Some(3)).unwrap();
        assert_eq!(r.chunk, Some(3));
    }
}
