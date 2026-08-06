// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The write operations: `create`, `update` and `delete` for a mirror mount.
//!
//! Every status a write diagnoses differently from a read is claimed in ONE
//! place, `diagnoseWrite`, so the three operations cannot drift on what a 403 or
//! a 412 means. The payload is always whatever the mount's MAPPER produced.

import { CAL, calFetch, coded, enc, raiseForStatus } from "./http.js";
import { calendarId } from "./mount.js";
import { toExternalItem } from "./items.js";

// ---- write (mirror) -------------------------------------------------------
//
// WHO GETS EMAILED IS A CONFIGURATION, AND ITS DEFAULT IS "NOBODY".
//
// Google mails every attendee when an event with attendees is created, moved or
// deleted, governed by `sendUpdates`. A sync engine mirroring a node is not a
// person deciding to notify twelve people — and the notification is irreversible
// and externally visible, the same property that makes an RSVP a `submit`
// command rather than a property edit. So this adapter sends `sendUpdates=none`
// unless the mount opts in via `sync_config.send_updates`, and a mount that
// wants invitations has to say so out loud.
export var SEND_UPDATES = { none: true, externalOnly: true, all: true };

export function sendUpdates(mount) {
  var cfg = (mount && (mount.config || mount.sync_config)) || {};
  var v = cfg.send_updates;
  return typeof v === "string" && SEND_UPDATES[v] ? v : "none";
}

export function eventUrl(mount, eventId) {
  return (
    CAL + "/calendars/" + enc(calendarId(mount)) + "/events" +
    (eventId ? "/" + enc(String(eventId).replace(/^\/+/, "")) : "")
  );
}

// Which OAuth scope a write needs, named in the 403 because the diagnosis is the
// whole value of that branch. `raiseForStatus` maps a non-rate-limit 403 to
// `auth_expired`, which sends the operator to reconnect the account — and
// reconnecting with the same read-only consent cannot fix a missing write scope.
// A wrong diagnosis costs more here than the failure did.
export var WRITE_SCOPE_HINT =
  "This is almost certainly a missing WRITE scope, not a stale token: the " +
  "connector requests read-only Calendar scopes by default. Add " +
  "https://www.googleapis.com/auth/calendar.events to the Google connector's " +
  "OAuth scopes in the console and RECONNECT each account — Google only issues " +
  "a widened scope on fresh consent.";

// Every status a WRITE diagnoses differently from a read, in ONE place, so the
// three operations cannot drift on what a 403 or a 412 means.
//
// Returns "gone" when the event no longer exists and the caller decides what
// that means; throws for everything terminal; falls through to the shared read
// mapping for 401/429/5xx, which is right for a write too.
export function diagnoseWrite(resp, context) {
  var status = resp.status;
  if (status >= 200 && status < 300) return null;
  var body = resp.body || {};
  var err = (body && body.error) || {};
  var message = err.message || "";
  var reason = "";
  try {
    reason = (err.errors && err.errors.length && err.errors[0].reason) || "";
  } catch (_) {
    reason = "";
  }

  // The event changed remotely since we read it. Resolved by the mount's
  // conflict policy and NEVER retried — the retry sends the same stale If-Match
  // and fails identically.
  //
  // The message text is load-bearing: `AdapterError::classify` scans for
  // auth_expired, rate_limited, cursor_invalid, config_error and THEN conflict,
  // so a conflict message containing any earlier token is misclassified.
  if (status === 412 || status === 409 || reason === "conditionNotMet") {
    throw coded(
      context + ": the event changed in Google Calendar since it was read (HTTP " +
        status + ")",
      "conflict"
    );
  }

  // GONE. Google answers 410 for an event that was already deleted and 404 for
  // one that never existed under this id.
  if (status === 404 || status === 410) return "gone";

  // A rate-limit 403 keeps the shared mapping; every other 403 on a WRITE is a
  // scope shortfall, which is the first thing a newly-writable mount hits.
  if (
    status === 403 &&
    reason !== "rateLimitExceeded" &&
    reason !== "userRateLimitExceeded" &&
    reason !== "dailyLimitExceeded"
  ) {
    throw coded(
      context + ": Google Calendar refused the write (403 " + (reason || "forbidden") +
        "). " + WRITE_SCOPE_HINT,
      "config_error"
    );
  }

  if (status === 400) {
    throw coded(
      context + ": " + (message || "Google Calendar rejected the request (400)"),
      "config_error"
    );
  }

  raiseForStatus(resp, context);
  return null;
}

export function isEmptyObject(v) {
  if (!v || typeof v !== "object") return true;
  for (var k in v) {
    if (Object.prototype.hasOwnProperty.call(v, k)) return false;
  }
  return true;
}

// Google's etag is a quoted token (`"abc123"`), and `toExternalItem` falls back
// to `updated` (an ISO timestamp) when one is absent — so a perfectly healthy
// node can carry a value that is not an etag at all. Sending that as If-Match is
// a 400, and 400 is TERMINAL, i.e. one such event would mark the whole mount
// misconfigured. The header therefore travels only for a value shaped like an
// etag; otherwise the write is last-write-wins, which is what it was before any
// etag was stored.
export var ETAG_SHAPE = /^(W\/)?"/;

