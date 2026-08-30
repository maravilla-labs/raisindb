// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The read operations: `list`, `get` and `get_content`, plus the series
//! expansion `list` needs to emit one node per series rather than one per
//! occurrence.

import { coded, enc } from "./common.js";
import { GRAPH, graphFetch, raiseForStatus } from "./http.js";
import { INSTANCE_PAGE, calendarId, driveBase, driveContainer, eventSelect, isMailTree, mailFolderId, mailSelect, outlookHeaders, pageSize, principal, resourceOf, windowBounds } from "./mount.js";
import { enrichAttachments } from "./mail.js";
import { toExternalItem } from "./items.js";
import { calendarChanges } from "./changes.js";
import { buildFolderMap, folderChainUp, folderSegment, listChildFolders, mailFolderItem, resolveRootFolder } from "./mail-folders.js";

export function opList(credential, mount, params) {
  var resource = resourceOf(mount);
  var url;
  // A mail TREE mount is walked folder by folder, exactly as a drive is, so it
  // has its own listing shape rather than a branch inside this one.
  if (resource === "mail" && isMailTree(mount)) {
    return mailTreeList(credential, mount, params);
  }
  if (params.cursor) {
    url = params.cursor;
  } else if (resource === "calendar") {
    url =
      GRAPH + principal(mount) + "/calendars/" + enc(calendarId(mount)) +
      "/events?$top=" + pageSize(params) + "&$select=" + enc(eventSelect(mount));
  } else if (resource === "files") {
    // The engine's full walk recurses folders EXPLICITLY: each `list` call
    // names the folder whose children it wants via `params.folder_id` (null
    // for the mount root). Ignoring it — as this branch used to — re-listed
    // the mount ROOT for every queued subfolder: nested content was never
    // enumerated at all, the same root-level folders were rediscovered and
    // re-queued on every pop, and the walk never converged. A flat drive
    // masked it completely, because the first pop IS the root.
    var container = params.folder_id
      ? driveBase(mount) + "/items/" + enc(params.folder_id)
      : driveContainer(mount);
    url = GRAPH + container + "/children?$top=" + pageSize(params);
  } else {
    url =
      GRAPH + principal(mount) + "/mailFolders/" + enc(mailFolderId(mount)) +
      "/messages?$top=" + pageSize(params) + "&$select=" + enc(mailSelect(mount));
  }
  var resp = graphFetch(credential, "GET", url, {
    context: "list",
    headers: outlookHeaders(mount),
  });
  var body = resp.body || {};
  var values = body.value || [];
  var items = values.map(function (v) {
    return toExternalItem(v, resource, mount);
  });
  if (resource === "calendar") {
    items = items.concat(seriesExceptions(credential, mount, values));
  } else if (resource === "mail") {
    enrichAttachments(credential, mount, items);
  }
  return { items: items, next_cursor: body["@odata.nextLink"] || null };
}

// ---- the mail tree walk ---------------------------------------------------

// One page of ONE folder of a mail tree mount: that folder's messages, and — on
// the LAST page only — its child folders.
//
// This is the half that makes the WALK authoritative again, which is the whole
// reason a tree mount can ever prune anything: `reconcile_deletes` acts on the
// `seen` set the walk builds, and the delta path deliberately never deletes
// (see `flatChanges`). A tree whose folders were never enumerated would leave
// every message of every subfolder unseen.
//
// The engine drives it: `full.rs` pushes `(item.external_id, rel_path)` for
// every `is_folder` item and pops it later with `params.folder_id`, accumulating
// `prefix` as it descends. So a message in `Projects/Acme` materializes at
// `Projects/Acme/{id}` — and `mailRelativeChain` must produce exactly
// `Projects/Acme` for the delta, which is why both call the same function.
//
// FOLDERS ON THE LAST PAGE, not the first: the engine pops the stack after this
// folder's pages are done, so discovering a subfolder late costs nothing but the
// order folders are visited in, and no page but the last pays a childFolders
// request.

