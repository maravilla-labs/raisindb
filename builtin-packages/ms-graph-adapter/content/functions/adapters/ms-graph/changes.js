// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The incremental feed: the first delta URL, `get_changes`, and the collapse
//! that keeps a recurring series ONE node instead of one per occurrence.

import { coded, enc } from "./common.js";
import { GRAPH, graphFetch, raiseForStatus } from "./http.js";
import { calendarSupportsDelta, driveContainer, eventSelect, folderScope, isMailTree, mailFolderId, mailSelect, outlookHeaders, principal, resourceOf, useImmutableIds, windowBounds, windowConfig } from "./mount.js";
import { enrichAttachments } from "./mail.js";
import { toExternalItem } from "./items.js";
import { buildFolderMap, mailFolderItem, mailRelativeChain } from "./mail-folders.js";
import { filesRelativePath, isMountRootContainer, mountRootSegments } from "./paths.js";
import { seriesExceptions } from "./read.js";

// Build the FIRST delta URL (no since_token yet). Subsequent calls reuse the
// engine-persisted token verbatim — it is a full @odata.nextLink/deltaLink.
//
// `baselineOnly` asks Graph for a delta link WITHOUT enumerating. This is the
// difference between "import everything from the beginning" and "tell me what
// changes from now on", and getting it wrong is not subtle:
//
// A delta query with no token performs an INITIAL FULL ENUMERATION. Graph
// returns every item in the folder, paged, and only emits @odata.deltaLink on
// the final page. The engine stores whatever comes back as the delta token, so
// page 1 of an enumeration becomes the "baseline" — and every later delta run
// walks that enumeration a page at a time, re-reading items the full walk had
// already imported (`0 written / 600 skipped`, run after run) while genuinely
// new items sit unreachable behind it. On a large mailbox that never converges.
//
// `$deltatoken=latest` (drive: `token=latest`) returns the delta link straight
// away with an empty page. The engine calls this ONLY after a full walk has
// materialized everything, which is exactly when "from now on" is correct.
export function initialDeltaUrl(mount, resource, baselineOnly) {
  if (resource === "calendar") {
    var win = windowBounds(mount);
    // Mailbox-level, NOT `/calendars/{id}/calendarView/delta` — that route is
    // not part of v1.0. `calendarSupportsDelta` is what guarantees we only get
    // here for the primary calendar, so addressing the mailbox is correct.
    return (
      GRAPH + principal(mount) + "/calendarView/delta?startDateTime=" + enc(win.start) +
      "&endDateTime=" + enc(win.end) +
      "&$select=" + enc(eventSelect(mount)) +
      (baselineOnly ? "&$deltatoken=latest" : "")
    );
  }
  if (resource === "files") {
    // Drive spells it differently: `token=latest`, no `$`.
    return GRAPH + driveContainer(mount) + "/delta" +
      (baselineOnly ? "?token=latest" : "");
  }
  return mailFolderDeltaUrl(mount, mailFolderId(mount), baselineOnly);
}

// The delta feed for ONE mail folder.
//
// Graph's message delta is FOLDER-SCOPED and there is no mailbox-wide
// `/messages/delta` in v1.0 — only `/mailFolders/{id}/messages/delta`
// (learn.microsoft.com message-delta). That single fact is why a tree mount
// needs one link per folder rather than one link, and it is not a limitation
// this adapter can design around.
export function mailFolderDeltaUrl(mount, folderId, baselineOnly) {
  return (
    GRAPH + principal(mount) + "/mailFolders/" + enc(folderId) +
    "/messages/delta?$select=" + enc(mailSelect(mount)) +
    (baselineOnly ? "&$deltatoken=latest" : "")
  );
}

// ---- cursor identity ------------------------------------------------------
//
// A delta cursor is NOT just a resume point: Graph bakes the query that minted
// it INTO the link. $select, the calendarView date range and the folder are all
// frozen at mint time and replayed verbatim on every subsequent poll. So a
// stored token silently encodes configuration that has since changed, and the
// engine — which treats the token as opaque — has no way to notice.
//
// Two real failures came from this:
//   * Turning on `include_body` was a permanent no-op. The widened $select
//     never reached the feed, so bodies never arrived, on old messages or new,
//     with nothing anywhere to say why.
//   * A primary calendar's `calendarView` window never slid forward. Roughly
//     `days_ahead` after the mount was created, newly scheduled meetings simply
//     stopped arriving.
//
// The fix is to give the cursor an IDENTITY and check it before use. The token
// is ours to shape — the engine stores whatever string we return — so the
// identity travels inside it. When it no longer matches the mount's current
// configuration we report `cursor_invalid`, which the engine already recovers
// from in-run by discarding the cursor and running a full reconcile.
//
// Deliberately NOT a hash: these strings are short, and a mismatch an operator
// can read in a log beats one they have to reproduce.
var CURSOR_PREFIX = "rsn-cur-1:";

export function cursorIdentity(mount, resource) {
  var ids = useImmutableIds(mount) ? "immutable" : "default";
  if (resource === "calendar") {
    var win = windowConfig(mount);
    return "calendar|" + principal(mount) + "|" + ids +
      "|win=" + win.daysBack + "/" + win.daysAhead +
      "|sel=" + eventSelect(mount);
  }
  if (resource === "files") {
    // Drive delta carries no projection and no window; the container is the
    // whole of what the link encodes.
    return "files|" + driveContainer(mount);
  }
  // `scope=tree` joins the identity so flipping `folder_scope` throws
  // `cursor_invalid` and forces ONE clean reconcile, rather than replaying a
  // folder-scoped link under a tree mount (or a tree map under a folder one)
  // and silently syncing the wrong shape.
  //
  // APPENDED ONLY IN TREE MODE, on purpose. Spelling `scope=folder` into the
  // identity would change the string every existing mail mount stored and
  // resync the entire fleet's mailboxes to record a value none of them use.
  // A tree mount is by definition new or reconfigured, so it pays a resync it
  // was going to pay anyway.
  var scope = folderScope(mount) === "tree" ? "|scope=tree" : "";
  return "mail|" + principal(mount) + "|" + mailFolderId(mount) + "|" + ids +
    scope + "|sel=" + mailSelect(mount);
}

