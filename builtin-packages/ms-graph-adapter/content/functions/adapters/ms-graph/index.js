/**
 * Microsoft 365 (Microsoft Graph) virtual-node adapter.
 *
 * EXPERIMENTAL / PREVIEW. Implements the frozen adapter contract
 * (docs/reference/virtual-node-adapters.md) over the Microsoft Graph v1.0 REST
 * API using the synchronous `raisin.http.fetch` binding. The sync engine invokes
 * this function directly, decrypts the account credential just before the call,
 * and materializes returned items into nodes.
 *
 * Entrypoint: handler(input) — exactly one argument.
 *   input = { operation, params, credential, mount }
 *
 * `input.mount.sync_config.resource` selects the surface:
 *   - "mail"     (default) -> /me/mailFolders/{id}/messages
 *   - "calendar"           -> /me/calendars/{calId}/events + calendarView/delta
 *   - "files"              -> /me/drive/root/children + /me/drive/root/delta
 *
 * Push (EXPERIMENTAL): mail/calendar/files are all push-capable. The engine calls
 * `subscribe`/`renew`/`unsubscribe` to manage a Microsoft Graph subscription whose
 * notifications are a pure invalidation signal — the engine re-runs `get_changes`.
 * The Graph validationToken handshake is echoed by the RaisinDB notifications
 * endpoint, NOT here; `clientState` is the per-subscription secret the endpoint
 * verifies.
 *
 * Token lifecycle is owned entirely by the engine: `credential.access_token` is
 * a current, decrypted bearer token; there is NO refresh_token and no refresh
 * logic here. If a token is rejected, throw `auth_expired` and let the engine
 * handle the reconnect/refresh cycle. Read-only: create/update/delete are not
 * supported (capabilities report can_write = false).
 */

var GRAPH = "https://graph.microsoft.com/v1.0";

// Message fields fetched so ExternalItem/metadata can be built in one call. Graph
// preserves $select inside the delta/next links, so the delta feed keeps them too.
var MAIL_SELECT =
  "id,subject,from,toRecipients,ccRecipients,receivedDateTime,sentDateTime," +
  "bodyPreview,isRead,hasAttachments,importance,conversationId,webLink," +
  "internetMessageId,createdDateTime,lastModifiedDateTime";

function coded(message, code) {
  var e = new Error(message);
  e.code = code;
  return e;
}

// Throw the reserved error codes the engine dispatches on. Never swallow an auth
// failure into an empty result — that reads to the engine as "everything was
// deleted". A plain Error (no `code`) is treated as transient and retried.
function raiseForStatus(resp, context) {
  var status = resp.status;
  if (status >= 200 && status < 300) return;
  var body = resp.body || {};
  if (status === 401 || status === 403) {
    throw coded("Microsoft Graph rejected the access token", "auth_expired");
  }
  if (status === 429) {
    throw coded("Microsoft Graph rate limit exceeded", "rate_limited");
  }
  var msg =
    (body && body.error && body.error.message) ||
    "Microsoft Graph request failed (" + status + ")";
  throw new Error(context + ": " + msg);
}

// Single authorized request. `raisin.http.fetch` is synchronous and returns
// { status, headers, body }.
function graphFetch(credential, method, url, opts) {
  opts = opts || {};
  // The engine passes `credential: null` when no account is selected. Without
  // this guard that surfaced as an opaque
  // "cannot read property 'access_token' of null" TypeError from deep inside
  // the adapter. Plain Error on purpose: a coded "auth_expired" would be
  // rewritten by the host into "credential is expired or was rejected", which
  // is the wrong diagnosis for "no account connected".
  if (!credential || !credential.access_token) {
    throw new Error(
      "no account credential — connect a Microsoft account and select it for this connector or mount"
    );
  }
  var headers = { Authorization: "Bearer " + credential.access_token };
  if (opts.headers) {
    for (var k in opts.headers) headers[k] = opts.headers[k];
  }
  var request = { method: method, headers: headers };
  if (opts.body !== undefined) request.body = opts.body;
  var resp = raisin.http.fetch(url, request);
  if (!opts.rawStatusOk || resp.status !== 404) {
    raiseForStatus(resp, opts.context || method + " " + url);
  }
  return resp;
}