// A mail-tree page cursor: the Graph link PLUS the two things resolving it cost
// a request each.
//
// `next_cursor` is opaque to the engine — `full.rs` stores it and hands it back
// verbatim — so it is ours to shape, exactly as the delta cursor is. Before
// this, EVERY page of a folder re-resolved the mount root (one request) and
// re-walked the folder's whole ancestor chain (one request per level). A folder
// three levels down with ten pages of mail paid forty requests to re-learn a
// string that cannot change between two pages of the same listing, against the
// same Graph ceiling the delta poll is bounded for.
//
// A bare URL is still accepted, so a cursor persisted by the previous shape
// resumes instead of erroring.
var PAGE_CURSOR_PREFIX = "rsn-mailpage-1:";

function wrapPageCursor(url, rootId, chain) {
  return PAGE_CURSOR_PREFIX + JSON.stringify({ u: url, r: rootId, p: chain });
}

function unwrapPageCursor(cursor) {
  if (typeof cursor !== "string" || cursor.indexOf(PAGE_CURSOR_PREFIX) !== 0) return null;
  var o = null;
  try {
    o = JSON.parse(cursor.slice(PAGE_CURSOR_PREFIX.length));
  } catch (e) {
    o = null;
  }
  if (!o || typeof o.u !== "string" || typeof o.p !== "string" || !o.r) return null;
  return o;
}

function mailTreeList(credential, mount, params) {
  var requested = params.folder_id;
  // The first pop carries `mount.remote_root`, which is usually a WELL-KNOWN
  // NAME ("inbox"). Graph accepts it in a URL but never returns it as a
  // `parentFolderId`, so it has to be normalized to the resolved id here or the
  // root's own chain resolves to null and the whole mount lists nothing.
  var atRoot = !requested || requested === mailFolderId(mount);
  var resumed = unwrapPageCursor(params.cursor);
  // A cursor that WEARS OUR PREFIX but does not unwrap is not a Graph link, and
  // it must never be fetched as one. `params.cursor` doubles as the raw URL
  // below (a bare link persisted by the previous cursor shape still resumes),
  // so a truncated or malformed `rsn-mailpage-1:` blob would otherwise be
  // handed to graphFetch verbatim and issue a request to a nonsense host.
  // Dropped to "no cursor" instead: this folder's listing restarts from page 1,
  // which costs a re-read of pages that are idempotent through the etag skip.
  var pageCursor =
    typeof params.cursor === "string" &&
    params.cursor.indexOf(PAGE_CURSOR_PREFIX) === 0 &&
    !resumed
      ? null
      : params.cursor;
  var rootId;
  var chain;
  if (resumed) {
    // A CONTINUATION of a listing this function already resolved. Nothing about
    // the root or the chain can have changed between two pages of one folder,
    // and re-deriving them is the per-page cost this cursor exists to remove.
    rootId = resumed.r;
    chain = resumed.p;
  } else if (atRoot && !pageCursor) {
    // THE `max_folders` CEILING IS ENFORCED ON THE WALK TOO, and it has to be.
    //
    // `buildFolderMap` throws `config_error` above the ceiling, but only
    // `get_changes` called it — so a mailbox with more folders than the ceiling
    // BACKFILLED happily (thousands of requests, a folder node per folder) and
    // only then discovered, on its first delta, that this mount can never sync.
    // The operator learned about a limit at the one moment the expensive work
    // was already done, and the mount sat permanently `misconfigured` on top of
    // a complete import.
    //
    // Checked on the ROOT's FIRST page only — once per walk, not once per
    // folder and not once per page — so it costs one folder-map build at the
    // start of a backfill and nothing at all thereafter. Its `rootId` is the
    // same one `resolveRootFolder` would have returned, so this replaces that
    // request rather than adding to it.
    rootId = buildFolderMap(credential, mount).rootId;
    chain = "";
  } else {
    rootId = resolveRootFolder(credential, mount).id;
    chain = folderChainUp(credential, mount, atRoot ? rootId : requested, rootId);
  }
  var container = atRoot ? rootId : requested;
  if (chain === null) {
    // The engine asked for a folder that is no longer under the mount root — it
    // was moved or deleted between the push and this pop. An empty page, not an
    // error: the walk carries on, and this run is `truncated:false` only if
    // everything else listed, so reconcile stays correct.
    return { items: [], next_cursor: null };
  }

  var url =
    (resumed ? resumed.u : pageCursor) ||
    GRAPH + principal(mount) + "/mailFolders/" + enc(container) +
      "/messages?$top=" + pageSize(params) + "&$select=" + enc(mailSelect(mount));
  var resp = graphFetch(credential, "GET", url, {
    context: "list:mail_tree",
    headers: outlookHeaders(mount),
  });
  var body = resp.body || {};
  var values = body.value || [];
  var items = [];
  for (var i = 0; i < values.length; i++) {
    // `chain` and NOT the message's own parentFolderId: the path the engine
    // gives this item is the prefix it accumulated for THIS folder, so the
    // folder-path folded into the etag has to be that same chain or the etag
    // would disagree with the path it is supposed to guard.
    items.push(toExternalItem(values[i], "mail", mount, chain));
  }
  enrichAttachments(credential, mount, items);

  var next = body["@odata.nextLink"] || null;
  if (!next) {
    items = items.concat(
      childFolderItems(listChildFolders(credential, mount, container), container, chain)
    );
  }
  return {
    items: items,
    next_cursor: next ? wrapPageCursor(next, rootId, chain) : null,
  };
}