// How long a calendar cursor may be reused before the window it froze is
// refreshed. A quarter of `days_ahead` keeps coverage at 75% of the configured
// window at worst, and floors at a day so a tiny window still makes progress.
function calendarCursorMaxAgeMs(mount) {
  var daysAhead = windowConfig(mount).daysAhead;
  return Math.max(86400000, Math.floor((daysAhead * 86400000) / 4));
}

export function wrapCursor(url, identity, mintedMs) {
  return CURSOR_PREFIX + JSON.stringify({ u: url, k: identity, m: mintedMs });
}

// Returns `{u, k, m}`, or null when this is not one of ours — which means a
// token minted before cursor identity existed.
export function unwrapCursor(token) {
  if (typeof token !== "string" || token.indexOf(CURSOR_PREFIX) !== 0) return null;
  try {
    var o = JSON.parse(token.slice(CURSOR_PREFIX.length));
    return o && typeof o.u === "string" ? o : null;
  } catch (e) {
    return null;
  }
}

// Resolve a stored token to the URL to fetch, or throw `cursor_invalid` when it
// can no longer be trusted.
//
// A LEGACY token — one minted before this wrapper existed — is invalidated
// rather than grandfathered. It cannot be trusted precisely because we cannot
// tell what minted it, and the two bugs above are both silent-data-loss paths
// that a grandfathered token would carry forward indefinitely. The cost is one
// full reconcile per existing mount, once.
function resolveCursor(mount, resource, token) {
  var wrapped = unwrapCursor(token);
  if (!wrapped) {
    throw coded(
      "stored delta cursor predates cursor identity and cannot be verified " +
        "against the mount's current projection; resyncing once",
      "cursor_invalid"
    );
  }
  var want = cursorIdentity(mount, resource);
  if (wrapped.k !== want) {
    throw coded(
      "delta cursor was minted for a different query and Graph freezes the " +
        "query inside the link (stored: " + wrapped.k + " / current: " + want + ")",
      "cursor_invalid"
    );
  }
  if (resource === "calendar" && typeof wrapped.m === "number") {
    var age = Date.now() - wrapped.m;
    if (age > calendarCursorMaxAgeMs(mount)) {
      throw coded(
        "calendarView cursor froze its date window " +
          Math.floor(age / 86400000) + " days ago and the window no longer " +
          "reaches days_ahead; resyncing to slide it forward",
        "cursor_invalid"
      );
    }
  }
  return wrapped;
}

// ---- partial delta payloads -----------------------------------------------
//
// A delta entry for a CHANGED item is not always the whole item. Graph is
// entitled to send only what changed — marking a message unread in Outlook
// produces `{ "@odata.etag": ..., id, isRead: false }` and nothing else — and
// `mailMeta` answers a missing key with an explicit `null`, because for the
// full walk an absent key genuinely means "no value".
//
// The engine's upsert then rebuilds the node's property map wholesale from that
// answer, which is its documented contract, so ONE flag change in Outlook wiped
// the message's subject, sender, recipients, date and body to null. The node
// kept its id and its etag, so nothing downstream could tell the difference
// between "this message has no subject" and "we were not told the subject".
//
// Fixed at the source rather than by teaching the engine to merge: an
// `ExternalItem` means COMPLETE state, everywhere, and a merge contract would
// make it impossible to ever clear a field legitimately. A partial entry is
// re-read in full instead.
//
// MAIL ONLY, deliberately.
//
// The anchor keys below are ones every real message carries under this
// adapter's own projection, so testing for their KEY (not their value)
// distinguishes "was not sent" from "was sent as null". A flag-only change
// carries none of them.
//
// The other two resources are left alone rather than guessed at. A driveItem
// delta is documented to carry the whole resource; and on the calendar side
// `calendarChanges` already re-reads a series master through `fetchEvent`, so a
// second hydration here would double-fetch — and a `seriesMaster` legitimately
// carries no `start` of its own, which makes any simple anchor wrong. If a
// calendar equivalent of this bug turns up, it needs its own anchor, not this
// one widened.
function isPartial(v, resource) {
  if (resource !== "mail") return false;
  if (!v || v["@removed"] || !v.id) return false;
  return !("receivedDateTime" in v) && !("sentDateTime" in v) && !("subject" in v);
}

// Re-read one message with the mount's full projection. Returns null when it is
// gone, which the caller treats as "skip", never as "it has no fields".
function refetchFull(credential, mount, id) {
  var url =
    GRAPH + principal(mount) + "/messages/" + enc(id) +
    "?$select=" + enc(mailSelect(mount));
  var resp = graphFetch(credential, "GET", url, {
    context: "get_changes:rehydrate",
    rawStatusOk: true,
    headers: outlookHeaders(mount),
  });
  if (resp.status === 404) return null;
  raiseForStatus(resp, "get_changes:rehydrate");
  return resp.body || null;
}