function enc(v) {
  return encodeURIComponent(v);
}

// ---- mount helpers --------------------------------------------------------

// Which Graph surface this mount targets. Reads the engine's pre-merged
// `mount.config` (api_config < connector < connection < sync_config) so a
// connection can set a default resource, with sync_config as the fallback for
// an older engine that does not send `config`.
function resourceOf(mount) {
  var merged = (mount && mount.config) || {};
  var sc = (mount && mount.sync_config) || {};
  var resource = merged.resource !== undefined ? merged.resource : sc.resource;
  if (resource === "calendar") return "calendar";
  if (resource === "files") return "files";
  return "mail";
}

// OneDrive drive-item container id: mount.remote_root or the well-known "root".
function driveRoot(mount) {
  return (mount && mount.remote_root) || null;
}

// Mail folder id: mount.remote_root or the well-known "inbox".
function mailFolderId(mount) {
  return (mount && mount.remote_root) || "inbox";
}

// Calendar id: mount.remote_root or the well-known "calendar". Operators should
// set remote_root to a real calendar id for non-default calendars.
function calendarId(mount) {
  return (mount && mount.remote_root) || "calendar";
}

function pageSize(params) {
  return params && params.limit && params.limit > 0
    ? Math.min(params.limit, 999)
    : 100;
}

// calendarView/delta bounds. days_back/days_ahead default to a 7d/30d window.
function windowBounds(mount) {
  var sc = (mount && mount.sync_config) || {};
  var win = sc.window || {};
  var daysAhead = win.days_ahead != null ? win.days_ahead : 30;
  var daysBack = win.days_back != null ? win.days_back : 7;
  var now = Date.now();
  return {
    start: new Date(now - daysBack * 86400000).toISOString(),
    end: new Date(now + daysAhead * 86400000).toISOString(),
  };
}

// ---- address formatting ---------------------------------------------------

function fmtAddr(recip) {
  if (!recip || !recip.emailAddress) return null;
  var e = recip.emailAddress;
  if (e.name && e.address) return e.name + " <" + e.address + ">";
  return e.address || e.name || null;
}

function fmtAddrList(list) {
  if (!list || !list.length) return null;
  var out = [];
  for (var i = 0; i < list.length; i++) {
    var a = fmtAddr(list[i]);
    if (a) out.push(a);
  }
  return out.length ? out.join(", ") : null;
}

// ---- ExternalItem builders ------------------------------------------------

function mailMeta(v) {
  return {
    subject: v.subject || null,
    from: fmtAddr(v.from),
    to: fmtAddrList(v.toRecipients),
    cc: fmtAddrList(v.ccRecipients),
    date: v.receivedDateTime || v.sentDateTime || null,
    snippet: v.bodyPreview || null,
    unread: v.isRead === false,
    has_attachments: v.hasAttachments === true,
    importance: v.importance || null,
    conversation_id: v.conversationId || null,
    internet_message_id: v.internetMessageId || null,
    web_url: v.webLink || null,
  };
}

function calendarMeta(v) {
  var attendees = [];
  var list = v.attendees || [];
  for (var i = 0; i < list.length; i++) {
    var a = fmtAddr(list[i]);
    if (a) attendees.push(a);
  }
  return {
    subject: v.subject || null,
    start: v.start ? v.start.dateTime : null,
    start_tz: v.start ? v.start.timeZone : null,
    end: v.end ? v.end.dateTime : null,
    end_tz: v.end ? v.end.timeZone : null,
    all_day: v.isAllDay === true,
    location: v.location ? v.location.displayName || null : null,
    attendees: attendees,
    organizer: v.organizer ? fmtAddr(v.organizer) : null,
    recurrence: v.recurrence ? JSON.stringify(v.recurrence) : null,
    status: v.isCancelled
      ? "cancelled"
      : (v.responseStatus && v.responseStatus.response) || v.showAs || "confirmed",
    webLink: v.webLink || null,
  };
}

