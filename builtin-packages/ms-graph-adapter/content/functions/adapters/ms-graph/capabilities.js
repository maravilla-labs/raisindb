// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! What this adapter declares it can do, per resource. Read by the engine once
//! per run to resolve a mount's write mode.

import { calendarSupportsDelta, resourceOf } from "./mount.js";
import { opCreate, opDelete, opUpdate } from "./write.js";

// Capabilities are RESOURCE-dependent, and both dependent fields are here:
// `supports_changes` (a non-primary calendar has no delta feed) and the write
// flags (only mail has an `update`).
//
// The write flags are declared PER RESOURCE. Declaring them adapter-wide would
// let a mount resolve to a writable mode and then discover at drain time that
// every push throws — after the engine had already claimed the candidates.
//
// `mutable_fields` holds the NODE property name (`unread`), not the Graph name
// (`isRead`): the engine intersects it with the mount's
// `write_config.mutable_fields`, which is authored in node terms, and passes the
// survivors to the MAPPER as `fields`. The Graph spelling is the mapper's
// business and appears nowhere in this file.
//
// Declared per RESOURCE, and only for what is actually implemented — the
// engine's serde defaults every write flag to false, so silence is read as
// read-only. Mail gets update + submit; calendar and files get the full mirror
// set (files plus `accepts_content`, which is what makes the engine send bytes
// at all); calendar also gets submit. Declaring a capability with no
// implementation behind it is how a mount resolves to a mode that throws at
// drain time, after the engine has already claimed its candidates.
export function opCapabilities(mount) {
  var resource = resourceOf(mount);
  var deltaOk = resource !== "calendar" || calendarSupportsDelta(mount);
  var caps = {
    can_read: true,
    can_write: false,
    can_create_folders: false,
    supports_changes: deltaOk,
    supports_webhooks: true,
    supports_search: false,
    supports_push: true,
    supports_browse: true,
    default_ttl: null,
    max_file_size: null,
  };
  if (resource === "mail") {
    caps.can_write = true;
    caps.can_update = true;
    // TWO spellings of the one writable flag, on purpose. `unread` is the
    // canonical raisin:Mail column; `is_read` is its Graph-truth alias
    // (is_read === !unread), accepted because it is the natural snake_case of
    // Graph's own `isRead` and a mount declared with it used to die at the
    // engine's intersection with this list — WriteMode::Refused, no push ever
    // attempted, the reason visible only in writeback_last_error. The mail
    // mapper folds both into one `isRead` payload key, so declaring both here
    // never produces two conflicting writes.
    // `importance` joins the two read-flag spellings: Graph accepts it on the
    // same message PATCH, from the same closed set it reports. The follow-up
    // FLAG is deliberately not here — it is imported (see `flags`) but writing
    // it means writing a flag OBJECT with status and dates, not a value, and a
    // half-specified flag is worse than none.
    caps.mutable_fields = ["unread", "is_read", "importance"];
    // The OUTBOX half. `can_submit` is a fact about the adapter, not about the
    // mount: the same declaration serves a `state_only` inbox that never issues
    // a command and a `submit` outbox that does nothing else. The MOUNT's
    // write_config.mode is what chooses.
    caps.can_submit = true;
  }
  if (resource === "calendar") {
    // A full MIRROR surface — create, update and delete are all implemented
    // (`opCreate` / `opUpdate` / `opDelete`) — plus RSVP as a `submit` command.
    caps.can_write = true;
    caps.can_create = true;
    caps.can_update = true;
    caps.can_delete = true;
    caps.can_submit = true;
    // NODE property names, not Graph's. The engine intersects this with the
    // mount's `write_config.mutable_fields` and hands the survivors to the
    // MAPPER as `fields`; the Graph spelling is the mapper's business and
    // appears nowhere in this file.
    //
    // What is deliberately NOT here is everything Graph owns or computes:
    // organizer, ical_uid, url, online_meeting_url, my_response and `status`.
    // Sending them is a 400 at best and a silent no-op at worst — and the two
    // that look writable are the dangerous ones: cancelling is a DELETE under
    // the mount's delete_policy, and an RSVP is a `submit` command precisely
    // because it notifies the organizer and must not hide behind a property
    // edit.
    caps.mutable_fields = [
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
    ];
    // Graph's DELETE moves an event to Deleted Items and offers no permanent
    // delete at all, so `trash` is the honest default AND the only policy this
    // adapter can serve; `opDelete` refuses `purge` rather than quietly
    // soft-deleting and reporting success.
    caps.supports_trash = true;
    caps.default_delete_policy = "trash";
  }
  if (resource === "files") {
    // A full MIRROR surface over driveItems: `driveCreate` / `driveUpdate` /
    // `driveDelete` in drive.js, plus `finalize_upload` for the bytes the
    // ENGINE streams when a file is too large to pass inline.
    caps.can_write = true;
    caps.can_create = true;
    caps.can_update = true;
    caps.can_delete = true;
    // A locally-created FOLDER becomes a real one at the provider. Declared
    // only now that `driveCreate` has a folder branch: this flag is what makes
    // the engine offer raisin:Folder as a creatable type, and offering it with
    // nothing behind it is how a mount resolves as capable and then throws at
    // drain time.
    caps.can_create_folders = true;
    // THE BYTE CHANNEL. Without this the engine sends metadata only, and a
    // "mirrored" file would arrive at OneDrive as a name with no content —
    // which is exactly what this surface did before the write path existed.
    caps.accepts_content = true;
    // NOT declared: `can_create_folders`. Graph can create one, this adapter
    // does not, and the gap is visible rather than papered over — a mirror mount
    // can only create files inside folders the walk already imported.
    //
    // NOT declared either: `can_submit`. A drive has no command surface.
    //
    // NODE property names. `title` is what the files mapper reads to produce the
    // driveItem `name`; the Graph spelling appears nowhere in this file. A
    // driveItem's other writable metadata (description, its parent — a MOVE) is
    // left out until it is implemented, for the reason above.
    caps.mutable_fields = ["title"];
    // Graph's DELETE moves a drive item to the recycle bin, where a user can
    // restore it, and v1.0 offers no permanent delete for one item — so `trash`
    // is the honest default AND the only policy this adapter can serve.
    // `driveDelete` refuses `purge` rather than soft-deleting and reporting
    // success.
    caps.supports_trash = true;
    caps.default_delete_policy = "trash";
  }
  // Graph has NO idempotency key for sendMail, reply, forward or an event
  // response. Declared honestly and never as an aspiration: the engine's
  // at-most-once guarantee rests on its own durable claim, and the only thing a
  // false `true` here would change is what an operator believes about the
  // duplicate they are looking at.
  caps.supports_idempotency_key = false;
  return caps;
}
