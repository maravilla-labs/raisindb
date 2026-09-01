// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! What a binary asset IS, read from its node properties.
//!
//! # Why these live here and not next to the job that uses them
//!
//! They were `pub(crate)` helpers of the asset-processing job in
//! `raisin-rocksdb`, and the module comment there already recorded why they had
//! been consolidated once: the enqueue gate and the processor had each carried
//! their own narrower spelling, so an asset whose mime type sat in
//! `contentType` was invisible to one and visible to the other.
//!
//! A THIRD caller has now appeared, and it cannot reach a `pub(crate)` item.
//! [`ExtractionArtifact`](super::extraction::ExtractionArtifact) can be written
//! by a plugin task running in the function layer — LibreOffice converting a
//! `.docx` to markdown, which no code in this process can do — and that
//! writeback has to stamp the SAME [`asset_fingerprint`] the job would have.
//!
//! The fingerprint is the loop-breaker: the enqueue gate skips a node whose
//! stamp matches its binary. A writeback that computed the stamp even slightly
//! differently would never match, so every write-back would re-enqueue
//! extraction, which would hand off again, which would write back again —
//! an unbounded loop minting a node revision, a fulltext reindex and an
//! embedding call each time round. Re-deriving it at a second site is the
//! failure mode; there is one implementation and everything calls it.

use std::collections::HashMap;

use crate::nodes::properties::PropertyValue;
use crate::nodes::Node;

/// Read a `PropertyValue` as a string, or `None`.
fn as_str(pv: &PropertyValue) -> Option<String> {
    match pv {
        PropertyValue::String(s) => Some(s.clone()),
        _ => None,
    }
}

/// Extract the content hash of the binary from node properties.
///
/// Looked for in the `file` Resource/Object metadata, then as a top-level
/// `content_hash` property (which `raisin:Asset` declares and the package
/// installer and the on-demand attachment fetch both set).
pub fn asset_content_hash(properties: &HashMap<String, PropertyValue>) -> Option<String> {
    if let Some(PropertyValue::Resource(res)) = properties.get("file") {
        if let Some(h) = res
            .metadata
            .as_ref()
            .and_then(|m| m.get("content_hash"))
            .and_then(as_str)
        {
            return Some(h);
        }
    }
    if let Some(PropertyValue::Object(obj)) = properties.get("file") {
        if let Some(PropertyValue::Object(meta)) = obj.get("metadata") {
            if let Some(h) = meta.get("content_hash").and_then(as_str) {
                return Some(h);
            }
        }
        if let Some(h) = obj.get("content_hash").and_then(as_str) {
            return Some(h);
        }
    }
    properties.get("content_hash").and_then(as_str)
}

/// Extract the storage key of the binary from node properties.
///
/// `None` rather than an error: a node with no key is an asset whose file is
/// not ready, which every caller treats as "nothing to do yet" rather than as a
/// failure.
pub fn asset_storage_key(properties: &HashMap<String, PropertyValue>) -> Option<String> {
    // Standard upload format (Resource type).
    if let Some(PropertyValue::Resource(resource)) = properties.get("file") {
        if let Some(ref metadata) = resource.metadata {
            if let Some(PropertyValue::String(key)) = metadata.get("storage_key") {
                return Some(key.clone());
            }
            if let Some(PropertyValue::String(key)) = metadata.get("storageKey") {
                return Some(key.clone());
            }
        }
    }

    // Legacy Object format.
    if let Some(PropertyValue::Object(obj)) = properties.get("file") {
        if let Some(PropertyValue::String(key)) = obj.get("storage_key") {
            return Some(key.clone());
        }
        if let Some(PropertyValue::String(key)) = obj.get("storageKey") {
            return Some(key.clone());
        }
        if let Some(PropertyValue::Object(metadata)) = obj.get("metadata") {
            if let Some(PropertyValue::String(key)) = metadata.get("storage_key") {
                return Some(key.clone());
            }
        }
    }

    // Package format.
    if let Some(PropertyValue::Resource(resource)) = properties.get("resource") {
        if let Some(ref metadata) = resource.metadata {
            if let Some(PropertyValue::String(key)) = metadata.get("storage_key") {
                return Some(key.clone());
            }
        }
    }

    None
}

/// Extract the mime type of the binary from node properties.
pub fn asset_mime_type(properties: &HashMap<String, PropertyValue>) -> Option<String> {
    if let Some(PropertyValue::Resource(resource)) = properties.get("file") {
        if let Some(ref mime) = resource.mime_type {
            return Some(mime.clone());
        }
        if let Some(ref metadata) = resource.metadata {
            if let Some(PropertyValue::String(mime)) = metadata.get("mime_type") {
                return Some(mime.clone());
            }
            if let Some(PropertyValue::String(mime)) = metadata.get("mimeType") {
                return Some(mime.clone());
            }
        }
    }

    if let Some(PropertyValue::Object(obj)) = properties.get("file") {
        if let Some(PropertyValue::String(mime)) = obj.get("mime_type") {
            return Some(mime.clone());
        }
        if let Some(PropertyValue::String(mime)) = obj.get("mimeType") {
            return Some(mime.clone());
        }
    }

    if let Some(PropertyValue::String(ct)) = properties.get("contentType") {
        return Some(ct.clone());
    }

    if let Some(PropertyValue::String(mt)) = properties.get("mimeType") {
        return Some(mt.clone());
    }

    None
}

