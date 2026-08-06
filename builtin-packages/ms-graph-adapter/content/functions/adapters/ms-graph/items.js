// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! One provider object to one `ExternalItem`, per resource.

import { mailMeta } from "./mail.js";
import { calendarMeta } from "./calendar.js";

// OneDrive driveItem: is_folder from the `folder` facet, mime_type/size from the
// `file` facet. The real filename lives in metadata.filename (name = id, below).
export function filesMeta(v) {
  return {
    filename: v.name || null,
    is_folder: !!v.folder,
    mime_type: v.file && v.file.mimeType ? v.file.mimeType : null,
    size_bytes: v.size != null ? v.size : null,
    parent_id: v.parentReference ? v.parentReference.id || null : null,
    parent_path: v.parentReference ? v.parentReference.path || null : null,
    child_count: v.folder && v.folder.childCount != null ? v.folder.childCount : null,
    download_url: v["@microsoft.graph.downloadUrl"] || null,
    web_url: v.webUrl || null,
    ctag: v.cTag || null,
  };
}

// external_id / name / relative_path are ALWAYS the Graph item id — never the
// subject/title/filename — so distinct items never collide on a path. All provider
// fields live in `metadata`, which the engine carries verbatim onto node properties.
export function toExternalItem(v, resource, mount) {
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
    item.is_folder = !!v.folder;
    item.mime_type = v.file && v.file.mimeType ? v.file.mimeType : null;
    item.size_bytes = v.size != null ? v.size : null;
    item.parent_id = v.parentReference ? v.parentReference.id || null : null;
    item.download_url = v["@microsoft.graph.downloadUrl"] || null;
    item.metadata = filesMeta(v);
  } else if (resource === "calendar") {
    item.metadata = calendarMeta(v, mount);
  } else {
    item.metadata = mailMeta(v);
  }
  return item;
}
