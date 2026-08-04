/**
 * Google Calendar virtual-node adapter (EXPERIMENTAL / PREVIEW).
 *
 * Implements the frozen adapter contract (docs/reference/virtual-node-adapters.md)
 * over the Google Calendar v3 REST API using the synchronous `raisin.http.fetch`
 * binding. The sync engine invokes this function directly, decrypts the account
 * credential just before the call, and materializes returned items into nodes.
 *
 * Entrypoint: handler(input) — exactly one argument.
 *   input = { operation, params, credential, mount }
 *
 * Read-only preview: this adapter reports can_read + supports_changes, plus push
 * (supports_push) via events.watch channels — see the "push subscription
 * lifecycle" section below; notifications are pure invalidation signals.
 * Token lifecycle is owned entirely by the engine: `credential.access_token` is
 * a current, decrypted token; there is NO refresh_token and no refresh logic
 * here. If a token is rejected, throw `auth_expired` and let the engine handle
 * the reconnect/refresh cycle.
 *
 * ── window + syncToken flow ────────────────────────────────────────────────
 *   full / list  → events.list bounded by a time window
 *                  (timeMin = now - window.days_back, timeMax = now + window.days_ahead),
 *                  singleEvents=true, orderBy=startTime. Recurring events are
 *                  expanded into individual instances.
 *   get_changes  → incremental sync via Google's opaque syncToken.
 *                  * no since_token → baseline: page a windowed list to the end
 *                    to obtain a nextSyncToken; return items:[] (the engine has
 *                    already run a full reconcile) and next_token = that token.
 *                  * with since_token → events.list?syncToken=since_token
 *                    (no timeMin/timeMax/orderBy — those invalidate a syncToken).
 *                    next_token = nextSyncToken. NEVER null — echo the prior
 *                    token when Google returns no new token.
 *                  * HTTP 410 GONE → the syncToken expired; reported as
 *                    `cursor_invalid`, so the engine drops the stored token and
 *                    full-reconciles within the same run.
 */

var CAL = "https://www.googleapis.com/calendar/v3";
var DAY_MS = 86400000;
// Google web_hook channels live at most ~7 days; request the full window and let
// the engine's renewal job rotate before expiry.
var WATCH_TTL_SECONDS = 604800;

function coded(message, code) {
  var e = new Error(message);
  e.code = code;
  return e;
}

// Throw the reserved error codes the engine dispatches on. Never swallow an
// auth failure into an empty result — that reads as "everything was deleted".
function raiseForStatus(resp, context) {
  var status = resp.status;
  if (status >= 200 && status < 300) return;

  var body = resp.body || {};
  var reason = "";
  try {
    if (body && body.error && body.error.errors && body.error.errors.length) {
      reason = body.error.errors[0].reason || "";
    }
  } catch (_) {
    reason = "";
  }

  if (status === 401) {
    throw coded("Google Calendar rejected the access token", "auth_expired");
  }
  if (status === 429) {
    throw coded("Google Calendar rate limit exceeded", "rate_limited");
  }
  if (status === 403) {
    if (
      reason === "rateLimitExceeded" ||
      reason === "userRateLimitExceeded" ||
      reason === "dailyLimitExceeded"
    ) {
      throw coded("Google Calendar usage limit exceeded", "rate_limited");
    }
    // Other 403s are authorization failures (revoked grant, insufficient scope).
    throw coded("Google Calendar denied the request", "auth_expired");
  }
  var msg =
    (body && body.error && body.error.message) ||
    "Google Calendar request failed (" + status + ")";

  // An EXPIRED syncToken. Google answers 410 GONE (reason `fullSyncRequired`)
  // and documents the recovery as "discard the token and do a full sync", which
  // is exactly what `cursor_invalid` asks the engine to do — in the same run,
  // rather than relying on some later full pass that may never be scheduled.
  //
  // This used to be a plain Error → `Transient`, so the job retried the same
  // rejected token three times per tick and the failure counter it accumulated
  // gated the mount's backfill fast path as well. Same defect the ms-graph
  // adapter had; fixed in both rather than left to drift.
  if (status === 410 || reason === "fullSyncRequired") {
    throw coded(context + ": " + msg, "cursor_invalid");
  }
  throw new Error(context + ": " + msg);
}

// Single authorized request. `raisin.http.fetch` is synchronous and returns
// { status, headers, body }.
function calFetch(credential, method, url, opts) {
  opts = opts || {};
  // The engine passes `credential: null` when no account is selected; guard so
  // that surfaces as a readable error rather than a TypeError. Plain Error on
  // purpose — a coded "auth_expired" would be rewritten by the host into
  // "credential is expired or was rejected", the wrong diagnosis here.
  if (!credential || !credential.access_token) {
    throw new Error(
      "no account credential — connect a Google account and select it for this connector or mount"
    );
  }
  var headers = { Authorization: "Bearer " + credential.access_token };
  if (opts.headers) {
    for (var k in opts.headers) headers[k] = opts.headers[k];
  }
  var request = { method: method, headers: headers };
  if (opts.body !== undefined) request.body = opts.body;
  var resp = raisin.http.fetch(url, request);
  // Callers that inspect specific statuses (404 on get, 410 on sync) opt out of
  // the automatic raise for exactly those codes.
  if (!opts.rawStatusOk) {
    raiseForStatus(resp, opts.context || method + " " + url);
  }
  return resp;
}

