// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Reading the MOUNT: which Graph surface it targets, whose mailbox/drive it
//! addresses, and which projection ($select) its requests carry.
//!
//! Every route in the adapter is built from these, which is what stops the list,
//! get, delta and write paths from disagreeing about the principal — a
//! disagreement that reports healthy and reads the wrong mailbox forever.

import { coded, enc, encSiteId } from "./common.js";
import { raiseForStatus } from "./http.js";
import { calendarMeta, onlineMeetingUrl } from "./calendar.js";
import { filesMeta } from "./items.js";
import { subscriptionResource } from "./subscribe.js";
import { initialDeltaUrl } from "./changes.js";

// Message fields fetched so ExternalItem/metadata can be built in one call. Graph
// preserves $select inside the delta/next links, so the delta feed keeps them too.
//
// WIDENED for the mail write path. `bccRecipients` and `replyTo` are only ever
// populated on a message the account itself sent, but they are what lets a
// synced Sent Items copy seed a reply-all without silently dropping recipients.
// `internetMessageHeaders` is NOT requested: Graph returns it only on a
// single-message GET and it is large; `In-Reply-To`/`References` come from
// `conversationId` + the per-message fetch instead.
//
// Widening the projection does NOT reach a mount that already holds a stored
// delta link — Graph freezes $select inside delta/next links. Recovering these
// fields on an existing mail mount requires discarding its cursor so the full
// walk re-materializes.
export var MAIL_SELECT =
  "id,subject,from,toRecipients,ccRecipients,bccRecipients,replyTo," +
  "receivedDateTime,sentDateTime,bodyPreview,isRead,isDraft,hasAttachments," +
  "importance,categories,conversationId,webLink,internetMessageId," +
  "createdDateTime,lastModifiedDateTime";

// Whether this mount wants attachment METADATA materialized as raisin:Asset
// child nodes (`sync_config.include_attachments`).
//
// Off by default: listing attachments is one extra request PER MESSAGE THAT HAS
// THEM — Graph has no way to expand them across a page — so on a mailbox-sized
// import it can double the request count. `hasAttachments` is always available
// without it, so a mount that only needs the flag pays nothing.
//
// Never fetches BYTES. `$select` on the attachment listing omits `contentBytes`
// deliberately: the engine's on-demand `get_content` fetch is what downloads a
// blob, once, when something asks for it.
export function includeAttachments(mount) {
  var v = configValue(mount, "include_attachments");
  return v === true || v === "true";
}

// ---- immutable Outlook ids ------------------------------------------------

// Ask Graph for IMMUTABLE ids on Outlook resources (mail, events).
//
// Graph's DEFAULT message id is derived from the item's location, so MOVING a
// message between folders changes its id. A virtual mount keys each node on
// `external_id` for the node's whole lifetime, so with default ids a user
// filing a mail into a subfolder is reported as a DELETE of the old id plus a
// CREATE of a new one: the node is destroyed and rebuilt, taking its
// attachment subnodes, its history and any local annotation with it, and any
// pending writeback against the old id 404s. Immutable ids are Microsoft's
// answer to exactly this, and they cost one request header.
//
// DEFAULT ON, because the alternative is a silent data-loss path on the single
// most ordinary thing a person does to an email. Opt out per mount/connection
// with `immutable_ids: false` — see the migration note below before you do.
//
// MIGRATION — read before flipping this on an existing mount. Immutable ids are
// a DIFFERENT id space, so a mount that already synced with default ids will
// not recognise its own items: the next full reconcile imports every message
// afresh under its immutable id and prunes the old nodes. That is a one-time
// re-import — node ids change, per-node history restarts, attachments are
// re-fetched — and on a mount with writeback enabled the re-imported nodes are
// unseeded, so the first drain pushes their current values back once (bounded
// by `max_items_per_sync` per run, idempotent). Nothing is lost at the
// PROVIDER. To defer the re-import, set `immutable_ids: false` on that mount
// and plan the migration; new mounts should always run with it on.
export function useImmutableIds(mount) {
  // OUTLOOK resources only. A driveItem id is already stable across moves and
  // renames, and Graph has no immutable-id notion for drives — sending the
  // header there would be noise on every request of the busiest resource.
  var resource = resourceOf(mount);
  if (resource !== "mail" && resource !== "calendar") return false;
  var v = configValue(mount, "immutable_ids");
  if (v === false || v === "false") return false;
  return true;
}

// Headers for a request against an Outlook resource, merged with anything the
// caller already sends. ONE helper so the read paths, the write path and the
// attachment fetch cannot disagree about which id space they are in — a mount
// that listed with immutable ids and PATCHed with default ones would 404 on
// every write it attempted.
export function outlookHeaders(mount, extra) {
  var headers = {};
  if (extra) {
    for (var k in extra) headers[k] = extra[k];
  }
  if (useImmutableIds(mount)) {
    headers["Prefer"] = 'IdType="ImmutableId"';
  }
  return headers;
}