export function writeHeaders(etag) {
  var headers = { "Content-Type": "application/json" };
  if (typeof etag === "string" && ETAG_SHAPE.test(etag)) {
    headers["If-Match"] = etag;
  }
  return headers;
}

export function writeReceipt(resp, fallbackId) {
  var body = resp.body || {};
  return {
    external_id: body.id || fallbackId || null,
    etag: body.etag || (resp.headers && resp.headers["etag"]) || null,
  };
}

// POST one new event from the payload the MAPPER produced, forwarded verbatim —
// the adapter never re-derives the node-to-Google field mapping, or a custom
// mapper pointed at the same mount would silently disagree with it.
export function opCreate(credential, mount, params) {
  params = params || {};
  if (isEmptyObject(params.payload)) {
    throw coded("create: refusing an empty event body", "config_error");
  }
  // `parent_id` is the engine's word for the mount's remote root. Google
  // addresses a calendar in the PATH, so it is the calendar id, and falling back
  // to `calendarId(mount)` keeps a primary-calendar mount (which has no
  // remote_root) working rather than posting to an empty path segment.
  var calId =
    typeof params.parent_id === "string" && params.parent_id
      ? params.parent_id
      : calendarId(mount);
  var url =
    CAL + "/calendars/" + enc(calId) + "/events?sendUpdates=" + enc(sendUpdates(mount));

  var resp = calFetch(credential, "POST", url, {
    context: "create",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(params.payload),
    rawStatusOk: true,
  });

  // A 404 on a CREATE is the CALENDAR, not the event — there is no event yet to
  // be gone. Terminal and named: the recovery is an operator pointing
  // remote_root at a calendar that exists, and returning null here would tell
  // the engine the create succeeded with no id.
  if (diagnoseWrite(resp, "create") === "gone") {
    throw coded(
      "create: calendar '" + calId + "' does not exist or is not accessible to this " +
        "account. Check the mount's remote_root.",
      "config_error"
    );
  }

  // The id is the whole point of the call — without it the engine cannot adopt
  // the node — so an absent one is diagnosed here rather than passed on as null.
  var created = resp.body || {};
  if (!created.id) {
    throw coded(
      "create: Google Calendar accepted the event (HTTP " + resp.status +
        ") but returned no id, so the new event cannot be matched to its node",
      "transient"
    );
  }
  return writeReceipt(resp, null);
}

// PATCH one event with the payload the mapper produced. PATCH, not PUT: an
// update carries an allow-listed subset of fields, and PUT would clear every
// field the mount does not push.
export function opUpdate(credential, mount, params) {
  params = params || {};
  if (!params.item_id) {
    throw coded("update: params.item_id is required", "config_error");
  }
  if (isEmptyObject(params.payload)) {
    // An empty PATCH still bumps the event's etag, invalidating every stored
    // one and making the next delta re-deliver the event for no reason. The
    // mapper already returns null rather than emit one; this is the second gate.
    throw coded("update: refusing an empty PATCH body", "config_error");
  }

  var resp = calFetch(
    credential,
    "PATCH",
    eventUrl(mount, params.item_id) + "?sendUpdates=" + enc(sendUpdates(mount)),
    {
      context: "update",
      headers: writeHeaders(params.etag),
      body: JSON.stringify(params.payload),
      rawStatusOk: true,
    }
  );

  // A vanished event SETTLES the node rather than failing it: the delta feed
  // reports the deletion and the engine removes the node on its own schedule.
  // Failing instead would retry a doomed PATCH on every drain forever.
  if (diagnoseWrite(resp, "update") === "gone") return null;
  return writeReceipt(resp, params.item_id);
}

// DELETE one event.
//
// Google has NO trash for calendar events — a delete is immediate and the event
// is not recoverable from any bin — which is why `capabilities` declares
// `supports_trash: false` and defaults the policy to `detach`. The engine then
// refuses a mount configured for `trash` at resolution time, before any delete
// is attempted, and an operator who wants deletes to propagate has to type
// `purge`. That is the intended shape: the irreversible option is never the
// default and never reached by accident.
export function opDelete(credential, mount, params) {
  params = params || {};
  if (!params.item_id) {
    throw coded("delete: params.item_id is required", "config_error");
  }
  // The SECOND gate. The engine already refuses a `trash` mount at policy
  // resolution — before any delete is attempted — precisely because promoting it
  // to a purge would turn a recoverable delete into an irreversible one. This
  // catches an engine that resolved against a stale cached capability record.
  if (params.policy === "trash") {
    throw coded(
      "delete: Google Calendar has no trash for events — a delete is immediate and " +
        "irreversible. Set the mount's delete_policy to 'purge' (which is what this " +
        "provider can actually do) or to 'detach'.",
      "config_error"
    );
  }

  var resp = calFetch(
    credential,
    "DELETE",
    eventUrl(mount, params.item_id) + "?sendUpdates=" + enc(sendUpdates(mount)),
    {
      context: "delete",
      headers: writeHeaders(params.etag),
      rawStatusOk: true,
    }
  );

  // Already gone is SUCCESS. A delete is the one operation whose desired end
  // state a 404/410 already satisfies, and failing it would leave the engine
  // retrying forever against an id that can never come back — which is the
  // COMMON case here, because Google answers 410 for an event deleted earlier.
  diagnoseWrite(resp, "delete");
  return { external_id: params.item_id, deleted: true };
}
