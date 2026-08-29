// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The incremental feed: the first delta URL, `get_changes`, and the collapse
//! that keeps a recurring series ONE node instead of one per occurrence.

import { coded, enc } from "./common.js";
import { GRAPH, graphFetch, raiseForStatus } from "./http.js";
import { calendarSupportsDelta, driveContainer, eventSelect, mailFolderId, mailSelect, outlookHeaders, principal, resourceOf, useImmutableIds, windowBounds, windowConfig } from "./mount.js";
import { enrichAttachments } from "./mail.js";
import { toExternalItem } from "./items.js";
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
  return (
    GRAPH + principal(mount) + "/mailFolders/" + enc(mailFolderId(mount)) +
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
  return "mail|" + principal(mount) + "|" + mailFolderId(mount) + "|" + ids +
    "|sel=" + mailSelect(mount);
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

export function opGetChanges(credential, mount, params) {
  var resource = resourceOf(mount);
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
      out.push({ type: "deleted", item: { external_id: v.id }, relative_path: v.id });
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
      out.push({ type: "deleted", item: { external_id: v.id }, relative_path: v.id });
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