// Replace every partial entry in a delta page with the full object.
//
// Costs one request per CHANGED item, and only for the ones that arrived
// partial — a page of unchanged items costs nothing, and a page of new mail
// arrives complete.
function hydratePage(credential, mount, resource, values) {
  var out = [];
  for (var i = 0; i < values.length; i++) {
    var v = values[i];
    if (!isPartial(v, resource) || !v.id) {
      out.push(v);
      continue;
    }
    var full = refetchFull(credential, mount, v.id);
    if (!full) continue; // gone between the page and now; the walk reconciles it
    out.push(full);
  }
  return out;
}

// ---- the mail TREE cursor -------------------------------------------------
//
// ONE opaque token, N delta links. The engine owns exactly one cursor per mount
// (`state.last_sync_token`) and treats it as an opaque string the adapter
// shapes — which is already true here, `rsn-cur-1:` is ours. A tree mount
// carries its per-folder map inside that same one string, so the engine's
// one-cursor-per-mount rule is untouched; the adapter simply decides what the
// cursor MEANS.
//
// Shape: { k: identity, r: resumeFolderId|null, i: rootFolderId, m: { <id>: e } }
//   e.t  the folder's $deltatoken / $skiptoken, VERBATIM as Graph encoded it,
//        or null for "start a fresh enumeration"
//   e.s  "enum" while this folder is still mid-enumeration ($skiptoken, or no
//        token yet), "delta" once it has handed back a deltaLink
//   e.p  the folder's resolved path chain AT MINT TIME — comparing it to the
//        chain resolved now is how a rename is detected
//   e.u  a full link, present ONLY when the token could not be parsed out of it
//
// THE TOKEN, NOT THE LINK, AND THAT IS A SIZE FIX. `delta.rs` rewrites this blob
// after every page, and a Graph message-delta link is dominated by the
// URL-encoded `$select` — ~330 characters of it once `include_body` and
// `parentFolderId` are on. At `max_folders: 100` that is ~33 KB of the ~95 KB
// cursor spent storing one constant a hundred times. It is reconstructible:
// `$select` comes from `mailSelect(mount)`, which is part of the cursor IDENTITY
// (`cursorIdentity`), so a cursor that survives the identity check is by
// definition one whose links carried exactly the projection `entryUrl` rebuilds.
// A link we cannot parse a token out of is kept whole in `e.u`, so an
// unrecognised link shape degrades to the old size rather than to a wrong query.
//
// Its own prefix, so it can never be mistaken for a folder-mode `rsn-cur-1:`
// link (or the reverse) by anything that merely checks for a JSON wrapper. `-2:`
// because the entry shape changed: a `-1:` token throws `cursor_invalid`, which
// costs one resync of a mount type that is opt-in and default-off.
var TREE_CURSOR_PREFIX = "rsn-mailtree-2:";

// How many folders ONE get_changes call advances.
//
// Bounded, and the rotation resume point is persisted, because the alternative
// is starvation that reports `ok`: one busy folder would consume the whole
// `max_items_per_sync` budget every run and the other 199 would never advance,
// with items written and nothing anywhere saying the rest of the mailbox is
// standing still.
export var MAIL_TREE_SLICE = 5;

function wrapTreeCursor(identity, resumeAt, rootId, entries) {
  return (
    TREE_CURSOR_PREFIX +
    JSON.stringify({ k: identity, r: resumeAt || null, i: rootId, m: entries })
  );
}

// Graph spells the continuation `$skiptoken` and the resume point `$deltatoken`,
// and percent-encodes the `$` in some links. The token value is captured RAW and
// re-appended raw, so the rebuilt URL is byte-identical to the one Graph minted
// rather than a re-encoding of it.
var LINK_TOKEN_RE = /[?&](?:\$|%24)(delta|skip)token=([^&]*)/i;

function linkToken(url) {
  var m = LINK_TOKEN_RE.exec(String(url || ""));
  if (!m) return null;
  return {
    t: m[2],
    s: m[1].toLowerCase() === "delta" ? "delta" : "enum",
  };
}

// The URL to fetch for one folder, rebuilt from its stored token.
export function entryUrl(mount, folderId, entry) {
  if (entry && entry.u) return entry.u;
  var base = mailFolderDeltaUrl(mount, folderId, false);
  if (!entry || !entry.t) return base;
  return base + (entry.s === "delta" ? "&$deltatoken=" : "&$skiptoken=") + entry.t;
}

// Returns the stored tree cursor, or null when there is no token yet. Anything
// that is not a CURRENT tree cursor throws `cursor_invalid`, which the engine
// already recovers from in-run by discarding the cursor and full-reconciling.
function resolveTreeCursor(mount, token) {
  if (!token) return null;
  var want = cursorIdentity(mount, "mail");
  if (typeof token !== "string" || token.indexOf(TREE_CURSOR_PREFIX) !== 0) {
    throw coded(
      "stored mail cursor is not a current folder-tree cursor (the mount was " +
        "switched to folder_scope: tree, or predates this cursor shape); " +
        "resyncing once",
      "cursor_invalid"
    );
  }
  var o = null;
  try {
    o = JSON.parse(token.slice(TREE_CURSOR_PREFIX.length));
  } catch (e) {
    o = null;
  }
  if (!o || !o.m || typeof o.m !== "object") {
    throw coded("stored mail tree cursor is unreadable; resyncing once", "cursor_invalid");
  }
  if (o.k !== want) {
    throw coded(
      "mail tree cursor was minted for a different query and Graph freezes the " +
        "query inside every folder link (stored: " + o.k + " / current: " + want + ")",
      "cursor_invalid"
    );
  }
  return o;
}

