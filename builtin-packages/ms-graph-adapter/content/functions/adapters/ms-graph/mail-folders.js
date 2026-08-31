// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! FOLDER IDENTITY for a mail TREE mount: the id -> {name, parentId} map, and
//! the one function that turns a folder id into the path chain a message
//! materializes under.
//!
//! This module is to a mail tree what `paths.js` is to a drive, and it exists
//! for the same reason: the full walk recurses folder by folder and the ENGINE
//! accumulates the prefix (`full.rs` `resolve_item_path`: `{prefix}/{name}`),
//! while the delta feed hands back a flat page from anywhere in the subtree. If
//! the two disagree by so much as one segment, the same message sits in one
//! place after a backfill and another after a webhook — and the engine relocates
//! the node on EVERY run where they disagree.
//!
//! So the walk and the delta call the SAME two functions here — `folderSegment`
//! and `mailRelativeChain` — and there is no second spelling of either.

import { coded, enc } from "./common.js";
import { GRAPH, graphFetch, raiseForStatus } from "./http.js";
import { mailFolderId, maxFolders, principal } from "./mount.js";

// What a mailFolder listing must return for both paths to work: `id` and
// `displayName` build the segment, `parentFolderId` builds the chain, and
// `isHidden` travels so an operator can see why Clutter appeared.
//
// `childFolderCount` is deliberately NOT here. It was used to skip listing a
// childless folder's children, which made this map's folder set a different one
// from the walk's; see buildFolderMap. Selecting a field nothing reads is how
// the next reader concludes the shortcut is still in force.
var FOLDER_SELECT = "id,displayName,parentFolderId,totalItemCount,isHidden";

// Page size for a folder listing. Folders are small objects and a mailbox with
// hundreds of them is the case this is bounded for.
var FOLDER_PAGE = 100;

// Depth cap on a chain walk. Outlook has no such limit, but a map that somehow
// contained a cycle would spin forever inside the sync hot loop; a bounded walk
// answers `null` (skip) instead, which is the same answer as "outside the mount".
var MAX_DEPTH = 64;

// ONE sanitizer for a folder's display name, used by the walk AND the delta.
//
// A folder called "R&D/Legal" is one Outlook folder with a slash in its name.
// Left alone it would mean TWO path segments on one side of the comparison and
// one on the other, so the walk and the delta would place its messages in
// different places forever. Control characters are stripped for the same
// reason: they are not addressable in a workspace path.
//
// `.` and `..` are rejected outright: a folder an Outlook user is perfectly
// entitled to name ".." would otherwise produce a relative_path the engine joins
// to mount_path verbatim and that walks OUT of the mount.
//
// The fallback is the folder ID rather than a fixed word: two folders that both
// sanitize to nothing must not collapse onto one path.
export function folderSegment(displayName, folderId) {
  var s = typeof displayName === "string" ? displayName : "";
  var out = "";
  for (var i = 0; i < s.length; i++) {
    var c = s.charAt(i);
    var code = s.charCodeAt(i);
    if (c === "/" || c === "\\" || code < 32 || code === 127) {
      out += "-";
    } else {
      out += c;
    }
  }
  out = out.replace(/^\s+|\s+$/g, "");
  if (!out.length || out === "." || out === "..") return String(folderId || "folder");
  return out;
}

// One page-following listing of a folder's children, hidden folders INCLUDED.
//
// Hidden folders (Clutter and friends) are excluded by default. Excluding them
// from the walk while their messages still arrive from... nowhere, because a
// tree mount only subscribes to folders it knows about, means those messages
// are invisible with nothing to observe. `includeHiddenFolders=true` is the
// documented opt-in (learn.microsoft.com list-childFolders).
export function listChildFolders(credential, mount, folderId) {
  var url =
    GRAPH + principal(mount) + "/mailFolders/" + enc(folderId) +
    "/childFolders?includeHiddenFolders=true&$top=" + FOLDER_PAGE +
    "&$select=" + enc(FOLDER_SELECT);
  var out = [];
  while (url) {
    var resp = graphFetch(credential, "GET", url, { context: "mail_folders:children" });
    raiseForStatus(resp, "mail_folders:children");
    var body = resp.body || {};
    var values = body.value || [];
    for (var i = 0; i < values.length; i++) out.push(values[i]);
    url = body["@odata.nextLink"] || null;
  }
  return out;
}

