// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Graph push subscriptions (mail / calendar / files).

import { enc } from "./common.js";
import { GRAPH, graphFetch, raiseForStatus } from "./http.js";
import { calendarId, driveBase, mailFolderId, principal, resourceOf } from "./mount.js";

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
  return principal(mount) + "/mailFolders/" + mailFolderId(mount) + "/messages";
}

// ~2 days out. Graph caps mail/calendar subscriptions near 3 days and driveItem
// far higher, so 2 days is safely under every ceiling; the engine renews before
// expiry (RENEW_WINDOW_SECS).
export function subscriptionExpiration() {
  return new Date(Date.now() + 2 * 86400000).toISOString();
}

// subscribe -> create a Graph subscription. `clientState` is our per-subscription
// secret, echoed back on every notification and verified by the RaisinDB endpoint.
export function opSubscribe(credential, mount, params) {
  var secret = raisin.crypto.uuid() + raisin.crypto.uuid();
  var expires = subscriptionExpiration();
  var payload = {
    changeType: "created,updated",
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