function sortedIds(obj) {
  var ids = [];
  for (var id in obj) {
    if (Object.prototype.hasOwnProperty.call(obj, id)) ids.push(id);
  }
  // Sorted, so the rotation visits folders in a STABLE order across runs. An
  // order that changed with the map's iteration order would let the persisted
  // resume point land on a different folder every call, which is starvation
  // wearing the costume of progress.
  ids.sort();
  return ids;
}

// Carry a stored entry map across a CACHED round, unchanged.
//
// No folder listing was made this call, so there is nothing that could add,
// drop, rename or re-home a folder — the entries ARE the chain lookup table for
// the round, and copying them is the whole of "the map is cached".
function carryEntries(prev) {
  var out = {};
  for (var id in prev) {
    if (!Object.prototype.hasOwnProperty.call(prev, id)) continue;
    var e = prev[id];
    if (!e || typeof e.p !== "string") continue;
    var n = { t: e.t || null, s: e.s === "delta" ? "delta" : "enum", p: e.p };
    if (e.u) n.u = e.u;
    out[id] = n;
  }
  return out;
}

// Reconcile the per-folder entries against a FRESH folder map: seed the folders
// that appeared, re-enumerate the ones whose chain moved, carry the rest, and
// emit the folder ITEMS for both of the first two.
//
// THE FOLDER ITEM IS THE HALF THAT WAS MISSING, and without it a rename or a new
// folder relocated the MESSAGES and left the folder NODE behind. The delta
// emitted no `is_folder` item at all, so the engine's `ensure_ancestors`
// auto-created a plain `raisin:Folder` at the new chain that carries no
// `__external_id` — which means `reconcile_deletes` can never prune it — while
// the real, mount-owned folder node kept its old, now-empty path until the next
// COMPLETE full walk. The user saw both, and only one of them was ever going
// away.
//
// The item comes from `mailFolderItem`, the same builder `childFolderItems` uses
// on the walk, so the two paths cannot produce two shapes (or two etags) for one
// folder.
function reconcileEntries(mount, built, prev, baselineOnly) {
  var rootId = built.rootId;
  var map = built.map;
  var ids = [rootId];
  for (var id in map) {
    if (Object.prototype.hasOwnProperty.call(map, id)) ids.push(id);
  }
  ids.sort();

  var entries = {};
  var order = [];
  var folderItems = [];
  for (var i = 0; i < ids.length; i++) {
    var fid = ids[i];
    var chain = fid === rootId ? "" : mailRelativeChain(fid, map, rootId);
    // Unreachable from the root: skip rather than place. Same rule as
    // `filesRelativePath` — the engine joins relative_path to mount_path
    // verbatim, so a chain that does not start inside the root escapes it.
    if (chain === null) continue;
    var old = prev[fid];
    var isNew = !old || typeof old.p !== "string";
    var moved = !isNew && old.p !== chain;
    if (isNew) {
      // A NEW FOLDER, and the mount-level rule is deliberately INVERTED here.
      //
      // At the mount level `$deltatoken=latest` is right after a full walk:
      // everything is already materialized, so "from now on" is the truth. A
      // folder that appeared since is the opposite case — NONE of its messages
      // has ever been imported — and seeding it with `latest` would drop an
      // entire folder's history with nothing anywhere to observe. So it is
      // seeded with an ENUMERATION (no token at all), and only a genuine
      // first-call baseline (where the walk has just materialized everything)
      // uses `latest`.
      entries[fid] = baselineOnly
        ? { t: "latest", s: "delta", p: chain }
        : { t: null, s: "enum", p: chain };
    } else if (moved) {
      // RENAMED OR MOVED. The folder id survives both (Graph's mailFolder ids
      // are stable across a rename and a move), so its messages are unchanged
      // and re-IMPORTING them would be wrong — but every one of them now
      // materializes at a different path, and nothing else will ever say so.
      //
      // A fresh ENUMERATION is what re-emits them all at the new chain. The old
      // delta token is not kept alongside it: an enumeration covers everything
      // the link would have carried, so holding both would only replay the same
      // messages twice. That re-emission relocates anything at all only because
      // `items.js` folds the chain into the etag — without that,
      // `can_skip_unmapped` drops every one of them before rel_path is read.
      entries[fid] = { t: null, s: "enum", p: chain };
    } else {
      var carried = { t: old.t || null, s: old.s === "delta" ? "delta" : "enum", p: chain };
      if (old.u) carried.u = old.u;
      entries[fid] = carried;
    }
    if (fid !== rootId && (isNew || moved)) {
      var m = map[fid];
      folderItems.push({
        type: "updated",
        item: mailFolderItem(fid, m.name, chain, m.parentId, {
          display_name: m.display_name,
          total_item_count: m.total_item_count,
          is_hidden: m.hidden === true,
        }),
        relative_path: chain,
      });
    }
    order.push(fid);
  }
  // A folder that DISAPPEARED — deleted, or access to it lost — simply has no
  // entry carried over, and nothing is emitted for it. Its messages are removed
  // by the walk's reconcile, which is the only remover that can tell "gone"
  // from "no longer readable by this account".
  return {
    rootId: rootId,
    entries: entries,
    order: order,
    folderItems: folderItems,
  };
}