// OneDrive driveItem: is_folder from the `folder` facet, mime_type/size from the
// `file` facet. The real filename lives in metadata.filename (name = id, below).
function filesMeta(v) {
  return {
    filename: v.name || null,
    is_folder: !!v.folder,
    mime_type: v.file && v.file.mimeType ? v.file.mimeType : null,
    size_bytes: v.size != null ? v.size : null,
    parent_id: v.parentReference ? v.parentReference.id || null : null,
    parent_path: v.parentReference ? v.parentReference.path || null : null,
    child_count: v.folder && v.folder.childCount != null ? v.folder.childCount : null,
    download_url: v["@microsoft.graph.downloadUrl"] || null,
    web_url: v.webUrl || null,
    ctag: v.cTag || null,
  };
}

// external_id / name / relative_path are ALWAYS the Graph item id — never the
// subject/title/filename — so distinct items never collide on a path. All provider
// fields live in `metadata`, which the engine carries verbatim onto node properties.
function toExternalItem(v, resource) {
  var id = v.id;
  var item = {
    external_id: id,
    name: id,
    mime_type: null,
    size_bytes: null,
    is_folder: false,
    parent_id: null,
    created_at: v.createdDateTime || null,
    modified_at: v.lastModifiedDateTime || null,
    etag: v["@odata.etag"] || v.eTag || v.lastModifiedDateTime || null,
    web_url: v.webLink || v.webUrl || null,
    download_url: null,
    metadata: null,
  };
  if (resource === "files") {
    item.is_folder = !!v.folder;
    item.mime_type = v.file && v.file.mimeType ? v.file.mimeType : null;
    item.size_bytes = v.size != null ? v.size : null;
    item.parent_id = v.parentReference ? v.parentReference.id || null : null;
    item.download_url = v["@microsoft.graph.downloadUrl"] || null;
    item.metadata = filesMeta(v);
  } else if (resource === "calendar") {
    item.metadata = calendarMeta(v);
  } else {
    item.metadata = mailMeta(v);
  }
  return item;
}

// ---- operations -----------------------------------------------------------

function opCapabilities() {
  return {
    can_read: true,
    can_write: false,
    can_create_folders: false,
    supports_changes: true,
    supports_webhooks: true,
    supports_search: false,
    supports_push: true,
    default_ttl: null,
    max_file_size: null,
  };
}

// ---- push subscriptions (mail / calendar / files) -------------------------

// Graph subscription resource path for the mount's surface. Not URL-encoded:
// Graph expects a literal resource path, not a query value.
function subscriptionResource(mount) {
  var resource = resourceOf(mount);
  if (resource === "calendar") return "/me/events";
  if (resource === "files") return "/me/drive/root";
  return "/me/mailFolders/" + mailFolderId(mount) + "/messages";
}

// ~2 days out. Graph caps mail/calendar subscriptions near 3 days and driveItem
// far higher, so 2 days is safely under every ceiling; the engine renews before
// expiry (RENEW_WINDOW_SECS).
function subscriptionExpiration() {
  return new Date(Date.now() + 2 * 86400000).toISOString();
}

