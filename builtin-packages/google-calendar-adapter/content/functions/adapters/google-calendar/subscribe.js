// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Push subscription lifecycle (events.watch channels).

import { CAL, WATCH_TTL_SECONDS, calFetch, enc, raiseForStatus } from "./http.js";
import { calendarId } from "./mount.js";

// ---- push subscription lifecycle (events.watch channels) ------------------
//
// A push notification is a pure INVALIDATION signal: Google pings the RaisinDB
// notifications endpoint, which verifies the channel token against the mount's
// stored secret and re-runs the mount's normal `get_changes` delta (syncToken).
// This adapter never parses the notification body — it only owns the channel
// lifecycle. Two engine-contract constraints shape the packing below:
//   1. channels.stop needs BOTH the channel id AND the opaque `resourceId`, but
//      the engine round-trips only a single `subscription_id`.
//   2. Every events.watch mints a NEW channel token, yet the engine updates
//      `push_secret` on subscribe only — NOT on renew. If renew minted a fresh
//      token the notifications endpoint would keep verifying the old secret and
//      silently reject the renewed channel's pings.
// So we pack "{channelId}\t{resourceId}\t{secret}" into `subscription_id` and,
// on renew, REUSE the same secret as the new channel's token — keeping the
// engine-stored `push_secret` valid for the channel's whole lifetime. The pack
// is stored server-side only (never sent to the notifications endpoint).

export var SUBID_SEP = "\t";

export function encodeSubId(channelId, resourceId, secret) {
  return channelId + SUBID_SEP + (resourceId || "") + SUBID_SEP + (secret || "");
}

// Recover { channelId, resourceId, secret } from a stored subscription_id.
// Tolerates a bare channel id (no separators) so an id minted before this
// scheme still stops cleanly.
export function decodeSubId(subId) {
  var parts = String(subId || "").split(SUBID_SEP);
  return {
    channelId: parts[0] || "",
    resourceId: parts[1] || "",
    secret: parts[2] || "",
  };
}

// Google returns channel expiration as epoch-millis (string). Map to ISO-8601;
// null when absent so the engine falls back to its default renewal window.
export function msToIso(expiration) {
  if (expiration === undefined || expiration === null || expiration === "") {
    return null;
  }
  var ms = Number(expiration);
  if (!isFinite(ms) || ms <= 0) return null;
  return new Date(ms).toISOString();
}

// POST a JSON body to a Calendar endpoint. Google's watch/stop want an explicit
// application/json content type; the body is serialized here so the host never
// has to guess how to encode an object body.
export function calPostJson(credential, url, obj, context, rawStatusOk) {
  return calFetch(credential, "POST", url, {
    context: context,
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(obj),
    rawStatusOk: rawStatusOk === true,
  });
}

// Open a new watch channel on this mount's calendar. Returns a fresh channel id,
// the secret Google echoes back as X-Goog-Channel-Token, and the resourceId
// needed later to stop it. `reuseSecret` (renew) keeps the token stable so the
// engine's stored `push_secret` stays valid across channel rotations.
export function startWatch(credential, mount, notificationUrl, reuseSecret) {
  var calId = calendarId(mount);
  var channelId = raisin.crypto.uuid();
  // Two uuids → a 64+ char unguessable token, well under Google's 256 limit.
  var token = reuseSecret || raisin.crypto.uuid() + raisin.crypto.uuid();
  var url = CAL + "/calendars/" + enc(calId) + "/events/watch";
  var resp = calPostJson(
    credential,
    url,
    {
      id: channelId,
      type: "web_hook",
      address: notificationUrl,
      token: token,
      params: { ttl: String(WATCH_TTL_SECONDS) },
    },
    "subscribe"
  );
  var b = resp.body || {};
  return {
    channelId: channelId,
    token: token,
    resourceId: b.resourceId || "",
    expiration: b.expiration,
  };
}

// Best-effort channels.stop. A stale/expired channel returns 404/410/403 — those
// are already gone, so swallow them; the engine treats teardown as best-effort.
export function stopWatch(credential, channelId, resourceId) {
  if (!channelId || !resourceId) return;
  var url = CAL + "/channels/stop";
  var resp = calPostJson(
    credential,
    url,
    { id: channelId, resourceId: resourceId },
    "unsubscribe",
    true
  );
  var s = resp.status;
  if (s >= 200 && s < 300) return;
  if (s === 404 || s === 410 || s === 403) return;
  raiseForStatus(resp, "channels.stop");
}

export function opSubscribe(credential, mount, params) {
  var notificationUrl = params && params.notification_url;
  if (!notificationUrl) {
    throw new Error("subscribe requires params.notification_url");
  }
  var w = startWatch(credential, mount, notificationUrl);
  return {
    subscription_id: encodeSubId(w.channelId, w.resourceId, w.token),
    secret: w.token,
    expires_at: msToIso(w.expiration),
    resource: w.resourceId,
  };
}

// Google channels cannot be renewed in place, so open a new channel first, then
// stop the old one (id + resourceId recovered from the packed subscription_id).
// The new channel REUSES the old secret as its token, so the engine's stored
// `push_secret` (which renew does not update) keeps verifying the fresh channel.
// Ordering matters: the new channel is live before we tear down the old, so no
// invalidation window is dropped.
export function opRenew(credential, mount, params) {
  var notificationUrl = params && params.notification_url;
  if (!notificationUrl) {
    throw new Error("renew requires params.notification_url");
  }
  var old = decodeSubId(params.subscription_id);
  var w = startWatch(credential, mount, notificationUrl, old.secret || null);
  try {
    stopWatch(credential, old.channelId, old.resourceId);
  } catch (_) {
    // Old channel will expire on its own; a failed stop must not fail renewal.
  }
  return {
    subscription_id: encodeSubId(w.channelId, w.resourceId, w.token),
    expires_at: msToIso(w.expiration),
  };
}

export function opUnsubscribe(credential, mount, params) {
  var sub = decodeSubId(params && params.subscription_id);
  stopWatch(credential, sub.channelId, sub.resourceId);
  return { ok: true };
}
