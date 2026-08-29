import { coded } from "./common.js";
import { connConfig } from "./mount.js";

/**
 * Gmail push (Pub/Sub watch): the `subscribe` / `renew` / `unsubscribe`
 * lifecycle, over the Gmail REST API.
 *
 * Its own module because it is HTTP against googleapis.com inside an adapter
 * whose entire design claim is "generic RFC 3501". Keeping it here is what makes
 * that claim checkable: a plain IMAP mount configures no `pubsub_topic`,
 * `opCapabilities` reports supports_push:false, and nothing in this file is ever
 * reached.
 */

// ---- Gmail push (Pub/Sub watch) -------------------------------------------
//
// Gmail exposes NO direct webhook. Push is a three-hop chain the OPERATOR sets
// up once: `users.watch` arms the mailbox against an operator-owned Pub/Sub
// TOPIC; that topic's push SUBSCRIPTION POSTs to this mount's notifications
// endpoint on every mailbox change. The adapter owns only the `users.watch` /
// `users.stop` hop — it can never create the topic or the Pub/Sub subscription
// (see this package's README for the operator steps).
//
// The push is a pure INVALIDATION signal: the Pub/Sub message body (its
// historyId) is IGNORED. A ping just means "re-run this mount's normal delta",
// and the engine's next get_changes fetches new mail over IMAP (UID delta) as
// usual. So Gmail push and IMAP polling share one code path — push only removes
// the wait.
//
// Gmail's REST watch/stop ride the same OAuth token as XOAUTH2 IMAP: the
// https://mail.google.com/ scope already grants the Gmail API, so credential
// .access_token is reused as the bearer. An app-password mount has no bearer,
// so push is unavailable there (throws auth_expired).
var GMAIL_API = "https://gmail.googleapis.com/gmail/v1/users/me";

// The operator's Pub/Sub topic (projects/<p>/topics/<t>). Absent => no push.
export function pubsubTopic(mount) {
  var sc = (mount && mount.sync_config) || {};
  return sc.pubsub_topic || null;
}

// Optional shared secret echoed back as the subscription `secret`; the engine
// can compare it against a token the operator configures on the Pub/Sub push
// subscription. Empty string when unset.
function pubsubVerifyToken(mount) {
  var sc = (mount && mount.sync_config) || {};
  return sc.pubsub_verify_token || "";
}

function accountEmail(credential, mount) {
  var cred = credential || {};
  return cred.username || cred.user || connConfig(mount).username || "me";
}

// Map a Gmail REST error into the engine's dispatch codes. An auth failure must
// surface as auth_expired, never a silent success (which reads as "nothing to
// watch"). A plain Error (no code) is treated as transient and retried.
function raiseGmail(resp, context) {
  var status = resp.status;
  if (status >= 200 && status < 300) return;
  if (status === 401 || status === 403) {
    throw coded("Gmail API rejected the access token", "auth_expired");
  }
  if (status === 429) {
    throw coded("Gmail API rate limit exceeded", "rate_limited");
  }
  var body = resp.body || {};
  var msg =
    (body.error && body.error.message) || "Gmail API request failed (" + status + ")";
  throw new Error(context + ": " + msg);
}

function gmailFetch(credential, method, path, body) {
  var cred = credential || {};
  if (!cred.access_token) {
    throw coded(
      "Gmail push requires an OAuth access token (XOAUTH2 account); an app-password mount cannot use push",
      "auth_expired"
    );
  }
  var request = {
    method: method,
    headers: { Authorization: "Bearer " + cred.access_token },
  };
  if (body !== undefined) request.body = body;
  var resp = raisin.http.fetch(GMAIL_API + path, request);
  raiseGmail(resp, method + " " + path);
  return resp;
}

// Gmail's watch `expiration` is an ms-epoch string ~7 days out. -> ISO-8601.
function msToIso(ms) {
  var n = Number(ms);
  if (!n || isNaN(n)) return null;
  return new Date(n).toISOString();
}

// subscribe: arm users.watch against the operator's Pub/Sub topic. subscription_id
// is stable per account. If pubsub_topic is missing this THROWS (never a silent
// no-op) so a mis-set mount fails loudly instead of pretending push is live.
export function opSubscribe(credential, mount) {
  var topic = pubsubTopic(mount);
  if (!topic) {
    throw coded(
      "Gmail push not configured: set sync_config.pubsub_topic to your Pub/Sub topic (projects/<p>/topics/<t>)",
      "conflict"
    );
  }
  var resp = gmailFetch(credential, "POST", "/watch", {
    topicName: topic,
    labelIds: ["INBOX"],
  });
  var out = resp.body || {};
  return {
    subscription_id: "gmail-watch:" + accountEmail(credential, mount),
    secret: pubsubVerifyToken(mount),
    expires_at: msToIso(out.expiration),
    resource: topic,
  };
}

// renew: Gmail watch lapses in ~7d and Google recommends re-calling daily. Just
// re-run users.watch for a fresh expiration; the engine's renewal job drives this.
export function opRenew(credential, mount, params) {
  var topic = pubsubTopic(mount);
  if (!topic) {
    throw coded(
      "Gmail push not configured: sync_config.pubsub_topic is required to renew",
      "conflict"
    );
  }
  var resp = gmailFetch(credential, "POST", "/watch", {
    topicName: topic,
    labelIds: ["INBOX"],
  });
  var out = resp.body || {};
  return {
    subscription_id:
      (params && params.subscription_id) ||
      "gmail-watch:" + accountEmail(credential, mount),
    expires_at: msToIso(out.expiration),
  };
}

// unsubscribe: stop all Gmail push for this account (users.stop, empty body).
export function opUnsubscribe(credential) {
  gmailFetch(credential, "POST", "/stop");
  return { ok: true };
}