// subscribe -> create a Graph subscription. `clientState` is our per-subscription
// secret, echoed back on every notification and verified by the RaisinDB endpoint.
function opSubscribe(credential, mount, params) {
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
function opRenew(credential, params) {
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
function opUnsubscribe(credential, params) {
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

function opList(credential, mount, params) {
  var resource = resourceOf(mount);
  var url;
  if (params.cursor) {
    url = params.cursor;
  } else if (resource === "calendar") {
    url =
      GRAPH + "/me/calendars/" + enc(calendarId(mount)) +
      "/events?$top=" + pageSize(params);
  } else if (resource === "files") {
    var root = driveRoot(mount);
    var container = root
      ? "/me/drive/items/" + enc(root)
      : "/me/drive/root";
    url = GRAPH + container + "/children?$top=" + pageSize(params);
  } else {
    url =
      GRAPH + "/me/mailFolders/" + enc(mailFolderId(mount)) +
      "/messages?$top=" + pageSize(params) + "&$select=" + enc(MAIL_SELECT);
  }
  var resp = graphFetch(credential, "GET", url, { context: "list" });
  var body = resp.body || {};
  var values = body.value || [];
  var items = values.map(function (v) {
    return toExternalItem(v, resource);
  });
  return { items: items, next_cursor: body["@odata.nextLink"] || null };
}

function opGet(credential, mount, params) {
  var resource = resourceOf(mount);
  if (!params.item_id) return null;
  var url;
  if (resource === "calendar") {
    url = GRAPH + "/me/events/" + enc(params.item_id);
  } else if (resource === "files") {
    url = GRAPH + "/me/drive/items/" + enc(params.item_id);
  } else {
    url = GRAPH + "/me/messages/" + enc(params.item_id) + "?$select=" + enc(MAIL_SELECT);
  }
  var resp = graphFetch(credential, "GET", url, { context: "get", rawStatusOk: true });
  if (resp.status === 404) return null;
  raiseForStatus(resp, "get");
  return toExternalItem(resp.body, resource);
}

// Message/event body (or file bytes) on demand. Not called during ordinary
// link-only sync. For files, Graph's /content 302-redirects to a per-item
// download host that the adapter network policy does NOT allow-list, so opt-in
// file content sync may be blocked — link via metadata.download_url instead.
function opGetContent(credential, mount, params) {
  var resource = resourceOf(mount);
  if (resource === "files") {
    var resp = graphFetch(
      credential,
      "GET",
      GRAPH + "/me/drive/items/" + enc(params.item_id) + "/content",
      { context: "get_content" }
    );
    var body = resp.body;
    var content = typeof body === "string" ? body : JSON.stringify(body);
    return { content: content, mime_type: "application/octet-stream" };
  }
  var base =
    resource === "calendar"
      ? GRAPH + "/me/events/" + enc(params.item_id)
      : GRAPH + "/me/messages/" + enc(params.item_id);
  var resp2 = graphFetch(credential, "GET", base + "?$select=body", {
    context: "get_content",
  });
  var b = resp2.body && resp2.body.body;
  var mime = b && b.contentType === "html" ? "text/html" : "text/plain";
  return { content: b ? b.content || "" : "", mime_type: mime };
}

// Build the FIRST delta URL (no since_token yet). Subsequent calls reuse the
// engine-persisted token verbatim — it is a full @odata.nextLink/deltaLink.
function initialDeltaUrl(mount, resource) {
  if (resource === "calendar") {
    var win = windowBounds(mount);
    return (
      GRAPH + "/me/calendars/" + enc(calendarId(mount)) +
      "/calendarView/delta?startDateTime=" + enc(win.start) +
      "&endDateTime=" + enc(win.end)
    );
  }
  if (resource === "files") {
    var root = driveRoot(mount);
    return root
      ? GRAPH + "/me/drive/items/" + enc(root) + "/delta"
      : GRAPH + "/me/drive/root/delta";
  }
  return (
    GRAPH + "/me/mailFolders/" + enc(mailFolderId(mount)) +
    "/messages/delta?$select=" + enc(MAIL_SELECT)
  );
}

function opGetChanges(credential, mount, params) {
  var resource = resourceOf(mount);
  var token = params.since_token;
  var url = token || initialDeltaUrl(mount, resource);
  var resp = graphFetch(credential, "GET", url, { context: "get_changes" });
  var body = resp.body || {};
  var values = body.value || [];
  var items = values.map(function (v) {
    if (v["@removed"]) {
      return { type: "deleted", item: { external_id: v.id }, relative_path: v.id };
    }
    var item = toExternalItem(v, resource);
    return { type: "updated", item: item, relative_path: item.external_id };
  });
  // Durable, resumable cursor. While paging Graph returns @odata.nextLink; the
  // final page returns @odata.deltaLink. NEVER null: when nothing is new the
  // deltaLink round-trips, and we defensively echo the prior token/url otherwise.
  var next = body["@odata.nextLink"] || body["@odata.deltaLink"] || token || url;
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
      return opRenew(credential, params);
    case "unsubscribe":
      return opUnsubscribe(credential, params);
    default:
      throw new Error("Unsupported operation: " + operation);
  }
}