function enc(v) {
  return encodeURIComponent(v);
}

function calendarId(mount) {
  return (mount && mount.remote_root) || "primary";
}

function syncWindow(mount) {
  var cfg = (mount && mount.sync_config) || {};
  var w = cfg.window || {};
  var daysAhead = w.days_ahead != null ? Number(w.days_ahead) : 90;
  var daysBack = w.days_back != null ? Number(w.days_back) : 7;
  var now = Date.now();
  return {
    timeMin: new Date(now - daysBack * DAY_MS).toISOString(),
    timeMax: new Date(now + daysAhead * DAY_MS).toISOString(),
  };
}

// Normalize a Calendar {dateTime|date} to an ISO string; report all-day.
function whenOf(slot) {
  if (!slot) return { value: null, allDay: false };
  if (slot.dateTime) return { value: slot.dateTime, allDay: false };
  if (slot.date) return { value: slot.date, allDay: true };
  return { value: null, allDay: false };
}

// Build a normalized ExternalItem from a Calendar event resource. Events are
// leaves (never folders); name and relative_path are the stable event id so the
// engine's upsert key is deterministic regardless of a mutable summary.
function toExternalItem(ev, calId) {
  var start = whenOf(ev.start);
  var end = whenOf(ev.end);
  var organizer =
    (ev.organizer && (ev.organizer.email || ev.organizer.displayName)) || null;
  return {
    external_id: ev.id,
    name: ev.id,
    relative_path: ev.id,
    mime_type: null,
    size_bytes: null,
    is_folder: false,
    parent_id: null,
    created_at: ev.created || null,
    modified_at: ev.updated || null,
    // Google's per-event etag is a stable change token — lets the engine's
    // skip-write suppress needless revisions when nothing changed.
    etag: ev.etag || ev.updated || null,
    web_url: ev.htmlLink || null,
    download_url: null,
    metadata: {
      summary: ev.summary || null,
      status: ev.status || null,
      location: ev.location || null,
      htmlLink: ev.htmlLink || null,
      organizer: organizer,
      attendees: ev.attendees || null,
      recurrence: ev.recurrence || null,
      start: start.value,
      end: end.value,
      all_day: start.allDay,
      calendar_id: calId,
    },
  };
}

// ---- operations -----------------------------------------------------------

function opCapabilities() {
  return {
    can_read: true,
    can_write: false,
    can_create_folders: false,
    supports_changes: true,
    supports_webhooks: false,
    supports_search: false,
    supports_push: true,
    supports_webhooks: true,
    default_ttl: WATCH_TTL_SECONDS,
    max_file_size: null,
  };
}

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

var SUBID_SEP = "\t";

function encodeSubId(channelId, resourceId, secret) {
  return channelId + SUBID_SEP + (resourceId || "") + SUBID_SEP + (secret || "");
}

// Recover { channelId, resourceId, secret } from a stored subscription_id.
// Tolerates a bare channel id (no separators) so an id minted before this
// scheme still stops cleanly.
function decodeSubId(subId) {
  var parts = String(subId || "").split(SUBID_SEP);
  return {
    channelId: parts[0] || "",
    resourceId: parts[1] || "",
    secret: parts[2] || "",
  };
}