// The mount root's REAL folder id.
//
// `mount.remote_root` is often a WELL-KNOWN NAME — "inbox" is the default — and
// Graph accepts those in a URL but never returns one: a message in the Inbox
// carries `parentFolderId` = the inbox's real id. Resolving the chain against
// the literal string "inbox" therefore matched nothing, and every message
// directly in the mount root was skipped by the delta as "outside the mount".
// One request, once per call, is what closes that.
export function resolveRootFolder(credential, mount) {
  var id = mailFolderId(mount);
  var resp = graphFetch(
    credential,
    "GET",
    GRAPH + principal(mount) + "/mailFolders/" + enc(id) + "?$select=" + enc(FOLDER_SELECT),
    { context: "mail_folders:root", rawStatusOk: true }
  );
  if (resp.status === 404) {
    throw coded(
      "mail folder '" + id + "' does not exist in this mailbox",
      "config_error"
    );
  }
  raiseForStatus(resp, "mail_folders:root");
  var v = resp.body || {};
  if (!v.id) {
    throw coded(
      "Microsoft Graph returned no id for mail folder '" + id + "'",
      "config_error"
    );
  }
  return v;
}

// The whole subtree under the mount root as `{id: {name, parentId, hidden}}`,
// plus the root's resolved id.
//
// Costs one request for the root plus one PER FOLDER — deliberately, even for a
// folder Graph reports as childless. Skipping on `childFolderCount === 0` made
// the delta's folder discovery a DIFFERENT rule from the walk's, which calls
// listChildFolders unconditionally: this adapter passes includeHiddenFolders
// because hidden folders are absent from a default listing, so a count derived
// from that same default view (or merely stale) hides a whole subtree from the
// delta while the walk materializes it — messages there would arrive only after
// a complete full walk. The walk and the delta must discover the same folders or
// the mount relocates nodes; that asymmetry has already cost this codebase twice.
// The extra requests are one per leaf per ROUND, not per poll, since the map is
// cached in the cursor.
//
// THROWS above `max_folders` rather than truncating. A truncated folder set is a
// partial `seen` set for the full walk, and `reconcile_deletes` would prune every
// message in the folders that fell off the end. Refusing to sync is recoverable;
// deleting real content is not.
export function buildFolderMap(credential, mount) {
  var root = resolveRootFolder(credential, mount);
  var rootId = root.id;
  var limit = maxFolders(mount);
  var map = {};
  // Breadth-first, so the `max_folders` ceiling trips on the widest level rather
  // than after descending one arbitrary branch to its leaf.
  var queue = [{ id: rootId }];
  var count = 0;
  while (queue.length) {
    var cur = queue.shift();
    var children = listChildFolders(credential, mount, cur.id);
    for (var i = 0; i < children.length; i++) {
      var f = children[i];
      if (!f || !f.id) continue;
      count += 1;
      if (count > limit) {
        throw coded(
          "this mailbox has more than max_folders (" + limit + ") folders under '" +
            mailFolderId(mount) + "'; refusing to sync a TRUNCATED folder set, " +
            "because a partial listing would make the walk's reconcile delete " +
            "every message in the folders that did not fit",
          "config_error"
        );
      }
      map[f.id] = {
        name: folderSegment(f.displayName, f.id),
        parentId: f.parentFolderId || cur.id,
        hidden: f.isHidden === true,
        display_name: f.displayName || null,
        total_item_count: f.totalItemCount != null ? f.totalItemCount : null,
      };
      queue.push({ id: f.id });
    }
  }
  return { rootId: rootId, map: map };
}

