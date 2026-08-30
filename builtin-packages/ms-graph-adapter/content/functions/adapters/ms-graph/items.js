// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! One provider object to one `ExternalItem`, per resource.

import { mailMeta } from "./mail.js";
import { calendarMeta } from "./calendar.js";

// A driveItem's facets, resolved through `remoteItem` when it has one.
//
// An item added with "Add to my OneDrive" — a shared folder or file living in
// SOMEONE ELSE'S drive — is returned as a shortcut: the top level carries only
// `remoteItem`, and the `folder` / `file` / `size` facets that say what the thing
// actually IS are nested inside it. Reading only the top level made every such
// shortcut look like a leaf with no file facet, so a shared folder was stored as
// a zero-byte file and its entire subtree was never walked.
//
// The remote drive id travels too: recursing into a shortcut means addressing a
// different drive than the mount's own, and `parentReference.driveId` on the
// remoteItem is the only place that id appears.
function driveFacets(v) {
  var r = v.remoteItem || null;
  var folder = v.folder || (r && r.folder) || null;
  var file = v.file || (r && r.file) || null;
  var size = v.size != null ? v.size : r && r.size != null ? r.size : null;
  return {
    folder: folder,
    file: file,
    size: size,
    remoteDriveId: r && r.parentReference ? r.parentReference.driveId || null : null,
    remoteId: r ? r.id || null : null,
  };
}

// OneDrive driveItem: is_folder from the `folder` facet, mime_type/size from the
// `file` facet. The real filename lives in metadata.filename (name = id, below).
export function filesMeta(v) {
  var f = driveFacets(v);
  return {
    filename: v.name || null,
    is_folder: !!f.folder,
    mime_type: f.file && f.file.mimeType ? f.file.mimeType : null,
    size_bytes: f.size,
    parent_id: v.parentReference ? v.parentReference.id || null : null,
    parent_path: v.parentReference ? v.parentReference.path || null : null,
    child_count: f.folder && f.folder.childCount != null ? f.folder.childCount : null,
    download_url: v["@microsoft.graph.downloadUrl"] || null,
    web_url: v.webUrl || null,
    ctag: v.cTag || null,
    // Null for an ordinary item. Present means "this node stands for something
    // in another drive", which is what `opList` needs to recurse into it.
    remote_drive_id: f.remoteDriveId,
    remote_item_id: f.remoteId,
  };
}

// external_id is ALWAYS the Graph item id, on every resource — it is what keys
// the node for its whole lifetime. All provider fields live in `metadata`, which
// the engine carries verbatim onto node properties.
//
// `name` is the id too for MAIL and CALENDAR, where it is the path segment an
// item materializes at and a subject would collide constantly. For FILES it is
// the FILENAME, and that difference is load-bearing rather than cosmetic — see
// the files branch below.
// `folderPath` is TREE MODE ONLY, and only for mail: the resolved folder chain
// this message materializes under, `""` at the mount root. Anything else passes
// it as undefined and gets today's item back byte for byte.
export function toExternalItem(v, resource, mount, folderPath) {
  var id = v.id;
  var item = {
    external_id: id,
    name: id,
    mime_type: null,
    size_bytes: null,
    is_folder: false,
    parent_id: null,
    created_at: v.createdDateTime || null,
    modified_at: v.lastModifiedDateTime || null,
    etag: v["@odata.etag"] || v.eTag || v.lastModifiedDateTime || null,
    web_url: v.webLink || v.webUrl || null,
    download_url: null,
    metadata: null,
  };
  if (resource === "files") {
    var f = driveFacets(v);
    // THE PATH LEAF, and the reason a drive mount lays out the way it does.
    //
    // The engine builds a full walk's path as `{parent prefix}/{item.name}`
    // (`full.rs` `resolve_item_path`), and the prefix is each ancestor FOLDER's
    // path built the same way — so `name` alone decides the layout. With the id
    // here, a drive materialized as a tree of opaque ids, and worse, the delta
    // feed could never agree with it: Graph gives a changed item its ancestry
    // only as `parentReference.path`, which is NAMES. So a webhook-delivered
    // file had no way to reconstruct the walk's path and landed flat at the
    // mount root, while the backfill had placed its siblings correctly.
    //
    // Two files with the same name in one folder still cannot collide: the
    // engine disambiguates a repeated path with an external-id suffix
    // (`materializer/ops.rs` `suffix_path`). Node identity is untouched — that
    // is `external_id`, which stays the Graph id.
    item.name = v.name || id;
    item.is_folder = !!f.folder;
    item.mime_type = f.file && f.file.mimeType ? f.file.mimeType : null;
    item.size_bytes = f.size;
    item.parent_id = v.parentReference ? v.parentReference.id || null : null;
    item.download_url = v["@microsoft.graph.downloadUrl"] || null;
    item.metadata = filesMeta(v);
  } else if (resource === "calendar") {
    item.metadata = calendarMeta(v, mount);
  } else {
    item.metadata = mailMeta(v);
    if (typeof folderPath === "string") {
      // THE FOLDER PATH IS FOLDED INTO THE ETAG, AND IT HAS TO BE.
      //
      // `batch.rs` `can_skip_unmapped` compares external_id + etag and RETURNS
      // BEFORE rel_path is ever consulted — on the full walk as well as the
      // delta. Renaming an Outlook folder changes no message's `@odata.etag`,
      // so without this every message in that folder is skipped as unchanged
      // and stays at its OLD path forever: a full walk does not repair it
      // either, only `force_rewrite` does.
      //
      // Folder mode keeps the bare provider etag, so no existing mount
      // re-writes a single node because this exists.
      if (item.etag) item.etag = item.etag + "|p=" + folderPath;
      item.metadata.folder_id = v.parentFolderId || null;
      // What the MAPPER reads for raisin:Mail's `folder`, instead of
      // `mount.remote_root` — a mount-level constant that is simply wrong once
      // one mount spans many folders.
      item.metadata.folder_path = folderPath;
    }
  }
  return item;
}
