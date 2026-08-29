//! Reading a local node's file bytes for a push, and describing them to the
//! adapter.
//!
//! # Why this exists
//!
//! A `raisin:Asset` created under a mirror mount is metadata plus a storage
//! key; the bytes live in the binary store, and until now nothing on the write
//! path could reach them. So a mirror over a file-shaped provider synced
//! metadata only — and the adapter contract documented `content` params on
//! `create`/`update` that the engine has never sent, which is why the Google
//! Drive adapter carries a multipart-upload path that can never execute.
//!
//! # Two shapes, and why both are needed
//!
//! Small files travel INLINE as base64 on the adapter call, which is the
//! read path's `content_base64` in reverse. Large ones cannot: the payload
//! crosses the QuickJS boundary as a JS string and is then copied twice more by
//! `JSON.stringify`/`JSON.parse`, against an adapter memory budget that is
//! typically 64 MiB in total. So above [`INLINE_CONTENT_LIMIT`] the engine
//! sends the DESCRIPTOR without the bytes and the adapter must answer with an
//! upload URL, which the engine then streams to in Rust — the exact mirror of
//! `get_content`'s `fetch_url`, and for the same reason.
//!
//! The cap is deliberately below the 64 MiB the read path allows, because the
//! write side pays the base64 and JSON costs that the read side's `fetch_url`
//! escape hatch avoids.

use base64::Engine as _;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use serde_json::{json, Value};

use super::super::adapter::AdapterError;
use super::super::ctx::SyncCtx;

/// Largest object passed to an adapter inline, as base64 inside the call.
///
/// 8 MiB of bytes is ~11 MiB of base64 and, after `JSON.stringify` and
/// `JSON.parse`, roughly three live copies inside a QuickJS heap whose whole
/// budget is 64 MiB for the ms-graph adapter. Microsoft's own simple-upload
/// ceiling is 4 MiB, so an adapter reaches ITS limit before this one — which is
/// the right order: the provider's rule should bind first, and the engine's cap
/// exists to stop an adapter with no rule of its own from taking the process
/// down.
pub(crate) const INLINE_CONTENT_LIMIT: u64 = 8 * 1024 * 1024;

/// What the engine knows about a node's bytes.
#[derive(Debug)]
pub(crate) struct OutboundContent {
    /// The adapter-facing `content` object.
    pub descriptor: Value,
    /// Where the bytes are, kept even when they were passed inline: an adapter
    /// is allowed to answer a small file with an upload URL anyway, and having
    /// to refuse that because the engine threw the locator away would be an
    /// arbitrary restriction.
    pub storage_key: String,
    pub mime_type: String,
}

/// The `file` Resource on a node, if it carries one with a usable storage key.
fn file_resource(node: &Node) -> Option<(String, Option<u64>, Option<String>, String)> {
    let PropertyValue::Resource(resource) = node.properties.get("file")? else {
        return None;
    };
    let key = match resource.metadata.as_ref()?.get("storage_key")? {
        PropertyValue::String(s) if !s.is_empty() => s.clone(),
        _ => return None,
    };
    let size = resource.size.and_then(|s| u64::try_from(s).ok());
    let mime = resource.mime_type.clone();
    // The Resource's own name is what the provider should see (it is the
    // stored file's name); the node name is the fallback for a Resource that
    // never carried one.
    let name = resource
        .name
        .clone()
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| node.name.clone());
    Some((key, size, mime, name))
}