// Google returns channel expiration as epoch-millis (string). Map to ISO-8601;
// null when absent so the engine falls back to its default renewal window.
function msToIso(expiration) {
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
function calPostJson(credential, url, obj, context, rawStatusOk) {
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
function startWatch(credential, mount, notificationUrl, reuseSecret) {
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
function stopWatch(credential, channelId, resourceId) {
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

function opSubscribe(credential, mount, params) {
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
function opRenew(credential, mount, params) {
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

function opUnsubscribe(credential, mount, params) {
  var sub = decodeSubId(params && params.subscription_id);
  stopWatch(credential, sub.channelId, sub.resourceId);
  return { ok: true };
}

function opList(credential, mount, params) {
  var calId = calendarId(mount);
  var win = syncWindow(mount);
  var pageSize =
    params.limit && params.limit > 0 ? Math.min(params.limit, 2500) : 250;
  var url =
    CAL +
    "/calendars/" +
    enc(calId) +
    "/events?singleEvents=true&orderBy=startTime" +
    "&timeMin=" +
    enc(win.timeMin) +
    "&timeMax=" +
    enc(win.timeMax) +
    "&maxResults=" +
    pageSize;
  if (params.cursor) url += "&pageToken=" + enc(params.cursor);

  var resp = calFetch(credential, "GET", url, { context: "list" });
  var body = resp.body || {};
  var events = body.items || [];
  var items = events.map(function (ev) {
    return toExternalItem(ev, calId);
  });
  return { items: items, next_cursor: body.nextPageToken || null };
}

function opGet(credential, mount, params) {
  var calId = calendarId(mount);
  // Events are keyed by id; relative_path is that same id, so path resolves to
  // an item_id lookup either way.
  var eventId = params.item_id || params.path;
  if (!eventId) return null;
  eventId = String(eventId).replace(/^\/+/, "");
  var url =
    CAL + "/calendars/" + enc(calId) + "/events/" + enc(eventId);
  var resp = calFetch(credential, "GET", url, {
    context: "get",
    rawStatusOk: true,
  });
  if (resp.status === 404 || resp.status === 410) return null;
  raiseForStatus(resp, "get");
  var ev = resp.body;
  if (!ev || ev.status === "cancelled") return null;
  return toExternalItem(ev, calId);
}

// Events carry no binary payload; content sync returns the event resource as a
// JSON document so opt-in content mounts still receive something meaningful.
function opGetContent(credential, mount, params) {
  var calId = calendarId(mount);
  var eventId = String(params.item_id || "").replace(/^\/+/, "");
  var url = CAL + "/calendars/" + enc(calId) + "/events/" + enc(eventId);
  var resp = calFetch(credential, "GET", url, {
    context: "get_content",
    rawStatusOk: true,
  });
  if (resp.status === 404 || resp.status === 410) return null;
  raiseForStatus(resp, "get_content");
  return { content: JSON.stringify(resp.body), mime_type: "application/json" };
}

function opGetChanges(credential, mount, params) {
  var calId = calendarId(mount);
  var token = params.since_token;

  // Baseline: no prior token. Page a windowed list to the end purely to harvest
  // a nextSyncToken; the engine has already reconciled the initial state, so we
  // report zero changes and hand back the token to drive future deltas.
  if (!token) {
    var win = syncWindow(mount);
    var base =
      CAL +
      "/calendars/" +
      enc(calId) +
      "/events?singleEvents=true&maxResults=2500" +
      "&timeMin=" +
      enc(win.timeMin) +
      "&timeMax=" +
      enc(win.timeMax);
    var syncToken = null;
    var pageToken = null;
    // Walk to the last page; nextSyncToken only appears on the final page.
    for (var guard = 0; guard < 50; guard++) {
      var u = base + (pageToken ? "&pageToken=" + enc(pageToken) : "");
      var r = calFetch(credential, "GET", u, { context: "get_changes(base)" });
      var b = r.body || {};
      if (b.nextSyncToken) {
        syncToken = b.nextSyncToken;
        break;
      }
      if (b.nextPageToken) {
        pageToken = b.nextPageToken;
        continue;
      }
      break;
    }
    return { items: [], next_token: syncToken || "" };
  }

  // Incremental: syncToken sync cannot be combined with timeMin/timeMax/orderBy.
  var url =
    CAL +
    "/calendars/" +
    enc(calId) +
    "/events?singleEvents=true&showDeleted=true&maxResults=2500" +
    "&syncToken=" +
    enc(token);
  if (params.cursor) url += "&pageToken=" + enc(params.cursor);

  var resp = calFetch(credential, "GET", url, {
    context: "get_changes",
    rawStatusOk: true,
  });
  // 410 GONE → the syncToken expired. Signal transient so the engine drops the
  // token and re-runs a full reconcile. No dedicated resync code path exists.
  if (resp.status === 410) {
    throw new Error("Google Calendar syncToken expired (410 GONE); full resync required");
  }
  raiseForStatus(resp, "get_changes");

  var body = resp.body || {};
  var events = body.items || [];
  var items = events.map(function (ev) {
    if (ev.status === "cancelled") {
      return {
        type: "deleted",
        item: { external_id: ev.id },
        relative_path: ev.id,
      };
    }
    var item = toExternalItem(ev, calId);
    return { type: "updated", item: item, relative_path: item.relative_path };
  });
  // next_token is NEVER null: prefer a fresh nextSyncToken, fall back to the
  // page token while paging, else echo the caller's token so the cursor holds.
  var next = body.nextSyncToken || body.nextPageToken || token;
  return { items: items, next_token: next };
}

// ---- dispatch -------------------------------------------------------------

function handler(input) {
  var operation = input.operation;
  var params = input.params || {};
  var credential = input.credential;
  var mount = input.mount || {};

  switch (operation) {
    case "capabilities":
      return opCapabilities();
    case "list":
      return opList(credential, mount, params);
    case "get":
      return opGet(credential, mount, params);
    case "get_content":
      return opGetContent(credential, mount, params);
    case "get_changes":
      return opGetChanges(credential, mount, params);
    case "subscribe":
      return opSubscribe(credential, mount, params);
    case "renew":
      return opRenew(credential, mount, params);
    case "unsubscribe":
      return opUnsubscribe(credential, mount, params);
    default:
      throw new Error("Unsupported operation: " + operation);
  }
}