// Whether this mount wants the FULL message body inline (sync_config.include_body).
//
// Off by default and deliberately so: mail syncs link-only, and adding `body`
// to $select multiplies the delta payload by whole HTML documents on every page
// of every run — on a mailbox-sized mount that is the difference between a few
// hundred KB and tens of MB. Turn it on when you want fulltext over the body.
export function includeBody(mount) {
  var v = configValue(mount, "include_body");
  return v === true || v === "true";
}

// $select for one mail request, widened when the mount opted into bodies.
// Kept as a function rather than a second constant so the flag is read at every
// call site and the delta and list paths cannot disagree about it.
export function mailSelect(mount) {
  return includeBody(mount) ? MAIL_SELECT + ",body" : MAIL_SELECT;
}

// Event fields the mapper actually reads, plus the recurrence discriminators.
//
// The calendar list used to send no $select at all, so Graph returned the FULL
// event — `body` (an HTML document) and `bodyPreview` for every event, none of
// which the mapper uses. With $top driven by `max_items_per_sync` (default 500)
// that is 500 full events in one response, which Microsoft explicitly warns can
// trigger a gateway timeout. `type` and `seriesMasterId` are load-bearing: they
// are how an occurrence is recognised and folded back into its series.
//
// `responseStatus` is here because `calendarMeta` READS IT and the projection
// did not request it — so on the list path it was always undefined and the RSVP
// silently degraded to a free/busy value, while the delta and get paths (which
// sent no $select at all) saw the real thing. The same event acquired two
// different vocabularies depending on which path fetched it. It now travels in
// its own metadata key (`my_response`), never sharing a column with `show_as`.
//
// `iCalUId` is the only key that identifies the same meeting across two mounts
// (organizer's copy vs attendee's copy) and across a calendar move.
// `originalStart` is what makes an exception addressable: it names the
// occurrence slot the exception replaces.
//
// Widening the projection does NOT reach a mount that already holds a stored
// delta link — Graph freezes $select inside delta/next links (see the comment
// on `initialDeltaUrl`). Recovering these fields on an existing calendar mount
// requires discarding its cursor so the full walk re-materializes.
export var EVENT_SELECT =
  "id,iCalUId,subject,start,end,isAllDay,location,attendees,organizer," +
  "recurrence,showAs,isCancelled,responseStatus,webLink,type,seriesMasterId," +
  "originalStart,onlineMeeting,onlineMeetingUrl,createdDateTime," +
  "lastModifiedDateTime";

// $select for one calendar request, widened when the mount opted into bodies.
// Same shape and the same reason as `mailSelect`: read the flag at every call
// site so the list, get and delta paths cannot disagree about the projection.
export function eventSelect(mount) {
  return includeBody(mount) ? EVENT_SELECT + ",body" : EVENT_SELECT;
}

// ---- mount helpers --------------------------------------------------------

// One config read, used by every helper below.
//
// Prefers the engine's pre-merged `mount.config`
// (api_config < connector < connection < sync_config), so a value may be set
// once on the CONNECTION ("this connection is the Sales shared mailbox") and
// overridden per mount. Falls back to the raw `sync_config` for an older engine
// that does not send `config`.
//
// Deliberately ONE function rather than the same two-line lookup repeated per
// key: every setting must resolve through the same precedence, or a mount and
// its push subscription can end up disagreeing about which mailbox they mean.
export function configValue(mount, key) {
  var merged = (mount && mount.config) || {};
  var sc = (mount && mount.sync_config) || {};
  return merged[key] !== undefined ? merged[key] : sc[key];
}

// Trim a config string, returning null for absent/blank. The admin console
// writes "" for a cleared text field, and "" must mean "unset" (i.e. /me), not
// "/users/".
export function configStr(mount, key) {
  var v = configValue(mount, key);
  if (typeof v !== "string") return null;
  v = v.trim();
  return v.length ? v : null;
}

// Which Graph surface this mount targets.
export function resourceOf(mount) {
  var resource = configValue(mount, "resource");
  if (resource === "calendar") return "calendar";
  if (resource === "files") return "files";
  return "mail";
}

// The mailbox/calendar OWNER segment: "/me", or "/users/{upn}" when the mount or
// connection names one.
//
// This is what makes a SHARED MAILBOX work: with `Mail.Read.Shared` granted and
// the signed-in account holding Full Access in Exchange, the identical requests
// against /users/{upn} read the shared mailbox. `principal` accepts a UPN, a
// mailbox address or a directory object id — Graph takes any of them.
//
// EVERY Graph URL in this adapter goes through here. If you add a request, use
// it; a request left on a literal /me syncs the wrong mailbox silently, with no
// error anywhere, and that is doubly true of `subscriptionResource` — a push
// subscription on the wrong mailbox looks healthy and delivers nothing.
export function principal(mount) {
  var who = configStr(mount, "principal");
  return who ? "/users/" + enc(who) : "/me";
}