// The child folders of `container`, as items the engine will materialize as
// nodes and push onto its backfill stack.
//
// The etag carries the folder's own resolved path for the same reason a
// message's does: `can_skip_unmapped` returns before rel_path is read, and an
// Outlook folder rename changes nothing else about the folder, so without it a
// renamed folder's NODE would stay at its old path forever too.
function childFolderItems(children, container, chain) {
  var out = [];
  for (var i = 0; i < children.length; i++) {
    var f = children[i];
    if (!f || !f.id) continue;
    var name = folderSegment(f.displayName, f.id);
    var childChain = chain ? chain + "/" + name : name;
    // The SAME builder the delta uses, so the walk and the delta cannot emit
    // two different shapes (or two different etags) for one folder.
    out.push(
      mailFolderItem(f.id, name, childChain, container, {
        display_name: f.displayName || null,
        total_item_count: f.totalItemCount != null ? f.totalItemCount : null,
        is_hidden: f.isHidden === true,
      })
    );
  }
  return out;
}

// The full walk's half of "an exception is its own node".
//
// `/events` returns singleInstance and seriesMaster entities only — an
// EXCEPTION (a single occurrence that was moved, renamed or cancelled) is not
// among them. The delta feed does see exceptions. If only the delta emitted
// them, every full reconcile would find those nodes absent from the listing and
// DELETE them, so an exception node would flap in and out of existence on
// alternating runs. Both paths must agree about what an item is.
//
// So each series master in the page is expanded once through `/instances` and
// only its `type === "exception"` instances are kept; plain occurrences stay
// collapsed into the master, exactly as the delta path collapses them. A
// non-recurring calendar makes zero extra requests — the cost is one request
// per SERIES, not per event.
export function seriesExceptions(credential, mount, values) {
  var out = [];
  var win = windowBounds(mount);
  for (var i = 0; i < values.length; i++) {
    var v = values[i];
    if (!v || v.type !== "seriesMaster" || !v.id) continue;
    var url =
      GRAPH + principal(mount) + "/events/" + enc(v.id) + "/instances" +
      "?startDateTime=" + enc(win.start) + "&endDateTime=" + enc(win.end) +
      "&$top=" + INSTANCE_PAGE + "&$select=" + enc(eventSelect(mount));
    var resp = graphFetch(credential, "GET", url, {
      context: "list:series_instances",
      rawStatusOk: true,
      headers: outlookHeaders(mount),
    });
    // A master that vanished between the page and this call is not an error:
    // the next run reconciles it. Anything else is reported normally.
    if (resp.status === 404) continue;
    raiseForStatus(resp, "list:series_instances");
    var instances = (resp.body && resp.body.value) || [];
    for (var j = 0; j < instances.length; j++) {
      if (instances[j] && instances[j].type === "exception") {
        out.push(toExternalItem(instances[j], "calendar", mount));
      }
    }
    cancelledOccurrences(credential, mount, v.id, out);
  }
  return out;
}

