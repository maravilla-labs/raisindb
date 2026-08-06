// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! What this adapter declares it can do. Read by the engine once per run to
//! resolve a mount's write mode.

import { WATCH_TTL_SECONDS } from "./http.js";
import { opCreate, opDelete, opUpdate } from "./write.js";

export function opCapabilities() {
  return {
    can_read: true,
    // The full MIRROR surface: create, update and delete are all implemented
    // (`opCreate` / `opUpdate` / `opDelete`). `can_submit` is deliberately
    // ABSENT — an RSVP through Google is a PATCH of the caller's own attendee
    // row rather than a distinct action endpoint, and declaring a command
    // surface with no implementation behind it is how an outbox mount resolves
    // to a mode that throws at drain time.
    can_write: true,
    can_create: true,
    can_update: true,
    can_delete: true,
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
