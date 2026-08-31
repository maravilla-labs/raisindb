// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Discovery for the mount editor. Never called during sync.
//!
//! Without this operation the console falls back to MANUAL ENTRY
//! (`browse.rs` reads "Unsupported operation" as "connector without
//! discovery"), which means an operator mounting anything but `primary` has to
//! hand-type an opaque Google calendar id — and a typo is indistinguishable
//! from a permission problem at sync time.

import { CAL, calFetch, coded, enc } from "./http.js";

// The shape `parse_browse` in browse.rs deserializes. `id` must be non-empty or
// the item is dropped; the rest default.
export function browseItem(id, name, kind, hasChildren, hint) {
  return {
    id: id,
    name: name || id,
    kind: kind,
    has_children: hasChildren === true,
    hint: hint || null,
  };
}

// The page size cap is Google's, not ours: calendarList.list documents a
// maximum of 250 entries per page.
export function browseLimit(params) {
  return params && params.limit && params.limit > 0
    ? Math.min(params.limit, 250)
    : 50;
}

// Which calendar, in the operator's words rather than in Google's.
//
// `accessRole` is what decides whether a mount can be more than read-only —
// only `writer` and `owner` may create or PATCH events — so it belongs in the
// hint, where a person picking a calendar for a MIRROR mount can see it before
// the first write 403s. `primary` is called out because it is the one id that
// needs no discovery at all.
export function calendarHint(entry) {
  var role = entry.accessRole || "";
  if (entry.primary === true) return role ? "primary · " + role : "primary";
  return role || null;
}

export function opBrowse(credential, mount, params) {
  params = params || {};
  var kind = params.kind || "calendar";
  // ONE kind. A Google calendar has no sub-structure to drill into: the mount
  // root is a calendar id and nothing else, so `has_children` is false
  // everywhere and there is no `parent_id` walk. Throwing on any other kind is
  // deliberate — silently answering an empty list would read as "this account
  // has no calendars".
  if (kind !== "calendar") {
    throw coded("browse: unsupported kind '" + kind + "'", "config_error");
  }

  // `calendarList` (the account's own list of calendars, including accepted
  // shares) rather than `calendars` — there is no endpoint that enumerates
  // `calendars`, only one that reads a single one by id. Covered by
  // calendar.readonly, so browsing needs no scope the read path does not
  // already have.
  var url =
    CAL + "/users/me/calendarList?maxResults=" + browseLimit(params);
  if (params.cursor) url += "&pageToken=" + enc(params.cursor);

  var resp = calFetch(credential, "GET", url, { context: "browse" });
  var body = resp.body || {};
  var entries = body.items || [];
  var items = [];
  for (var i = 0; i < entries.length; i++) {
    var entry = entries[i] || {};
    if (!entry.id) continue;
    // `deleted` entries are tombstones of removed subscriptions; mounting one
    // produces a mount that 404s on its first list.
    if (entry.deleted === true) continue;
    items.push(
      browseItem(
        entry.id,
        // summaryOverride is the name the OPERATOR gave the calendar in their
        // own list; it wins over the owner's `summary` because it is the label
        // they will be looking for in the picker.
        entry.summaryOverride || entry.summary,
        "calendar",
        false,
        calendarHint(entry)
      )
    );
  }
  // A page token, not a full URL: Google pages by token and the next request is
  // rebuilt from it above, so the cursor stays opaque to the console.
  return { items: items, next_cursor: body.nextPageToken || null };
}
