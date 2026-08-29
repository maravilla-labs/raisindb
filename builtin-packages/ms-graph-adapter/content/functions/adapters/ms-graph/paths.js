// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Where a drive item materializes, relative to the MOUNT ROOT.
//!
//! Only the delta feed needs this. The full walk recurses folder by folder and
//! the ENGINE accumulates the prefix as it descends (`full.rs`
//! `resolve_item_path`: `{prefix}/{item.name}`, where the prefix is the parent
//! folder's own resolved path). The delta feed has no recursion — it hands back
//! a flat page of items from anywhere in the subtree — so the path has to be
//! reconstructed from what Graph puts on each item, and it has to come out
//! IDENTICAL to what the walk produces or the same file sits in one place after
//! a backfill and another after a webhook. That is not hypothetical: it shipped,
//! and two files delivered by webhook landed flat at the mount root while the
//! backfill had placed their siblings correctly.

import { enc } from "./common.js";
import { GRAPH, graphFetch } from "./http.js";
import { driveBase, driveRoot } from "./mount.js";

// Graph writes a driveItem's parent as `/drive/root:/A/B` or
// `/drives/{driveId}/root:/A/B`. Everything up to and including the `root:`
// marker is DRIVE ADDRESSING, not path, and it differs between the two forms —
// so the marker is what the split keys on rather than a fixed segment count.
var ROOT_MARKER = /^.*?\/root:/;

// Graph PERCENT-ENCODES path segments, so a folder named "Maravilla
// Accelerator" arrives as "Maravilla%20Accelerator" — and materializing it that
// way puts the file in a folder whose name no human typed and the walk will
// never produce.
//
// A segment that is not valid encoding is kept verbatim rather than dropped: a
// literal `%` in a filename is legal, and a wrong-looking name is a far better
// outcome than losing the segment and moving the file up a level.
function decodeSegment(s) {
  try {
    return decodeURIComponent(s);
  } catch (e) {
    return s;
  }
}

// The folder chain a `parentReference.path` names, decoded.
//
// Returns `[]` for an item sitting directly in the drive root, and `null` when
// there is no usable path at all — which the caller must treat as "unknown",
// never as "the root".
export function pathSegments(parentPath) {
  if (typeof parentPath !== "string" || !parentPath) return null;
  var m = ROOT_MARKER.exec(parentPath);
  if (!m) return null;
  var rest = parentPath.slice(m[0].length);
  var raw = rest.split("/");
  var out = [];
  for (var i = 0; i < raw.length; i++) {
    if (raw[i]) out.push(decodeSegment(raw[i]));
  }
  return out;
}

// The mount root's own chain, so item paths can be made relative to it.
//
// A mount whose `remote_root` names a subfolder starts its walk INSIDE that
// folder, with an empty prefix — so the folder's own path has to come off every
// item, or every delta path would carry it and disagree with the walk by one
// segment (or several).
//
// Costs ONE request per `get_changes`, and only for a mount that has a
// remote_root; a drive-root mount answers `[]` without touching the network.
// Returns `null` when the root cannot be resolved, which the caller reads as
// "do not attempt a relative path" rather than guessing.
export function mountRootSegments(credential, mount) {
  var root = driveRoot(mount);
  if (!root) return [];
  var url =
    GRAPH + driveBase(mount) + "/items/" + enc(root) + "?$select=name,parentReference";
  var resp = graphFetch(credential, "GET", url, { context: "get_changes:mount_root" });
  var v = resp.body || {};
  var segs = pathSegments(v.parentReference && v.parentReference.path);
  if (segs === null || !v.name) return null;
  return segs.concat([v.name]);
}

// Whether this delta entry is the mount's own root container rather than
// something inside it.
//
// Graph's `/delta` always reports the container it is scoped to as an item.
// Emitting it created a stray `root` folder node AT the mount root — a folder
// standing for the mount itself, inside itself.
export function isMountRootContainer(v, mount) {
  // The drive root carries a `root` facet and nothing else does.
  if (v && v.root) return true;
  var root = driveRoot(mount);
  return Boolean(root && v && v.id === root);
}

// One item's path relative to the mount root, or `null` to SKIP it.
//
// Null means the item lives outside the mount root. Skipping is the only safe
// answer: a path computed from a chain that does not start with the root would
// escape the mount, and the engine joins it to `mount_path` verbatim.
export function filesRelativePath(v, rootSegments) {
  var name = v.name || v.id;
  var segs = pathSegments(v.parentReference && v.parentReference.path);
  // No parent path on this item shape (a `remoteItem` shortcut carries its own
  // parent in another drive), or an unresolvable mount root. Fall back to the
  // bare name rather than inventing a folder chain: landing a file at the mount
  // root is recoverable, and materializing it under a folder that does not
  // exist is not.
  if (segs === null || rootSegments === null) return name;
  for (var i = 0; i < rootSegments.length; i++) {
    if (segs[i] !== rootSegments[i]) return null;
  }
  return segs.slice(rootSegments.length).concat([name]).join("/");
}