// `get_changes` for a mail mount spanning a folder SUBTREE.
//
// THE FOLDER MAP IS CACHED IN THE CURSOR, and that is a rate-limit fix, not a
// micro-optimisation. `buildFolderMap` costs 1 + (folders that have children)
// requests, and it used to run on EVERY call — including the idle poll, which is
// almost all of them. With a slice of 5 a 100-folder tree pays for one whole map
// build 20 times per completed round, which on a folder-heavy mailbox approaches
// two thousand `childFolders` requests per round for ONE mount, against a Graph
// ceiling of 10,000 requests per 10 minutes per app per mailbox. We have been
// rate-limited before by polling too hard; this was worse than any interval.
//
// So the map is rebuilt exactly twice as often as it has to be: when the
// rotation completes a full round (`r: null`), and when a chain LOOKUP MISSES —
// a message arriving from a folder the cached entries have never heard of. Every
// other call reads the chains straight out of the cursor, at zero requests. The
// cost is that a rename or a new folder is noticed at the END of the round it
// happened in rather than immediately; a round is bounded by
// `ceil(folders / MAIL_TREE_SLICE)` polls, and the walk's reconcile is
// unaffected either way.
export function mailTreeChanges(credential, mount, params) {
  var identity = cursorIdentity(mount, "mail");
  var stored = resolveTreeCursor(mount, params.since_token);
  var firstRound = stored === null;
  // `baseline_only` is only ever meaningful on the FIRST call: a stored cursor
  // already is a resume point.
  var baselineOnly = firstRound && params.baseline_only === true;

  // A non-empty `r` means "this round is still in progress, resume at this
  // folder" — and therefore also "the cached chains are still the ones this
  // round started with". `null` means the round closed, which is where the
  // rebuild lives.
  var resumeAt = stored && typeof stored.r === "string" && stored.r ? stored.r : null;
  var cachedRound = Boolean(resumeAt && stored.i);

  var st = {
    credential: credential,
    mount: mount,
    cached: cachedRound,
    rebuilt: false,
    rootId: null,
    entries: null,
    folderItems: [],
  };
  if (cachedRound) {
    st.rootId = stored.i;
    st.entries = carryEntries(stored.m);
  } else {
    var rec = reconcileEntries(
      mount,
      buildFolderMap(credential, mount),
      (stored && stored.m) || {},
      baselineOnly
    );
    st.rootId = rec.rootId;
    st.entries = rec.entries;
    st.folderItems = rec.folderItems;
  }

  var order = sortedIds(st.entries);
  if (baselineOnly || order.length === 0) {
    // A baseline fetches NOTHING: `capture_delta_baseline` discards the items
    // and keeps only the token, so pulling pages here would be pure cost.
    return {
      items: [],
      next_token: wrapTreeCursor(identity, null, st.rootId, st.entries),
      has_more: false,
    };
  }

  var n = order.length;
  // RESUME BY FOLDER ID, NEVER BY POSITION. `order` is sorted, so one folder
  // created or deleted since the last call shifts the index of every folder
  // after it — and a persisted index then skipped one folder or visited another
  // twice, silently, for as long as the mailbox kept changing. `>=` lands on the
  // persisted folder when it is still there and on the next one when it is not,
  // which is the only answer that neither skips nor repeats.
  var start = n;
  if (resumeAt) {
    for (var q = 0; q < n; q++) {
      if (order[q] >= resumeAt) {
        start = q;
        break;
      }
    }
    // Every remaining folder sorted BELOW the resume point: the round has
    // nothing left in it, so a new one starts here rather than burning a poll.
    if (start >= n) start = 0;
  } else {
    start = 0;
  }
  // The slice never wraps past the end of the order. Wrapping would let the
  // resume point drift so that a round never closed, and the "rotation came back
  // round" test — which is half of `has_more` — could then never be true.
  var slice = Math.min(MAIL_TREE_SLICE, n - start);

  // The order the SLICE was taken from, kept because a mid-call rebuild replaces
  // `order` and the resume point has to be derived from what was actually
  // visited, not from a position in a list that has since changed.
  var visitOrder = order;
  var out = [];
  var visited = [];
  for (var k = 0; k < slice; k++) {
    var visitId = visitOrder[start + k];
    var entry = st.entries[visitId];
    if (!entry) continue;
    var resp = fetchFolderDelta(credential, mount, entry, visitId);
    if (resp === null) {
      // ONE folder's link expired; the other N-1 are still good. Reseeded with a
      // plain enumeration and skipped for this call, so the rotation picks it up
      // next round.
      visited.push({ id: visitId, p: entry.p, t: null, s: "enum", u: null });
      continue;
    }
    var body = resp.body || {};
    // A changed message may arrive carrying only what changed; re-read those in
    // full before anything maps them, or the upsert writes nulls over real
    // values. Same hazard, same fix, as the single-folder path.
    var values = hydratePage(credential, mount, "mail", body.value || []);
    collectTreeChanges(out, values, st, visitId);
    var nextLink = body["@odata.nextLink"];
    var deltaLink = body["@odata.deltaLink"];
    var link = nextLink || deltaLink || null;
    var wantState = nextLink ? "enum" : "delta";
    var parsed = link ? linkToken(link) : null;
    if (parsed && parsed.s === wantState) {
      visited.push({ id: visitId, p: entry.p, t: parsed.t, s: wantState, u: null });
    } else if (link) {
      // A link whose token we could not read, or whose token kind disagrees with
      // the link kind: keep it WHOLE rather than rebuild a query from a guess.
      visited.push({ id: visitId, p: entry.p, t: null, s: wantState, u: link });
    }
  }

  // A lookup miss rebuilt the map mid-slice; its fresh entries (and the folder
  // items for whatever appeared) replace the cached ones.
  if (st.rebuilt) {
    order = sortedIds(st.entries);
    n = order.length;
  }
  for (var v = 0; v < visited.length; v++) {
    var got = visited[v];
    var target = st.entries[got.id];
    // Only if the folder still sits where it did when we polled it. A rebuild
    // that found this folder RENAMED has already re-seeded it with an
    // enumeration, and writing the token from the pre-rename link back over that
    // would replay the old link and leave every message at the old path.
    if (!target || target.p !== got.p) continue;
    target.t = got.t;
    target.s = got.s;
    if (got.u) target.u = got.u;
    else delete target.u;
  }
  out = st.folderItems.concat(out);

  enrichAttachments(
    credential,
    mount,
    out.filter(function (c) { return !c.item.is_folder; })
       .map(function (c) { return c.item; })
  );

  // THE RESUME POINT IS A FOLDER ID, resolved against the order as it stands
  // NOW — which is not necessarily the order the slice was taken from, because a
  // lookup miss may have rebuilt the map mid-call. "The first folder sorting
  // after the last one visited" is the only formulation that neither skips nor
  // repeats when the folder set changes underneath the rotation.
  var lastVisited = visitOrder[start + slice - 1];
  var nextR = null;
  for (var z = 0; z < n; z++) {
    if (order[z] > lastVisited) {
      nextR = order[z];
      break;
    }
  }
  var roundComplete = nextR === null;
  var midEnum = false;
  for (var e in st.entries) {
    if (Object.prototype.hasOwnProperty.call(st.entries, e) && st.entries[e].s === "enum") {
      midEnum = true;
      break;
    }
  }
  return {
    items: out,
    next_token: wrapTreeCursor(identity, nextR, st.rootId, st.entries),
    // Keep paging while any folder is still enumerating, or while the rotation
    // has not visited every folder this round. Stopping earlier is how the
    // quiet folders starve.
    has_more: midEnum || !roundComplete,
  };
}