// How many cancelled instances of ONE series are recovered per walk.
export var MAX_CANCELLED_PER_SERIES = 50;

// A CANCELLED single occurrence, which nothing else in this adapter can see.
//
// Graph removes a cancelled instance from `/instances` entirely — it is not
// returned as a cancelled exception — and the delta feed reports it only as an
// `@removed` entry carrying a `seriesMasterId`, which `calendarChanges` refuses
// to act on (correctly: a removal from a bounded calendarView also covers an
// event that merely moved out of the window, so treating one as a delete
// destroys rescheduled events). The result was that a cancellation produced NO
// item at all, in either path.
//
// That is not a cosmetic gap. The expander suppresses a projected occurrence
// only when an exception node exists at that slot (`calendar_expand/rebuild.rs`
// builds the suppression set from `recurrence_type: exception` +
// `original_start_utc`). With no node, it regenerates the meeting on every
// rebuild — a cancelled standup showing up every week, forever, with no error
// anywhere.
//
// `cancelledOccurrences` on the series master is the documented way to see
// them. It is read through its own request, and every failure degrades to the
// previous behaviour rather than failing the walk: a Graph that does not answer
// it must not be able to take the whole calendar listing down.
export function cancelledOccurrences(credential, mount, masterId, out) {
  var probe = graphFetch(
    credential,
    "GET",
    GRAPH + principal(mount) + "/events/" + enc(masterId) +
      "?$select=id,cancelledOccurrences",
    { context: "list:cancelled_occurrences", rawStatusOk: true, headers: outlookHeaders(mount) }
  );
  if (probe.status < 200 || probe.status >= 300) return;
  var ids = (probe.body && probe.body.cancelledOccurrences) || [];
  if (!ids.length) return;

  var limit = Math.min(ids.length, MAX_CANCELLED_PER_SERIES);
  for (var k = 0; k < limit; k++) {
    // The occurrence id is `OID.{master}.{date}` — a DATE, not the instant the
    // suppression is keyed by. Only the occurrence resource itself carries
    // `originalStart`, which Graph states in UTC, so it is fetched rather than
    // derived: this sandbox has no tz database and a wrong instant suppresses
    // the wrong slot, which is worse than suppressing none.
    var one = graphFetch(
      credential,
      "GET",
      GRAPH + principal(mount) + "/events/" + enc(ids[k]) +
        "?$select=" + enc(eventSelect(mount)),
      { context: "list:cancelled_occurrence", rawStatusOk: true, headers: outlookHeaders(mount) }
    );
    if (one.status < 200 || one.status >= 300) continue;
    var ev = one.body;
    if (!ev || !ev.id || !ev.originalStart) continue;
    // Stated as an EXCEPTION, whatever Graph calls it. A cancelled instance is
    // an override of one slot of the series, which is exactly what an exception
    // is; typed as an `occurrence` it would instead look like one of the
    // expander's own projection nodes and be overwritten by the next rebuild.
    ev.type = "exception";
    ev.isCancelled = true;
    if (!ev.seriesMasterId) ev.seriesMasterId = masterId;
    out.push(toExternalItem(ev, "calendar", mount));
  }
}

export function opGet(credential, mount, params) {
  var resource = resourceOf(mount);
  if (!params.item_id) return null;
  var url;
  if (resource === "calendar") {
    url =
      GRAPH + principal(mount) + "/events/" + enc(params.item_id) +
      "?$select=" + enc(eventSelect(mount));
  } else if (resource === "files") {
    url = GRAPH + driveBase(mount) + "/items/" + enc(params.item_id);
  } else {
    url =
      GRAPH + principal(mount) + "/messages/" + enc(params.item_id) +
      "?$select=" + enc(mailSelect(mount));
  }
  var resp = graphFetch(credential, "GET", url, {
    context: "get",
    rawStatusOk: true,
    headers: outlookHeaders(mount),
  });
  if (resp.status === 404) return null;
  raiseForStatus(resp, "get");
  return toExternalItem(resp.body, resource, mount);
}

