// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! What this adapter declares it can do. Read by the engine once per run to
//! resolve a mount's write mode.

// The unused `opCreate/opDelete/opUpdate` import that used to sit here was
// removed. It read as link-time proof that the three operations exist — which
// is exactly what a reader wants from a capabilities file and exactly what an
// import does NOT provide; the dispatch table in index.js is the only thing
// that connects a declaration to an implementation.
import { WATCH_TTL_SECONDS } from "./http.js";

export function opCapabilities() {
  return {
    can_read: true,
    // The full MIRROR surface: create, update and delete are all implemented
    // (`write.js`), and dispatched by index.js.
    can_write: true,
    can_create: true,
    can_update: true,
    can_delete: true,
    // The COMMAND surface: an RSVP against a raisin:CalendarAction on a
    // `submit` mount, implemented in submit.js and paired with the
    // google-calendar-outbox mapper.
    //
    // This was absent for as long as there was no implementation, and that was
    // the right call: a capability with nothing behind it makes a mount resolve
    // as capable and then throw at drain time, with a command already claimed.
    // It is declared here in the same change that adds submit.js, never before.
    //
    // Google has no idempotency header for events.patch, so
    // `supports_idempotency_key` stays absent (false): at-most-once rests
    // entirely on the engine's durable claim.
    can_submit: true,
    // NODE property names, not Google's. The engine intersects this with the
    // mount's `write_config.mutable_fields` and hands the survivors to the
    // MAPPER as `fields`; the Google spelling is the mapper's business.
    //
    // Absent by design: `status` (cancelling is a delete under the mount's
    // delete_policy), `my_response` (an RSVP notifies the organizer and must not
    // hide behind a property edit), and everything Google computes —
    // ical_uid, url, organizer_*, online_meeting_url.
    mutable_fields: [
      "title",
      "description_html",
      "description_text",
      "start_local",
      "start_utc",
      "end_local",
      "end_utc",
      "timezone",
      "all_day",
      "location",
      "attendees",
      "show_as",
      "recurrence",
    ],
    // Google has NO trash for calendar events: a delete is immediate and
    // unrecoverable. So `trash` is not offerable and the default is `detach` —
    // local deletes do not propagate unless an operator types `purge`. Declaring
    // trash here would let a mount configure a soft delete this provider cannot
    // perform, and the engine would report success for it.
    supports_trash: false,
    default_delete_policy: "detach",
    can_create_folders: false,
    supports_changes: true,
    supports_search: false,
    // calendarList discovery (browse.js). Without it the console falls back to
    // manual entry and an operator has to hand-type a calendar id for anything
    // but `primary`. Needs no scope the read path does not already hold.
    supports_browse: true,
    supports_push: true,
    // ONE declaration. This object used to carry supports_webhooks twice with
    // contradicting values; JS last-wins made the effective answer `true`, so
    // push worked purely by key order and any reformat would have silently
    // turned it off for every Google calendar mount.
    supports_webhooks: true,
    default_ttl: WATCH_TTL_SECONDS,
    max_file_size: null,
  };
}