// Fetch ONE folder's delta link, RESEEDING that folder alone when Graph rejects
// the link — never letting it invalidate the whole tree cursor.
//
// Graph expires a mail delta token and answers 410 / `syncStateNotFound` /
// `resyncRequired`; `http.js` maps all three to `cursor_invalid`. In folder mode
// that is exactly right: there is one link, so "this link is dead" and "this
// cursor is dead" are the same statement.
//
// IN TREE MODE THEY ARE NOT, AND LETTING THE ERROR OUT IS A WHOLE-MAILBOX
// RE-IMPORT. The engine recovers `cursor_invalid` by `state.last_sync_token =
// None` plus `full::run_with` (`phases.rs`) — so ONE of N folder links aging out
// discards the N-1 that were still valid AND re-walks every folder and every
// message in the subtree. On a real mailbox that is the most expensive thing
// this adapter can do, triggered by the most routine thing Graph does.
//
// A dead link for one folder needs exactly one repair: enumerate THAT folder
// again. Its messages come back with their current chain folded into their
// etags, so anything that moved while the link was dead is relocated, and
// anything unchanged is skipped by `can_skip_unmapped` at no write cost. The
// walk's reconcile remains the only remover either way, so nothing is deleted
// on the strength of a resynced folder.
//
// Returns null when the folder must be reseeded and skipped this call; the
// `enum` state its entry is left in keeps `has_more` true, so the rotation comes
// back to it rather than leaving it a round behind.
function fetchFolderDelta(credential, mount, entry, folderId) {
  try {
    return graphFetch(credential, "GET", entryUrl(mount, folderId, entry), {
      context: "get_changes:mail_tree",
      headers: outlookHeaders(mount),
    });
  } catch (e) {
    if (!e || e.code !== "cursor_invalid") throw e;
    return null;
  }
}

// The chain a folder's contents materialize under, read out of the entry table.
//
// The entries ARE the cached folder map: each carries the chain that folder
// resolved to when the map was last built, which is exactly what
// `mailRelativeChain` would answer. A MISS means the cached table has never
// heard of this folder — it was created since the round began — and that is one
// of the two things that force a rebuild, because the alternative is silently
// skipping every message in a brand-new folder until the round happens to close.
function chainFor(st, fid) {
  var e = st.entries[fid];
  if (e) return e.p;
  if (!st.cached || st.rebuilt) return null;
  st.rebuilt = true;
  var rec = reconcileEntries(
    st.mount,
    buildFolderMap(st.credential, st.mount),
    st.entries,
    false
  );
  st.rootId = rec.rootId;
  st.entries = rec.entries;
  st.folderItems = st.folderItems.concat(rec.folderItems);
  var fresh = st.entries[fid];
  return fresh ? fresh.p : null;
}