/// Identify the BINARY an extraction result was produced from.
///
/// Extraction terminates in a node property, and writing a node property emits
/// `node:updated` — which is the very event that enqueues asset processing. So
/// the write-back would re-trigger the job, which would write again, forever.
/// The job (and any delegated task writing back through the function layer)
/// stamps this fingerprint alongside the text, and the enqueue gate skips a node
/// whose stamp already matches. One upload therefore extracts exactly once, and
/// REPLACING the file (new content hash, new storage key, or new size) changes
/// the fingerprint and re-extracts.
///
/// A revision counter or `updated_by` would not do: replication delivers node
/// revisions written by another node's system actor, and a mount sync stamps its
/// own. Only "which bytes was this text made from" answers the question that is
/// actually being asked.
///
/// The `v1:` prefix is the extractor generation. Bump it to force every asset to
/// be re-extracted after a change that makes old output wrong.
pub fn asset_fingerprint(node: &Node) -> String {
    // A MOUNTED asset is identified by the PROVIDER, not by our copy of it.
    //
    // The local triple below describes the cached bytes, which for a mount is a
    // cache of someone else's system of record — so it answers the wrong
    // question twice. Evicting the cache would change the fingerprint and
    // re-open the processing gate, re-downloading and re-extracting a file that
    // has not changed (and evicting again, forever). And a file EDITED in the
    // provider whose size happened to stay the same would not change it at all,
    // so the edit would never be re-read.
    //
    // The provider already answers both: `__etag` is its own version marker.
    if let Some(fp) = mount_fingerprint(&node.properties) {
        return fp;
    }

    let hash = asset_content_hash(&node.properties).unwrap_or_else(|| "-".to_string());
    let key = asset_storage_key(&node.properties).unwrap_or_else(|| "-".to_string());
    let size = match node.properties.get("file_size") {
        Some(PropertyValue::Integer(n)) => n.to_string(),
        Some(PropertyValue::Float(f)) => f.to_string(),
        _ => "-".to_string(),
    };
    format!("v1:{hash}|{key}|{size}")
}

/// Whether this asset's bytes live on a virtual mount and can still be fetched.
///
/// A mount's sync writes METADATA ONLY — a `raisin:Asset` per remote file with
/// its name, mimetype and size, and no bytes — because downloading a whole
/// drive during a sync would multiply an import by every document in it. So
/// `file == null` on a mount-owned asset means "not fetched yet", NOT "broken"
/// or "not ready", and the two must not be confused: the first is repairable by
/// asking the provider, the second is not.
///
/// `__external_id` is what makes it repairable — it is the provider's own
/// identifier for the file, and without it there is nothing to ask for.
pub fn is_fetchable_mount_content(properties: &HashMap<String, PropertyValue>) -> bool {
    let mounted = matches!(
        properties.get("__virtual"),
        Some(PropertyValue::Boolean(true))
    );
    let identified = matches!(
        properties.get("__external_id"),
        Some(PropertyValue::String(id)) if !id.is_empty()
    );
    mounted && identified
}

/// The size the PROVIDER reports for a mount-owned asset, before any fetch.
///
/// Read from the sync's own metadata (`size`, or `file_size` once hydrated), so
/// a decision about whether the bytes are worth downloading can be made without
/// downloading them.
pub fn asset_reported_size(properties: &HashMap<String, PropertyValue>) -> Option<u64> {
    for key in ["size", "file_size"] {
        match properties.get(key) {
            Some(PropertyValue::Integer(n)) if *n >= 0 => return Some(*n as u64),
            Some(PropertyValue::Float(f)) if *f >= 0.0 => return Some(*f as u64),
            _ => {}
        }
    }
    None
}

/// Identity of a mount-owned asset, from the provider's own metadata.
///
/// `None` for anything not mounted, or mounted without an etag — a provider
/// that reports no version leaves us nothing better than the local copy, so the
/// caller falls back to it rather than inventing a constant that would freeze
/// the gate shut.
fn mount_fingerprint(properties: &HashMap<String, PropertyValue>) -> Option<String> {
    if !is_fetchable_mount_content(properties) {
        return None;
    }
    let PropertyValue::String(external) = properties.get("__external_id")? else {
        return None;
    };
    let PropertyValue::String(etag) = properties.get("__etag")? else {
        return None;
    };
    if etag.is_empty() {
        return None;
    }
    Some(format!("v1:mount|{external}|{etag}"))
}