// Where a `files` mount's drive lives. Three scopes, one code path, because all
// three return driveItems and therefore share `filesMeta()` and the
// `/mappers/ms-graph-files` mapper unchanged:
//
//   me    -> /me/drive                            (the signed-in user's OneDrive)
//   user  -> /users/{principal}/drive             (someone else's OneDrive)
//   site  -> /sites/{site_id}/drives/{drive_id}   (a SharePoint document library)
//
// `site` without a `drive_id` falls back to /sites/{id}/drive, the site's
// DEFAULT library — the common case, and the one an operator can configure
// without discovering a drive id first.
//
// The scope is inferred when unset so existing mounts and half-filled configs
// still resolve: a site_id implies `site`, a principal implies `user`.
export function driveBase(mount) {
  var scope = configStr(mount, "drive_scope");
  var site = configStr(mount, "site_id");
  var drive = configStr(mount, "drive_id");
  if (!scope) scope = site ? "site" : configStr(mount, "principal") ? "user" : "me";

  if (scope === "site") {
    if (!site) {
      // A config error, not a transient one: retrying sends the same
      // incomplete request forever. Match `raiseForStatus`'s 400/404 handling
      // so the engine marks the mount misconfigured and stops hammering.
      throw coded(
        "drive_scope 'site' requires a site_id (the SharePoint site to read)",
        "config_error"
      );
    }
    return drive
      ? "/sites/" + encSiteId(site) + "/drives/" + enc(drive)
      : "/sites/" + encSiteId(site) + "/drive";
  }
  if (scope === "user") {
    var who = configStr(mount, "principal");
    if (!who) {
      throw coded(
        "drive_scope 'user' requires a principal (whose OneDrive to read)",
        "config_error"
      );
    }
    return "/users/" + enc(who) + "/drive";
  }
  return "/me/drive";
}

// OneDrive/SharePoint drive-item container id: mount.remote_root or the
// well-known "root".
export function driveRoot(mount) {
  return (mount && mount.remote_root) || null;
}

// The drive-item container a `files` mount is rooted at, as a full path segment.
// Shared by list and delta so the two can never disagree about the root.
export function driveContainer(mount) {
  var base = driveBase(mount);
  var root = driveRoot(mount);
  return root ? base + "/items/" + enc(root) : base + "/root";
}

// Mail folder id: mount.remote_root or the well-known "inbox".
export function mailFolderId(mount) {
  return (mount && mount.remote_root) || "inbox";
}

// Calendar id: mount.remote_root or the well-known "calendar". Operators should
// set remote_root to a real calendar id for non-default calendars.
export function calendarId(mount) {
  return (mount && mount.remote_root) || "calendar";
}

// How many instances of one series are inspected for exceptions per full walk.
// Bounded on purpose: a daily standup over a 30-day window is ~30 instances, and
// an unbounded expansion of a years-long series is not something a sync run
// should ever attempt.
export var INSTANCE_PAGE = 200;

export function pageSize(params) {
  return params && params.limit && params.limit > 0
    ? Math.min(params.limit, 999)
    : 100;
}

// The CONFIGURED calendar window, in days. Separated from `windowBounds` because
// the two are used for opposite purposes: the bounds are relative to `now` and
// therefore change on every call, while the config is stable and is what a
// cursor's identity is compared against. Hashing the bounds instead would
// invalidate the cursor on every single run.
export function windowConfig(mount) {
  var sc = (mount && mount.sync_config) || {};
  var win = sc.window || {};
  return {
    daysAhead: win.days_ahead != null ? win.days_ahead : 30,
    daysBack: win.days_back != null ? win.days_back : 7,
  };
}

// calendarView/delta bounds. days_back/days_ahead default to a 7d/30d window.
export function windowBounds(mount) {
  var win = windowConfig(mount);
  var now = Date.now();
  return {
    start: new Date(now - win.daysBack * 86400000).toISOString(),
    end: new Date(now + win.daysAhead * 86400000).toISOString(),
  };
}

// Whether this mount's calendar can be delta-synced at all.
//
// v1.0 documents `calendarView/delta` ONLY at the mailbox level — `/me/...` and
// `/users/{id}/calendarView/delta` — and both are the PRIMARY calendar. There is
// no `/calendars/{id}/calendarView/delta`. A mount pointed at a secondary or
// shared calendar therefore has no incremental feed, and claiming one made the
// engine store a cursor for a route Graph does not serve.
//
// Saying `supports_changes: false` is not a degradation, it is the truth: the
// engine already handles it by running a full reconcile every time, which is
// what a non-primary calendar has to do.
export function calendarSupportsDelta(mount) {
  return calendarId(mount) === "calendar";
}
