// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Graph push subscriptions (mail / calendar / files).

import { enc } from "./common.js";
import { GRAPH, graphFetch, raiseForStatus } from "./http.js";
import { calendarId, driveBase, isMailTree, mailFolderId, principal, resourceOf } from "./mount.js";

// ---- push subscriptions (mail / calendar / files) -------------------------

// Graph subscription resource path for the mount's surface.
//
// MUST resolve the same principal/drive as the polling paths. A subscription
// created against /me while get_changes reads /users/{upn} reports healthy and
// delivers notifications for the wrong mailbox forever — there is no error to
// observe, so it is only caught by reading this function.
export function subscriptionResource(mount) {
  var resource = resourceOf(mount);
  if (resource === "calendar") {
    // `/events` is the DEFAULT calendar's collection. A mount whose remote_root
    // names another calendar polled `/calendars/{id}` while subscribing here,
    // so it could never receive a notification for the calendar it syncs —
    // reported as a healthy subscription with "no delivery yet" forever. This
    // is precisely the mismatch the comment above warns about.
    var cal = calendarId(mount);
    return cal === "calendar"
      ? principal(mount) + "/events"
      : principal(mount) + "/calendars/" + cal + "/events";
  }
  if (resource === "files") return driveBase(mount) + "/root";
  // A TREE mount goes MAILBOX-WIDE. The per-folder collection covers exactly
  // one folder out of N and would report a perfectly healthy subscription while
  // never notifying about anything in the other N-1 — the same silent mismatch
  // the comment above describes, only harder to see because one folder DOES
  // deliver. Microsoft documents `/users/{id}/messages` and `/me/messages` as
  // supported change-notification resources, with a ceiling of 1,000 active
  // Outlook subscriptions per mailbox across all applications, so one
  // subscription and one renewal is well inside it.
  if (isMailTree(mount)) return principal(mount) + "/messages";
  return principal(mount) + "/mailFolders/" + mailFolderId(mount) + "/messages";
}

// ~2 days out. Graph caps mail/calendar subscriptions near 3 days and driveItem
// far higher, so 2 days is safely under every ceiling; the engine renews before
// expiry (RENEW_WINDOW_SECS).
export function subscriptionExpiration() {
  return new Date(Date.now() + 2 * 86400000).toISOString();
}

// Which change types Graph will ACCEPT for this mount's resource.
//
// This is per-resource and Graph rejects the request outright when it is wrong —
// `Invalid 'changeType' attribute: 'created'` — so a single shared value cannot
// serve all three surfaces. `driveItem` (a files mount subscribes to
// `{drive}/root`, see `subscriptionResource`) supports **`updated` only**:
// Microsoft models a new file as an update of its parent, so there is no
// `created` to ask for. Messages and events accept the full set.
//
// The consequence of getting it wrong is not subtle and was live: every
// OneDrive/SharePoint mount on a `webhook` or `hybrid` sync mode failed to
// subscribe and fell back to polling, reporting a config_error each time.
function subscriptionChangeType(mount) {
  return resourceOf(mount) === "files" ? "updated" : "created,updated";
}

// subscribe -> create a Graph subscription. `clientState` is our per-subscription
// secret, echoed back on every notification and verified by the RaisinDB endpoint.
export function opSubscribe(credential, mount, params) {
  var secret = raisin.crypto.uuid() + raisin.crypto.uuid();
  var expires = subscriptionExpiration();
  var payload = {
    changeType: subscriptionChangeType(mount),
    notificationUrl: params.notification_url,
    resource: subscriptionResource(mount),
    expirationDateTime: expires,
    clientState: secret,
  };
  var resp = graphFetch(credential, "POST", GRAPH + "/subscriptions", {
    headers: { "Content-Type": "application/json" },
    body: payload,
    context: "subscribe",
  });
  var b = resp.body || {};
  return {
    subscription_id: b.id,
    secret: secret,
    expires_at: b.expirationDateTime || expires,
    resource: payload.resource,
  };
}

// renew -> PATCH a new expirationDateTime. Same clientState/notificationUrl.
export function opRenew(credential, params) {
  var expires = subscriptionExpiration();
  var resp = graphFetch(
    credential,
    "PATCH",
    GRAPH + "/subscriptions/" + enc(params.subscription_id),
    {
      headers: { "Content-Type": "application/json" },
      body: { expirationDateTime: expires },
      context: "renew",
    }
  );
  var b = resp.body || {};
  return {
    subscription_id: b.id || params.subscription_id,
    expires_at: b.expirationDateTime || expires,
  };
}

// unsubscribe -> DELETE. An already-absent subscription unsubscribes idempotently.
export function opUnsubscribe(credential, params) {
  var resp = graphFetch(
    credential,
    "DELETE",
    GRAPH + "/subscriptions/" + enc(params.subscription_id),
    { context: "unsubscribe", rawStatusOk: true }
  );
  if (resp.status === 404) return { ok: true };
  raiseForStatus(resp, "unsubscribe");
  return { ok: true };
}