// One folder's delta page -> changes, with the path the WALK would have given
// each message.
function collectTreeChanges(out, values, st, fetchedFolderId) {
  for (var i = 0; i < values.length; i++) {
    var v = values[i];
    if (!v || !v.id) continue;

    if (v["@removed"]) {
      // NEVER A DELETE IN TREE MODE, and this is the load-bearing line of the
      // whole feature.
      //
      // Microsoft documents `@removed` with `"reason": "deleted"` as covering an
      // item that was deleted OR MOVED FROM the folder, as a collection-level
      // event (learn.microsoft.com message-delta). Filing a mail from Inbox into
      // Archive inside ONE tree mount therefore produces `@removed(id)` on
      // Inbox's feed and a create of the SAME id — immutable ids are on by
      // default — on Archive's feed, two independent feeds with no defined
      // ordering between them. Emitting the removal as `type: "deleted"` races
      // the create and destroys the node whenever the removal is processed
      // second: the node, its attachment children, its history and any local
      // annotation, lost to the single most ordinary thing a person does to an
      // email.
      //
      // So the walk's reconcile is the only remover here. THE TRADE, stated
      // plainly: a message moved OUT of the whole mount, or hard-deleted,
      // lingers until the next COMPLETE full walk — and `full_reconcile.rs`
      // skips reconcile entirely on a truncated walk, so on a big mailbox that
      // can be a long time. A stale node is recoverable; a destroyed one is not.
      //
      // Folder mode keeps its delete arm: the ambiguity is real there too, but
      // a one-folder mount has nowhere else inside itself for the message to go.
      continue;
    }

    // The message's OWN parent is authoritative — a message that moved INTO
    // this folder still arrives on this feed but already belongs to another —
    // and it is what the walk would use too. The fetched folder is the fallback
    // for a payload that carried no parentFolderId.
    var fid = v.parentFolderId || fetchedFolderId;
    var chain = chainFor(st, fid);
    if (chain === null) {
      // The message sits outside the mount root (moved away between the mint
      // and this page). Skipped, never placed at the root: the engine joins
      // relative_path to mount_path verbatim.
      continue;
    }
    var item = toExternalItem(v, "mail", st.mount, chain);
    out.push({
      type: "updated",
      item: item,
      relative_path: chain ? chain + "/" + item.external_id : item.external_id,
    });
  }
  return out;
}

export function opGetChanges(credential, mount, params) {
  var resource = resourceOf(mount);
  // A mail TREE mount holds one delta link PER FOLDER inside the single opaque
  // cursor, so it has its own path from here down rather than a branch inside
  // the single-link one.
  if (isMailTree(mount)) return mailTreeChanges(credential, mount, params);
  var token = params.since_token;
  // Only meaningful when there is no token yet — a stored token already IS a
  // resume point and must be used verbatim.
  var baselineOnly = !token && params.baseline_only === true;
  // A cursor still mid-enumeration keeps the mint time of the link that started
  // it, so a long paging catch-up cannot outrun the calendar window check.
  var wrapped = token ? resolveCursor(mount, resource, token) : null;
  var identity = cursorIdentity(mount, resource);
  var mintedMs = wrapped && typeof wrapped.m === "number" ? wrapped.m : Date.now();
  var url = wrapped ? wrapped.u : initialDeltaUrl(mount, resource, baselineOnly);
  var resp = graphFetch(credential, "GET", url, {
    context: "get_changes",
    headers: outlookHeaders(mount),
  });
  var body = resp.body || {};
  // A changed item may arrive carrying ONLY what changed; re-read those in full
  // before anything maps them, or the upsert writes nulls over real values.
  var values = hydratePage(credential, mount, resource, body.value || []);
  var items =
    resource === "calendar"
      ? calendarChanges(credential, mount, values)
      : flatChanges(credential, mount, resource, values);
  if (resource === "mail") {
    enrichAttachments(
      credential,
      mount,
      items.filter(function (c) { return c.type === "updated"; })
           .map(function (c) { return c.item; })
    );
  }
  // Durable, resumable cursor. While paging Graph returns @odata.nextLink; the
  // final page returns @odata.deltaLink. NEVER null: when nothing is new the
  // deltaLink round-trips, and we defensively echo the prior token/url otherwise.
  //
  // `has_more` tells the engine whether to KEEP PAGING NOW (a nextLink: this is
  // a mid-enumeration cursor) or stop (a deltaLink: caught up; the token is the
  // next run's resume point). The engine cannot infer this from the token
  // itself — Graph mints a fresh delta token on every poll of an idle feed, so
  // "the token stopped changing" never happens, and before this field the
  // delta loop spun empty deltaLink pages at request speed until the job
  // watchdog killed the run.
  var next = body["@odata.nextLink"] || body["@odata.deltaLink"] || url;
  return {
    items: items,
    // Wrapped with the identity of the query that minted it, so the next run can
    // tell whether the mount's projection or window has moved underneath it.
    next_token: wrapCursor(next, identity, mintedMs),
    has_more: Boolean(body["@odata.nextLink"]),
  };
}

// Mail and drive changes — everything whose delta entries map one-to-one onto
// items, which is every resource except calendar.
//
// The two differ in exactly one place, and it is the path. A MAIL node's
// relative path is its id and that is correct: the mount is one folder, the
// engine's full walk never builds a prefix for it, and a `path_template`
// reshapes it when an operator wants something else. A DRIVE is a tree, and a
// flat path there is a bug — see `paths.js`.
function flatChanges(credential, mount, resource, values) {
  var out = [];
  var isFiles = resource === "files";
  // Resolved LAZILY and once: it costs a request, and an idle poll — which is
  // most polls — has nothing to place.
  var rootSegments;
  var rootResolved = false;

  for (var i = 0; i < values.length; i++) {
    var v = values[i];

    // TWO removal vocabularies, not one. Outlook resources mark a deletion with
    // the `@removed` annotation; a driveItem marks it with a `deleted` FACET and
    // no annotation at all. Testing only for `@removed` meant every OneDrive and
    // SharePoint deletion arrived as an ordinary update, so files deleted at the
    // provider persisted in the workspace indefinitely — the walk's reconcile
    // being the only other thing that removes a node.
    //
    // A deletion keeps the ID as its path, on purpose: the engine's delete arm
    // matches on `external_id` and never reads the path, and a deleted item
    // carries no `parentReference` to derive one from anyway.
    if (v["@removed"] || (isFiles && v.deleted)) {
      out.push({ type: "deleted", item: { external_id: v.id, name: v.id }, relative_path: v.id });
      continue;
    }

    if (!isFiles) {
      var mailItem = toExternalItem(v, resource, mount);
      out.push({ type: "updated", item: mailItem, relative_path: mailItem.external_id });
      continue;
    }

    // Graph reports the container the delta is SCOPED TO as an item of that
    // delta. Emitting it materialized a stray folder node standing for the
    // mount, at the mount root, inside itself.
    if (isMountRootContainer(v, mount)) continue;

    if (!rootResolved) {
      rootSegments = mountRootSegments(credential, mount);
      rootResolved = true;
    }
    var rel = filesRelativePath(v, rootSegments);
    // Null means the item lives outside the mount root. Skipped rather than
    // placed: the engine joins `relative_path` to `mount_path` verbatim, so a
    // chain that does not start inside the root would write outside the mount.
    if (rel === null) continue;
    out.push({ type: "updated", item: toExternalItem(v, "files", mount), relative_path: rel });
  }
  return out;
}

