// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! `submit`: issuing an RSVP as a COMMAND rather than mirroring a property.
//!
//! The engine's submit drain sends
//!   { payload: { action, body }, external_id, idempotency_key }
//! for a raisin:CalendarAction node, exactly as it does for ms-graph. This is
//! the Google half of that surface.

import { calFetch, coded, enc } from "./http.js";
import { calendarId } from "./mount.js";
import { diagnoseWrite, eventUrl, writeHeaders } from "./write.js";

// The node's vocabulary -> Google's `responseStatus`. Google has no
// accept/decline/tentative ENDPOINT at all (unlike Graph's /accept, /decline,
// /tentativelyAccept): an RSVP is an events.patch of the caller's own attendee
// row, which is why this operation looks nothing like its Graph twin.
export var RSVP_STATUS = {
  accept: "accepted",
  decline: "declined",
  tentative: "tentative",
};

// WHY THIS IS A READ-MODIFY-WRITE AND NOT A ONE-LINE PATCH.
//
// events.patch documents: "Array fields, if specified, overwrite the existing
// arrays; this discards any previous array elements." `attendees` is an array
// field. So PATCHing `{ attendees: [ <just my row> ] }` — the obvious way to
// write one response — REMOVES EVERY OTHER GUEST from the meeting, silently and
// irreversibly, and mails them all about it. The event must therefore be read
// first, the row with `self === true` mutated in place, and the WHOLE array
// sent back.
//
// The cost is two round trips instead of one. The GET is idempotent and safe to
// repeat; the PATCH is the single ambiguous window, the same shape as ms-graph's
// one-call RSVP. `supports_idempotency_key` stays false in `capabilities`
// because Google offers no such header — inventing one would be a lie the
// engine's at-most-once accounting would then rely on.
export function opSubmit(credential, mount, params) {
  params = params || {};
  var payload = params.payload || {};
  var action = payload.action;
  if (!action) {
    throw coded("submit: params.payload.action is required", "config_error");
  }
  var responseStatus = RSVP_STATUS[action];
  if (!responseStatus) {
    throw coded(
      "submit: unsupported calendar action '" + action +
        "' (expected accept, decline or tentative)",
      "config_error"
    );
  }
  var eventId = params.external_id;
  if (!eventId) {
    throw coded(
      "submit: an RSVP needs the event's provider id (target_external_id)",
      "config_error"
    );
  }
  var body = payload.body || {};

  // ---- read ---------------------------------------------------------------
  var url = eventUrl(mount, eventId);
  var read = calFetch(credential, "GET", url, {
    context: "submit(read)",
    rawStatusOk: true,
  });
  // A vanished event is TERMINAL here and is NOT the `null` that `update`
  // returns for the same status. On the update path a 404 settles the node and
  // the delta re-imports it; there is no such recovery for a command — the
  // meeting being responded to is gone, so this RSVP can never be issued as
  // written. Returning null would park it at `unknown`, i.e. tell the operator
  // we might have answered an invitation, which is strictly false.
  if (diagnoseWrite(read, "submit") === "gone") {
    throw coded(
      "submit: event '" + eventId + "' no longer exists in calendar '" +
        calendarId(mount) + "', so the RSVP cannot be sent",
      "config_error"
    );
  }
  var event = read.body || {};
  var attendees = event.attendees;
  if (!Array.isArray(attendees) || !attendees.length) {
    throw coded(
      "submit: event '" + eventId + "' has no attendee list, so this account is not " +
        "invited to it and has nothing to respond to",
      "config_error"
    );
  }

  // ---- modify -------------------------------------------------------------
  // `self` is Google's marker for the row belonging to the calendar the request
  // is made against — the only way to identify it, since the adapter never sees
  // the account's own address. No self row means the principal is not an
  // attendee (typically: it is the ORGANIZER, who has nothing to RSVP to, or
  // the mount points at someone else's calendar). Named rather than silently
  // no-op'd, because a command that reports `sent` without sending is the worst
  // outcome the submit protocol can produce.
  var next = [];
  var found = false;
  for (var i = 0; i < attendees.length; i++) {
    var row = attendees[i];
    if (row && row.self === true && !found) {
      found = true;
      var mine = {};
      for (var k in row) {
        if (Object.prototype.hasOwnProperty.call(row, k)) mine[k] = row[k];
      }
      mine.responseStatus = responseStatus;
      if (body.comment) mine.comment = String(body.comment);
      next.push(mine);
      continue;
    }
    next.push(row);
  }
  if (!found) {
    throw coded(
      "submit: no attendee row marked `self` on event '" + eventId + "' in calendar '" +
        calendarId(mount) + "' — this account is not an attendee (it may be the " +
        "organizer, who has nothing to respond to). Nothing was sent.",
      "config_error"
    );
  }

  // ---- write --------------------------------------------------------------
  // sendUpdates comes from the COMMAND, not from `write.js`'s sendUpdates(mount).
  // That helper defaults every mirror write to "none" on purpose — a sync engine
  // mirroring a node is not a person deciding to mail twelve people. An RSVP is
  // the opposite case: telling the organizer IS the RSVP, so a response nobody
  // is told about is a no-op with a green tick on it. Default true, overridable
  // per command via raisin:CalendarAction.send_response, which the mapper puts
  // in `body.send_response`.
  var notify = body.send_response !== false ? "all" : "none";
  var resp = calFetch(credential, "PATCH", url + "?sendUpdates=" + enc(notify), {
    context: "submit",
    // IF-MATCH, from the event this call just read.
    //
    // This PATCH replaces the WHOLE attendee array, so anyone added to the
    // meeting between the GET above and this request is not in the array being
    // sent and would be DELETED by it — the same "array fields overwrite the
    // existing arrays" behaviour the read-modify-write exists to survive,
    // narrowed to the read-write window. The etag closes that window: Google
    // rejects a stale one with 412, `diagnoseWrite` reports it as `conflict`,
    // and `submit_outcome.rs` (`disposition`) makes Conflict TERMINAL — the
    // command fails visibly and a person requeues it, rather than the RSVP
    // quietly dropping a guest. `writeHeaders` sends the header only for a
    // value shaped like an etag, so an event answered with none is
    // last-write-wins exactly as before.
    headers: writeHeaders(event.etag),
    body: JSON.stringify({ attendees: next }),
    rawStatusOk: true,
  });
  // Same diagnosis table as the mirror path — a 403 (missing calendar.events
  // scope) and a 400 must not be read differently here than there.
  //
  // A 5xx needs no branch of its own, unlike the ms-graph twin: `raiseForStatus`
  // leaves it a PLAIN Error, which the engine reads as Transient and the submit
  // drain PARKS at `unknown` — never auto-retried. That is the right default for
  // a command whose outcome is genuinely unknown (a gateway timeout on a PATCH
  // often means the PATCH landed and the organizer was already mailed). Do not
  // "improve" it into rate_limited, which is the one disposition that re-sends.
  if (diagnoseWrite(resp, "submit") === "gone") {
    throw coded(
      "submit: event '" + eventId + "' disappeared between reading it and answering " +
        "it; the RSVP was not sent",
      "config_error"
    );
  }

  var updated = resp.body || {};
  return {
    external_id: updated.id || eventId,
    etag: updated.etag || (resp.headers && resp.headers["etag"]) || null,
  };
}