/// Whether this node's content has not arrived yet.
///
/// An asset is routinely created in two steps: the node first, its bytes when
/// the upload finishes. A create issued in the gap has nothing to send, and a
/// provider that needs the bytes (Graph's drive create IS the byte transfer)
/// can only refuse — terminally, because the request can never succeed as
/// written. That refusal would mark the whole mount misconfigured for what is
/// actually an ordinary, self-resolving race.
///
/// So the create waits instead. It costs one skipped candidate per drain, makes
/// no provider call, and the node is created as soon as its bytes land.
///
/// `is_loaded: false` counts as pending for the same reason: the Resource
/// exists but its bytes do not.
pub(crate) fn content_pending(node: &Node, accepts_content: bool) -> bool {
    if !accepts_content {
        return false;
    }
    match node.properties.get("file") {
        Some(PropertyValue::Resource(resource)) => {
            if resource.is_loaded == Some(false) {
                return true;
            }
            // A Resource with no storage key is a placeholder — an upload that
            // has been announced and not completed.
            !resource
                .metadata
                .as_ref()
                .and_then(|m| m.get("storage_key"))
                .is_some_and(|v| matches!(v, PropertyValue::String(s) if !s.is_empty()))
        }
        // No file property at all: on a content mount this is the first half of
        // a two-step create, not a node that will never have bytes.
        _ => true,
    }
}

/// Describe (and where possible carry) a node's bytes for a create or update.
///
/// `Ok(None)` means this push has no content dimension at all — the node has no
/// file, or the adapter did not declare `accepts_content` for this mount. That
/// is the ordinary case for mail, calendars and every metadata-only mirror, and
/// it must stay indistinguishable from today's behaviour.
pub(crate) async fn outbound_content(
    retrieval: Option<&crate::jobs::handlers::package_install::BinaryRetrievalCallback>,
    node: &Node,
    accepts_content: bool,
) -> std::result::Result<Option<OutboundContent>, AdapterError> {
    if !accepts_content {
        return Ok(None);
    }
    let Some((storage_key, declared_size, mime, name)) = file_resource(node) else {
        return Ok(None);
    };
    let mime_type = mime.unwrap_or_else(|| "application/octet-stream".to_string());

    // Refused, not degraded. Creating the object at the provider from metadata
    // alone would leave an empty file that looks synced — and the next run has
    // no way to notice, because the node and the item agree on everything the
    // engine compares.
    let Some(retrieval) = retrieval else {
        return Err(AdapterError::Config(format!(
            "node '{}' carries file content but this deployment has no binary \
             retrieval wired, so the bytes cannot be read for the push",
            node.path
        )));
    };

    // The declared size decides the route BEFORE anything is read, so an
    // oversized object is never pulled into memory just to be rejected.
    let size = declared_size.unwrap_or(0);
    if size > INLINE_CONTENT_LIMIT {
        return Ok(Some(OutboundContent {
            descriptor: json!({
                "name": name,
                "mime_type": mime_type,
                "size": size,
                "inline": false,
            }),
            storage_key,
            mime_type,
        }));
    }

    let bytes = retrieval(storage_key.clone()).await.map_err(|e| {
        // Transient: a storage blip must requeue rather than mark the item bad
        // forever, and the item is still perfectly valid.
        AdapterError::Transient(format!(
            "reading file bytes for '{}' failed: {e}",
            node.path
        ))
    })?;

    // The stored size is metadata and can be stale; what is about to cross the
    // boundary is what matters, so the limit is re-checked against reality.
    let actual = bytes.len() as u64;
    if actual > INLINE_CONTENT_LIMIT {
        return Ok(Some(OutboundContent {
            descriptor: json!({
                "name": name,
                "mime_type": mime_type,
                "size": actual,
                "inline": false,
            }),
            storage_key,
            mime_type,
        }));
    }

    Ok(Some(OutboundContent {
        descriptor: json!({
            "name": name,
            "mime_type": mime_type,
            "size": actual,
            "inline": true,
            "content_base64": base64::engine::general_purpose::STANDARD.encode(&bytes),
        }),
        storage_key,
        mime_type,
    }))
}