// One folder by id, or null when it is gone.
export function fetchFolder(credential, mount, folderId) {
  var resp = graphFetch(
    credential,
    "GET",
    GRAPH + principal(mount) + "/mailFolders/" + enc(folderId) +
      "?$select=" + enc(FOLDER_SELECT),
    { context: "mail_folders:one", rawStatusOk: true }
  );
  if (resp.status === 404) return null;
  raiseForStatus(resp, "mail_folders:one");
  var v = resp.body || {};
  return v.id ? v : null;
}

// The chain for ONE folder, resolved by walking UP its ancestors.
//
// The WALK uses this rather than `buildFolderMap`: it already knows which folder
// it is in and needs only that folder's chain, so paying O(mailbox width) on
// every page — a hundred requests per page on a hundred-folder mailbox — to
// learn one chain is not a cost a backfill can carry. This is O(DEPTH), which
// for a real Outlook tree is two or three requests.
//
// It is NOT a second path resolver. It assembles a partial map of exactly the
// ancestors it walked and then hands it to `mailRelativeChain`, the same one
// function the delta calls — so the walk and the delta cannot drift, which is
// the whole point of this module.
export function folderChainUp(credential, mount, folderId, rootId) {
  if (folderId === rootId) return "";
  var map = {};
  var cur = folderId;
  for (var depth = 0; depth < MAX_DEPTH; depth++) {
    var f = fetchFolder(credential, mount, cur);
    if (!f) return null;
    var entry = {
      name: folderSegment(f.displayName, f.id),
      parentId: f.parentFolderId || null,
    };
    map[f.id] = entry;
    // Also keyed by what we ASKED for, so a well-known name ("inbox") resolves
    // even though Graph answers with the real id.
    map[cur] = entry;
    if (!entry.parentId || entry.parentId === rootId) break;
    cur = entry.parentId;
  }
  return mailRelativeChain(folderId, map, rootId);
}

// The path chain a folder's CONTENTS materialize under, relative to the mount
// root — `""` for the root itself, `"Projects/Acme"` two levels down.
//
// Returns `null` when the chain does not reach the root, and the caller must
// SKIP such an item rather than place it. The engine joins `relative_path` to
// `mount_path` verbatim, so a chain that does not start inside the root would
// write outside the mount — the same rule, and the same reason, as
// `filesRelativePath`.
export function mailRelativeChain(folderId, map, rootId) {
  if (!folderId) return null;
  if (folderId === rootId) return "";
  var segs = [];
  var cur = folderId;
  for (var depth = 0; depth < MAX_DEPTH; depth++) {
    var entry = map[cur];
    if (!entry) return null;
    segs.unshift(entry.name);
    if (entry.parentId === rootId) return segs.join("/");
    cur = entry.parentId;
    if (!cur) return null;
  }
  return null;
}

// ONE ExternalItem shape for a mail FOLDER, built by the walk AND by the delta.
//
// It lives here, next to `folderSegment` and `mailRelativeChain`, for the same
// reason they do: the walk (`childFolderItems` in `read.js`) and the delta
// (`mailTreeChanges`) both have to produce a BYTE-IDENTICAL folder item, or the
// folder node flaps between two shapes on alternating runs — and the etag, which
// carries the folder's own resolved chain, decides whether a rename is seen at
// all (`can_skip_unmapped` returns before rel_path is read).
//
// `chain` is the folder's OWN full chain relative to the mount root, i.e. it
// already ends with this folder's segment.
export function mailFolderItem(folderId, name, chain, parentId, meta) {
  var m = meta || {};
  return {
    external_id: folderId,
    name: name,
    mime_type: null,
    size_bytes: null,
    is_folder: true,
    parent_id: parentId || null,
    created_at: null,
    modified_at: null,
    etag: "mailfolder-1|p=" + chain,
    web_url: null,
    download_url: null,
    metadata: {
      is_folder: true,
      display_name: m.display_name != null ? m.display_name : null,
      total_item_count: m.total_item_count != null ? m.total_item_count : null,
      is_hidden: m.is_hidden === true,
      folder_path: chain,
    },
  };
}