// Message/event body (or file bytes) on demand. Not called during ordinary
// link-only sync. For files, Graph's /content 302-redirects to a per-item
// download host that the adapter network policy does NOT allow-list, so opt-in
// file content sync may be blocked — link via metadata.download_url instead.
export function opGetContent(credential, mount, params) {
  var resource = resourceOf(mount);
  if (resource === "files") {
    // POINT at the bytes; do not carry them.
    //
    // This adapter cannot return drive-file bytes itself: `raisin.http.fetch`
    // decodes every response as TEXT, so any binary file (image, PDF, zip) is
    // corrupted before the adapter even sees it — the failure the attachment
    // branch below documents — and Graph's `/content` 302-redirects to a
    // per-item download host outside this adapter's network allow-list.
    //
    // So we answer with `fetch_url` and the ENGINE downloads it, in Rust,
    // behind the operator's egress policy. The URL is minted HERE, on this
    // call, and used immediately: `@microsoft.graph.downloadUrl` is
    // pre-authenticated and lives about an hour, so the one thing that must
    // never happen is serving a copy persisted at sync time. That is exactly
    // why the node's `meta.download_url` is a convenience link and not the
    // content path.
    var meta = graphFetch(
      credential,
      "GET",
      GRAPH + driveBase(mount) + "/items/" + enc(params.item_id) +
        "?$select=id,name,size,file,@microsoft.graph.downloadUrl",
      { context: "get_content(file)", rawStatusOk: true }
    );
    if (meta.status === 404) return null;
    raiseForStatus(meta, "get_content(file)");
    var item = meta.body || {};
    var url = item["@microsoft.graph.downloadUrl"];
    if (typeof url !== "string" || !url) {
      // A folder, or an item Graph will not hand out a link for. Saying so
      // beats storing an empty file that reads as "fetched".
      throw coded(
        "get_content: Microsoft Graph returned no download URL for '" +
          params.item_id + "' (a folder, or content this account cannot read)",
        "config_error"
      );
    }
    return {
      fetch_url: url,
      mime_type: (item.file && item.file.mimeType) || "application/octet-stream",
    };
  }
  // A mail ATTACHMENT: `parent_item_id` is the message, `item_id` the
  // attachment. Graph has no route that addresses an attachment on its own, so
  // the engine sends both halves of the namespaced `__external_id`.
  //
  // Returned as `content_base64`, never `content`: a JS string cannot hold
  // arbitrary bytes, and a PDF round-tripped through one comes back corrupted
  // with no error anywhere. `contentBytes` is already base64 on the wire, so
  // this is a pass-through, not an encode.
  if (resource === "mail" && params.parent_item_id) {
    var att = graphFetch(
      credential,
      "GET",
      GRAPH + principal(mount) + "/messages/" + enc(params.parent_item_id) +
        "/attachments/" + enc(params.item_id),
      { context: "get_content(attachment)", headers: outlookHeaders(mount) }
    ).body;
    if (!att || typeof att.contentBytes !== "string") {
      // A referenceAttachment / itemAttachment carries no bytes at all. Saying
      // so is better than storing an empty file that reads as "fetched".
      throw coded(
        "get_content: attachment " + params.item_id + " has no downloadable content " +
          "(type " + (att && att["@odata.type"]) + ")",
        "config_error"
      );
    }
    return {
      content_base64: att.contentBytes,
      mime_type: att.contentType || "application/octet-stream",
    };
  }

  var base =
    resource === "calendar"
      ? GRAPH + principal(mount) + "/events/" + enc(params.item_id)
      : GRAPH + principal(mount) + "/messages/" + enc(params.item_id);
  var resp2 = graphFetch(credential, "GET", base + "?$select=body", {
    context: "get_content",
    headers: outlookHeaders(mount),
  });
  var b = resp2.body && resp2.body.body;
  var mime = b && b.contentType === "html" ? "text/html" : "text/plain";
  return { content: b ? b.content || "" : "", mime_type: mime };
}