/// Call the adapter for a content-capable push, completing a deferred transfer
/// if it asks for one.
///
/// The two-step shape is what keeps provider knowledge in the adapter: the
/// engine moves bytes, and the adapter says what the provider made of them.
/// Parsing the upload response here would put a driveItem's field names in the
/// sync engine, which is the one thing the adapter boundary exists to prevent.
pub(crate) async fn call_with_content(
    ctx: &SyncCtx<'_>,
    operation: &str,
    mut params: Value,
    node: &Node,
    accepts_content: bool,
    item_id: Option<&str>,
) -> std::result::Result<Value, AdapterError> {
    let content = outbound_content(ctx.binary_retrieval.as_ref(), node, accepts_content).await?;
    if let Some(content) = content.as_ref() {
        if let Some(obj) = params.as_object_mut() {
            obj.insert("content".to_string(), content.descriptor.clone());
        }
    }

    let result = ctx.call(operation, params).await?;

    let Some(request) = super::upload::UploadRequest::from_adapter_value(&result) else {
        // An adapter that was handed a descriptor with `inline: false` and did
        // NOT ask for an upload has silently dropped the bytes — it would
        // create an empty object at the provider that looks synced forever.
        if let Some(content) = content.as_ref() {
            if content.descriptor.get("inline").and_then(|v| v.as_bool()) == Some(false) {
                return Err(AdapterError::Config(format!(
                    "'{}' is {} bytes, over the {INLINE_CONTENT_LIMIT}-byte inline ceiling,                      and the adapter answered without an upload url — it cannot store this                      file's content",
                    node.path,
                    content
                        .descriptor
                        .get("size")
                        .and_then(|v| v.as_u64())
                        .unwrap_or_default(),
                )));
            }
        }
        return Ok(result);
    };

    let Some(content) = content else {
        return Err(AdapterError::Config(format!(
            "adapter asked to upload bytes for '{}', which carries no file content",
            node.path
        )));
    };

    let retrieval = ctx.binary_retrieval.as_ref().ok_or_else(|| {
        AdapterError::Config("no binary retrieval wired for an upload".to_string())
    })?;
    let bytes = retrieval(content.storage_key.clone())
        .await
        .map_err(|e| AdapterError::Transient(format!("reading bytes to upload failed: {e}")))?;

    let outcome = super::upload::upload_bytes(&request, bytes, &content.mime_type)
        .await
        .map_err(|e| AdapterError::Transient(format!("upload failed: {e}")))?;

    ctx.call(
        "finalize_upload",
        json!({
            "status": outcome.status,
            "body": outcome.body,
            "intent": operation,
            "item_id": item_id,
        }),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_models::nodes::properties::value::Resource;
    use std::collections::HashMap;

    fn retrieval_of(
        bytes: Vec<u8>,
    ) -> crate::jobs::handlers::package_install::BinaryRetrievalCallback {
        std::sync::Arc::new(move |_key: String| {
            let bytes = bytes.clone();
            Box::pin(async move { Ok(bytes) })
                as std::pin::Pin<
                    Box<dyn std::future::Future<Output = raisin_error::Result<Vec<u8>>> + Send>,
                >
        })
    }

    fn node_with_file(size: Option<i64>, key: &str) -> Node {
        let mut node = Node {
            id: "n1".into(),
            node_type: "raisin:Asset".into(),
            name: "photo.jpg".into(),
            path: "/drives/onedrive/photo.jpg".into(),
            ..Default::default()
        };
        let mut metadata = HashMap::new();
        metadata.insert(
            "storage_key".to_string(),
            PropertyValue::String(key.to_string()),
        );
        let resource = Resource {
            uuid: "u".to_string(),
            name: Some("photo.jpg".to_string()),
            size,
            mime_type: Some("image/jpeg".to_string()),
            url: None,
            metadata: Some(metadata),
            is_loaded: Some(true),
            is_external: Some(false),
            created_at: chrono::Utc::now().into(),
            updated_at: chrono::Utc::now().into(),
        };
        node.properties
            .insert("file".to_string(), PropertyValue::Resource(resource));
        node
    }

    /// The default has to stay exactly what every adapter shipped before the
    /// content channel existed did: nothing.
    #[tokio::test]
    async fn an_adapter_that_did_not_opt_in_is_never_offered_content() {
        let node = node_with_file(Some(10), "k");
        let out = outbound_content(Some(&retrieval_of(vec![1, 2, 3])), &node, false)
            .await
            .unwrap();
        assert!(out.is_none());
    }

    #[tokio::test]
    async fn a_node_with_no_file_has_no_content_dimension() {
        let node = Node {
            id: "n2".into(),
            node_type: "raisin:Event".into(),
            name: "event".into(),
            ..Default::default()
        };
        let out = outbound_content(Some(&retrieval_of(vec![])), &node, true)
            .await
            .unwrap();
        assert!(out.is_none());
    }

    #[tokio::test]
    async fn a_small_file_travels_inline_as_base64() {
        let node = node_with_file(Some(3), "k");
        let out = outbound_content(Some(&retrieval_of(vec![1, 2, 3])), &node, true)
            .await
            .unwrap()
            .expect("content");
        assert_eq!(out.descriptor["inline"], true);
        assert_eq!(out.descriptor["content_base64"], "AQID");
        assert_eq!(out.descriptor["size"], 3);
        assert_eq!(out.descriptor["mime_type"], "image/jpeg");
        assert_eq!(out.descriptor["name"], "photo.jpg");
        assert_eq!(out.storage_key, "k");
    }

    /// Decided from the DECLARED size, so an oversized object is never pulled
    /// into memory just to be rejected.
    #[tokio::test]
    async fn an_oversized_file_is_described_but_not_carried() {
        let node = node_with_file(Some((INLINE_CONTENT_LIMIT + 1) as i64), "k");
        // A retrieval that would panic proves the bytes are never read.
        let never: crate::jobs::handlers::package_install::BinaryRetrievalCallback =
            std::sync::Arc::new(|_key: String| {
                Box::pin(async move {
                    panic!("bytes must not be read for a deferred upload");
                    #[allow(unreachable_code)]
                    Ok(Vec::new())
                })
                    as std::pin::Pin<
                        Box<dyn std::future::Future<Output = raisin_error::Result<Vec<u8>>> + Send>,
                    >
            });
        let out = outbound_content(Some(&never), &node, true)
            .await
            .unwrap()
            .expect("content");
        assert_eq!(out.descriptor["inline"], false);
        assert!(out.descriptor.get("content_base64").is_none());
        assert_eq!(out.descriptor["size"], INLINE_CONTENT_LIMIT + 1);
    }

    /// A stale `size` must not become a way past the ceiling.
    #[tokio::test]
    async fn the_limit_is_re_checked_against_the_bytes_that_actually_arrived() {
        let node = node_with_file(Some(3), "k");
        let big = vec![0u8; (INLINE_CONTENT_LIMIT + 1) as usize];
        let out = outbound_content(Some(&retrieval_of(big)), &node, true)
            .await
            .unwrap()
            .expect("content");
        assert_eq!(out.descriptor["inline"], false);
        assert!(out.descriptor.get("content_base64").is_none());
    }

    /// A node whose upload has not finished is not ready to be created.
    #[tokio::test]
    async fn a_two_step_upload_defers_its_create_instead_of_failing_the_mount() {
        // Step one: the node exists, the bytes do not.
        let mut announced = node_with_file(Some(0), "");
        let PropertyValue::Resource(resource) = announced.properties.get_mut("file").unwrap()
        else {
            unreachable!()
        };
        resource.is_loaded = Some(false);
        assert!(content_pending(&announced, true));

        // A node with no file property at all is the same case, earlier.
        let bare = Node {
            id: "n3".into(),
            node_type: "raisin:Asset".into(),
            name: "later.pdf".into(),
            ..Default::default()
        };
        assert!(content_pending(&bare, true));

        // Step two: the bytes landed.
        assert!(!content_pending(&node_with_file(Some(3), "k"), true));

        // And a mount that does not carry content is never made to wait.
        assert!(!content_pending(&bare, false));
    }

    /// Refused rather than degraded: a create from metadata alone would leave
    /// an empty file at the provider that looks synced forever.
    #[tokio::test]
    async fn no_retrieval_wired_is_a_refusal_not_a_metadata_only_push() {
        let node = node_with_file(Some(3), "k");
        let err = outbound_content(None, &node, true).await.unwrap_err();
        assert!(
            matches!(err, AdapterError::Config(ref m) if m.contains("binary retrieval")),
            "got {err:?}"
        );
    }
}