// ONE NODE PER SERIES, not one per occurrence.
//
// The two calendar paths disagreed about what an item IS. The full walk reads
// `/events`, which returns single instances and SERIES MASTERS — one item per
// series, carrying `recurrence`. The delta path reads `/calendarView/delta`,
// which returns OCCURRENCES AND EXCEPTIONS expanded across the window — one
// item per instance, each with its own id and no `recurrence`. Since a node is
// keyed on the Graph id, a weekly meeting became ~5 nodes and a daily standup
// ~26, all siblings of the series-master node the full walk had already created
// for the same meeting, with nothing relating them.
//
// calendarView/delta is the only delta a v1.0 calendar has, so the fix is to
// collapse its output rather than abandon it: an unmodified OCCURRENCE is
// reported as an update of its `seriesMasterId`, deduped within the page.
//
// An EXCEPTION is NOT collapsed. It is a real divergence from the rule — a
// single occurrence moved, renamed or cancelled — and folding it into the
// master produced an "update" whose properties were byte-identical, so
// rescheduling one occurrence of a weekly meeting changed nothing observable in
// the data. It is emitted as its own item, carrying `seriesMasterId` and
// `originalStart`, which is what lets a consumer subtract that slot from the
// expanded series. Its master is emitted alongside it, so the node holding the
// recurrence rule exists before anything points at it. The full walk emits the
// same set (see `seriesExceptions`), or a reconcile would delete them.
//
// Two consequences worth stating:
//  * A single recurring series changing produces ONE update no matter how many
//    of its occurrences moved, unless those occurrences are exceptions.
//  * The master is fetched only when the page did not already contain it, so
//    the common case (a series edited as a whole) costs no extra request.
export function calendarChanges(credential, mount, values) {
  var out = [];
  var emitted = {};
  var i;

  function emit(v) {
    if (!v || !v.id || emitted[v.id]) return;
    emitted[v.id] = true;
    var item = toExternalItem(v, "calendar", mount);
    out.push({ type: "updated", item: item, relative_path: item.external_id });
  }

  // Series masters present in this page, so an occurrence of one of them needs
  // no extra fetch.
  var mastersInPage = {};
  for (i = 0; i < values.length; i++) {
    if (!values[i]["@removed"] && values[i].type === "seriesMaster") {
      mastersInPage[values[i].id] = values[i];
    }
  }

  for (i = 0; i < values.length; i++) {
    var v = values[i];

    if (v["@removed"]) {
      // A removal from calendarView is NOT necessarily a deletion. Microsoft
      // documents that within a date-bound view, `@removed` also covers events
      // that merely moved OUTSIDE the window — so treating every one as a delete
      // silently destroyed events an operator had only rescheduled. We cannot
      // tell the two apart from the delta payload, and deleting real content is
      // far worse than keeping a stale node, so only a removal we can attribute
      // to a whole series or a standalone event is acted on.
      //
      // A removed OCCURRENCE says nothing about its series: the series is still
      // there, and the next full walk reconciles anything genuinely gone.
      if (v.seriesMasterId) continue;
      out.push({ type: "deleted", item: { external_id: v.id, name: v.id }, relative_path: v.id });
      continue;
    }

    if (v.type === "occurrence" || v.type === "exception") {
      var masterId = v.seriesMasterId;
      if (!masterId) {
        // Shouldn't happen, but an occurrence with no master is better carried
        // through as itself than dropped.
        emit(v);
        continue;
      }
      // The master carries the recurrence rule, so it is emitted whether or not
      // the instance is. Skipped only when this page already emitted it.
      if (!emitted[masterId]) {
        var master = mastersInPage[masterId] || fetchEvent(credential, mount, masterId);
        // A master we cannot read (deleted between pages, or no access) is
        // skipped rather than materialized from the occurrence, which would
        // reintroduce exactly the per-occurrence nodes this exists to prevent.
        if (master) emit(master);
      }
      // The exception itself is the override; a plain occurrence adds nothing
      // the rule does not already say.
      if (v.type === "exception") emit(v);
      continue;
    }

    emit(v);
  }
  return out;
}

// Read one event by id, or null when it is gone. Used to resolve an occurrence
// back to its series master when the delta page did not include it.
export function fetchEvent(credential, mount, eventId) {
  var url = GRAPH + principal(mount) + "/events/" + enc(eventId) +
    "?$select=" + enc(eventSelect(mount));
  var resp = graphFetch(credential, "GET", url, {
    context: "get_changes:series_master",
    rawStatusOk: true,
    headers: outlookHeaders(mount),
  });
  if (resp.status === 404) return null;
  raiseForStatus(resp, "get_changes:series_master");
  return resp.body || null;
}
