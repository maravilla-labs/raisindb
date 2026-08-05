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
 *   - "mail"     (default) -> {principal}/mailFolders/{id}/messages
 *   - "calendar"           -> {principal}/calendars/{calId}/events, and
 *                             {principal}/calendarView/delta for the PRIMARY
 *                             calendar only (v1.0 has no per-calendar delta, so
 *                             a secondary/shared calendar reports
 *                             supports_changes:false and full-reconciles)
 *   - "files"              -> {drive}/root/children + {drive}/root/delta
 *
 * `{principal}` is `/me` by default and `/users/{upn}` when the mount or
 * connection names a `principal` — that is what makes SHARED MAILBOXES work.
 * `{drive}` additionally honours `drive_scope` (me | user | site), which is what
 * makes a SharePoint document library mountable through the same `files`
 * resource and the same mapper. See `principal()` / `driveBase()` below.
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

// Whether this mount wants the FULL message body inline (sync_config.include_body).
//
// Off by default and deliberately so: mail syncs link-only, and adding `body`
// to $select multiplies the delta payload by whole HTML documents on every page
// of every run — on a mailbox-sized mount that is the difference between a few
// hundred KB and tens of MB. Turn it on when you want fulltext over the body.
function includeBody(mount) {
  var v = configValue(mount, "include_body");
  return v === true || v === "true";
}

// $select for one mail request, widened when the mount opted into bodies.
// Kept as a function rather than a second constant so the flag is read at every
// call site and the delta and list paths cannot disagree about it.
function mailSelect(mount) {
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
var EVENT_SELECT =
  "id,subject,start,end,isAllDay,location,attendees,organizer,recurrence," +
  "showAs,isCancelled,webLink,type,seriesMasterId,createdDateTime," +
  "lastModifiedDateTime";

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

  // 400 and 404 mean Graph rejected the REQUEST, not the moment: a malformed or
  // unknown folder/calendar/drive id, or a resource this mailbox does not have.
  // Retrying sends the identical request and gets the identical rejection, so
  // these are reported as config errors — the engine records the diagnosis,
  // stops retrying and marks the mount misconfigured, instead of hammering
  // Graph on every scheduler tick.
  //
  // An EXPIRED DELTA CURSOR. Graph answers 410 Gone, and also reports it as
  // `syncStateNotFound` / `resyncRequired` — sometimes with a 400, which is why
  // this is checked BEFORE the 400/404 branch below. The documented recovery is
  // "start over with a full sync", so it is reported as `cursor_invalid`: the
  // engine drops the stored cursor and full-reconciles in the same run.
  //
  // This is normal operation, not a fault. Until it had its own code it fell
  // through to a plain Error → `Transient`, so the job retried the same rejected
  // cursor three times per tick forever; the failure counter that accumulated
  // then gated the backfill fast path too, so a production mailbox could neither
  // delta-sync nor finish its import. Graph had moved to sync generation 51
  // while our stored token still said generation 1.
  var graphCode = (body && body.error && body.error.code) || "";
  var isStaleCursor =
    status === 410 ||
    graphCode === "syncStateNotFound" ||
    graphCode === "resyncRequired" ||
    /sync state.*not found|resync required/i.test(msg);
  if (isStaleCursor) {
    throw coded(context + ": " + msg, "cursor_invalid");
  }

  // Deliberately NARROW. Other 4xx codes must stay retryable:
  //   408 request timeout      — a blip
  //   409 conflict             — resolved by the next sync's fresh read
  // (401/403 and 429 are already handled above as auth_expired / rate_limited;
  //  410/resync is handled just above as cursor_invalid.)
  if (status === 400 || status === 404) {
    throw coded(context + ": " + msg, "config_error");
  }
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

// A SharePoint site id is the COMPOSITE form `hostname,siteGuid,webGuid`, and
// Microsoft documents it with literal commas. A comma is a sub-delim that RFC
// 3986 permits unescaped in a path segment, and Graph's routing is happier with
// the documented spelling than with %2C — so encode everything (a site id is
// still operator-supplied input that must not smuggle a `/` or `?` into the
// path) and then put the commas back.
function encSiteId(v) {
  return encodeURIComponent(v).split("%2C").join(",");
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
function configValue(mount, key) {
  var merged = (mount && mount.config) || {};
  var sc = (mount && mount.sync_config) || {};
  return merged[key] !== undefined ? merged[key] : sc[key];
}

// Trim a config string, returning null for absent/blank. The admin console
// writes "" for a cleared text field, and "" must mean "unset" (i.e. /me), not
// "/users/".
function configStr(mount, key) {
  var v = configValue(mount, key);
  if (typeof v !== "string") return null;
  v = v.trim();
  return v.length ? v : null;
}

// Which Graph surface this mount targets.
function resourceOf(mount) {
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
function principal(mount) {
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
function driveBase(mount) {
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
function driveRoot(mount) {
  return (mount && mount.remote_root) || null;
}

// The drive-item container a `files` mount is rooted at, as a full path segment.
// Shared by list and delta so the two can never disagree about the root.
function driveContainer(mount) {
  var base = driveBase(mount);
  var root = driveRoot(mount);
  return root ? base + "/items/" + enc(root) : base + "/root";
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

// Bare address with the display name stripped: `from` carries whatever name the
// sender currently uses, which changes over time and makes it a poor grouping
// key. The mapper indexes this one separately for GROUP BY / equality.
function bareAddr(recip) {
  return recip && recip.emailAddress ? recip.emailAddress.address || null : null;
}

function mailMeta(v) {
  var meta = {
    subject: v.subject || null,
    from: fmtAddr(v.from),
    from_address: bareAddr(v.from),
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
  // Present only when the mount opted in; `body` is absent from $select
  // otherwise, and an absent key is what tells the mapper to leave the property
  // unset rather than write an empty string over a previously synced body.
  if (v.body && typeof v.body.content === "string") {
    meta.body = v.body.content;
    meta.body_type = v.body.contentType === "html" ? "html" : "text";
  }
  return meta;
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
function calendarSupportsDelta(mount) {
  return calendarId(mount) === "calendar";
}

function opCapabilities(mount) {
  var deltaOk = resourceOf(mount) !== "calendar" || calendarSupportsDelta(mount);
  return {
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
}

// ---- browse (discovery for the mount editor) ------------------------------

// Never called during sync. See §2.10 of the adapter contract: this exists so an
// operator picks a folder/calendar/site/library instead of pasting a Graph id.

function browseItem(id, name, kind, hasChildren, hint) {
  return {
    id: id,
    name: name || id,
    kind: kind,
    has_children: hasChildren === true,
    hint: hint || null,
  };
}

// One Graph page -> BrowseItem[]. `map` turns a Graph entity into a BrowseItem.
function browsePage(credential, url, map) {
  var resp = graphFetch(credential, "GET", url, { context: "browse" });
  var body = resp.body || {};
  var values = body.value || [];
  var items = [];
  for (var i = 0; i < values.length; i++) {
    var item = map(values[i]);
    if (item) items.push(item);
  }
  return { items: items, next_cursor: body["@odata.nextLink"] || null };
}

function browseLimit(params) {
  return params && params.limit && params.limit > 0
    ? Math.min(params.limit, 200)
    : 50;
}

// `startswith` search over the directory, shared by the `mailbox` and `user`
// kinds. Single quotes are doubled per OData escaping.
function directoryFilter(params) {
  if (!params || !params.query || !params.query.trim()) return "";
  var term = params.query.trim().split("'").join("''");
  return "&$filter=" + enc(
    "startswith(displayName,'" + term + "') or startswith(mail,'" + term + "')"
  );
}

// One mapper per browse kind, so the first page and every later page agree.
function browseMapper(kind) {
  if (kind === "folder") {
    return function (v) {
      return browseItem(
        v.id, v.displayName, "folder",
        (v.childFolderCount || 0) > 0,
        v.totalItemCount != null ? v.totalItemCount + " items" : null
      );
    };
  }
  if (kind === "calendar") {
    return function (v) {
      var owner = v.owner ? v.owner.address : null;
      // `canShare: false` on a calendar you did not create is Graph's marker for
      // "shared with me" — there is no `isShared` property in v1.0.
      var hint = v.isDefaultCalendar ? "primary" : owner;
      if (v.canShare === false && !v.isDefaultCalendar) hint = "shared · " + (owner || "");
      return browseItem(v.id, v.name, "calendar", false, hint);
    };
  }
  if (kind === "mailbox" || kind === "user") {
    return function (v) {
      var addr = v.mail || v.userPrincipalName;
      if (!addr) return null;
      // A shared mailbox is an unlicensed, sign-in-blocked directory object, so
      // `accountEnabled: false` is the only cheap signal v1.0 offers. It is a
      // HEURISTIC, not proof — `mailboxSettings/userPurpose` is authoritative but
      // needs APPLICATION permission to read for anyone but the signed-in user,
      // which delegated OAuth cannot do. Labelled as "likely" for that reason.
      var hint = v.accountEnabled === false ? "likely shared mailbox" : addr;
      return browseItem(addr, v.displayName || addr, kind, kind === "user", hint);
    };
  }
  if (kind === "room") {
    return function (v) {
      var addr = v.emailAddress;
      if (!addr) return null;
      // Key on the SMTP address, never `place.id`, which Microsoft documents as
      // NOT immutable. The address also drops straight into the `/users/{addr}`
      // principal a room mailbox is addressed by.
      return browseItem(addr, v.displayName || addr, "room", false, v.building || addr);
    };
  }
  return function (v) {
    return browseItem(v.id, v.displayName || v.name, kind, false, null);
  };
}

function opBrowse(credential, mount, params) {
  params = params || {};
  // A cursor is a full Graph nextLink; the kind is already baked into it.
  if (params.cursor) {
    // Map with the SAME function the first page used. The generic mapper here
    // fell back to `v.id`, so page two of a `mailbox` listing returned directory
    // object GUIDs where page one returned SMTP addresses — and the address is
    // what gets written into `principal`, so a mailbox picked from page two
    // produced a mount that could never resolve.
    return browsePage(credential, params.cursor, browseMapper(params.kind || "item"));
  }

  var top = browseLimit(params);
  var parent = params.parent_id || null;
  var kind = params.kind || (resourceOf(mount) === "calendar" ? "calendar" : "folder");

  // Mail folders. Hierarchical: childFolderCount drives the expander, and
  // `parent_id` walks into a subfolder.
  if (kind === "folder") {
    var base = GRAPH + principal(mount) + "/mailFolders";
    var url = parent
      ? base + "/" + enc(parent) + "/childFolders?$top=" + top
      : base + "?$top=" + top;
    return browsePage(credential, url, function (v) {
      return browseItem(
        v.id,
        v.displayName,
        "folder",
        (v.childFolderCount || 0) > 0,
        v.totalItemCount != null ? v.totalItemCount + " items" : null
      );
    });
  }

  // Calendars, either the mount principal's or — with `parent_id` set to an SMTP
  // address — another mailbox's.
  //
  // The second form matters because a shared PRIMARY calendar never appears in
  // `/me/calendars`. Only accepted shares of SECONDARY calendars are copied into
  // the recipient's mailbox; "Alex shared his main calendar with me", which is
  // the common case, is reachable only as `/users/{alex}/calendars`. Browsing
  // `/me` alone therefore hid exactly the calendars an admin most wants to mount.
  //
  // The emitted id is the calendar id, and the caller must pair it with
  // `principal = parent_id`: calendar ids are MAILBOX-SCOPED, so an id from one
  // mailbox addressed under another simply errors.
  if (kind === "calendar") {
    var calBase = parent ? "/users/" + enc(parent) : principal(mount);
    return browsePage(
      credential,
      GRAPH + calBase + "/calendars?$top=" + top,
      browseMapper("calendar")
    );
  }

  // People, as step one of picking someone else's calendar. `has_children` is
  // true so the picker drills into `kind: "calendar"` with `parent_id` = address.
  //
  // Same caveat as `mailbox` below: this is the DIRECTORY, not an access list.
  if (kind === "user") {
    return browsePage(
      credential,
      GRAPH + "/users?$select=id,displayName,mail,userPrincipalName,accountEnabled&$top=" +
        top + directoryFilter(params),
      browseMapper("user")
    );
  }

  // Room and equipment mailboxes. `/places` is v1.0 and needs Place.Read.All.
  //
  // Deliberately NOT `findRooms`, which is beta-only and caps at 100.
  if (kind === "room") {
    return browsePage(
      credential,
      GRAPH + "/places/microsoft.graph.room?$top=" + top,
      browseMapper("room")
    );
  }

  // SharePoint sites. Graph requires a search term to enumerate; `*` is the
  // documented "all sites" wildcard.
  if (kind === "site") {
    var q = params.query && params.query.trim() ? params.query.trim() : "*";
    return browsePage(
      credential,
      GRAPH + "/sites?search=" + enc(q) + "&$top=" + top,
      function (v) {
        // The COMPOSITE id (hostname,siteGuid,webGuid) is what a mount needs —
        // the short guid alone will not resolve /sites/{id}/drives.
        return browseItem(v.id, v.displayName || v.name, "site", true, v.webUrl || null);
      }
    );
  }

  // Document libraries of a site (parent_id) or a user's drives.
  if (kind === "drive") {
    var owner = parent
      ? "/sites/" + encSiteId(parent)
      : principal(mount) === "/me"
        ? "/me"
        : principal(mount);
    return browsePage(credential, GRAPH + owner + "/drives?$top=" + top, function (v) {
      return browseItem(v.id, v.name, "drive", true, v.driveType || null);
    });
  }

  // Folders inside a drive. Files are omitted deliberately: this picker chooses
  // a mount ROOT, and a file is never one.
  if (kind === "driveItem") {
    var container = parent
      ? driveBase(mount) + "/items/" + enc(parent)
      : driveBase(mount) + "/root";
    return browsePage(
      credential,
      GRAPH + container + "/children?$top=" + top,
      function (v) {
        if (!v.folder) return null;
        return browseItem(
          v.id,
          v.name,
          "driveItem",
          (v.folder.childCount || 0) > 0,
          v.folder.childCount != null ? v.folder.childCount + " items" : null
        );
      }
    );
  }

  // Directory users, for picking a shared mailbox or another user's OneDrive.
  //
  // NOT a permission-accurate list: Graph has no delegated API that returns
  // "the mailboxes this account may open", so this enumerates the DIRECTORY
  // (requires User.ReadBasic.All) and a listed mailbox can still fail at sync
  // time. The console therefore keeps manual entry, and Test connection is what
  // actually proves access. Do not "improve" this into an access list.
  if (kind === "mailbox") {
    return browsePage(
      credential,
      GRAPH + "/users?$select=id,displayName,mail,userPrincipalName,accountEnabled&$top=" +
        top + directoryFilter(params),
      browseMapper("mailbox")
    );
  }


  throw coded("browse: unsupported kind '" + kind + "'", "config_error");
}

// ---- push subscriptions (mail / calendar / files) -------------------------

// Graph subscription resource path for the mount's surface.
//
// MUST resolve the same principal/drive as the polling paths. A subscription
// created against /me while get_changes reads /users/{upn} reports healthy and
// delivers notifications for the wrong mailbox forever — there is no error to
// observe, so it is only caught by reading this function.
function subscriptionResource(mount) {
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
      GRAPH + principal(mount) + "/calendars/" + enc(calendarId(mount)) +
      "/events?$top=" + pageSize(params) + "&$select=" + enc(EVENT_SELECT);
  } else if (resource === "files") {
    url = GRAPH + driveContainer(mount) + "/children?$top=" + pageSize(params);
  } else {
    url =
      GRAPH + principal(mount) + "/mailFolders/" + enc(mailFolderId(mount)) +
      "/messages?$top=" + pageSize(params) + "&$select=" + enc(mailSelect(mount));
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
    url = GRAPH + principal(mount) + "/events/" + enc(params.item_id);
  } else if (resource === "files") {
    url = GRAPH + driveBase(mount) + "/items/" + enc(params.item_id);
  } else {
    url =
      GRAPH + principal(mount) + "/messages/" + enc(params.item_id) +
      "?$select=" + enc(mailSelect(mount));
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
      GRAPH + driveBase(mount) + "/items/" + enc(params.item_id) + "/content",
      { context: "get_content" }
    );
    var body = resp.body;
    var content = typeof body === "string" ? body : JSON.stringify(body);
    return { content: content, mime_type: "application/octet-stream" };
  }
  var base =
    resource === "calendar"
      ? GRAPH + principal(mount) + "/events/" + enc(params.item_id)
      : GRAPH + principal(mount) + "/messages/" + enc(params.item_id);
  var resp2 = graphFetch(credential, "GET", base + "?$select=body", {
    context: "get_content",
  });
  var b = resp2.body && resp2.body.body;
  var mime = b && b.contentType === "html" ? "text/html" : "text/plain";
  return { content: b ? b.content || "" : "", mime_type: mime };
}

// Build the FIRST delta URL (no since_token yet). Subsequent calls reuse the
// engine-persisted token verbatim — it is a full @odata.nextLink/deltaLink.
//
// `baselineOnly` asks Graph for a delta link WITHOUT enumerating. This is the
// difference between "import everything from the beginning" and "tell me what
// changes from now on", and getting it wrong is not subtle:
//
// A delta query with no token performs an INITIAL FULL ENUMERATION. Graph
// returns every item in the folder, paged, and only emits @odata.deltaLink on
// the final page. The engine stores whatever comes back as the delta token, so
// page 1 of an enumeration becomes the "baseline" — and every later delta run
// walks that enumeration a page at a time, re-reading items the full walk had
// already imported (`0 written / 600 skipped`, run after run) while genuinely
// new items sit unreachable behind it. On a large mailbox that never converges.
//
// `$deltatoken=latest` (drive: `token=latest`) returns the delta link straight
// away with an empty page. The engine calls this ONLY after a full walk has
// materialized everything, which is exactly when "from now on" is correct.
function initialDeltaUrl(mount, resource, baselineOnly) {
  if (resource === "calendar") {
    var win = windowBounds(mount);
    // Mailbox-level, NOT `/calendars/{id}/calendarView/delta` — that route is
    // not part of v1.0. `calendarSupportsDelta` is what guarantees we only get
    // here for the primary calendar, so addressing the mailbox is correct.
    return (
      GRAPH + principal(mount) + "/calendarView/delta?startDateTime=" + enc(win.start) +
      "&endDateTime=" + enc(win.end) +
      (baselineOnly ? "&$deltatoken=latest" : "")
    );
  }
  if (resource === "files") {
    // Drive spells it differently: `token=latest`, no `$`.
    return GRAPH + driveContainer(mount) + "/delta" +
      (baselineOnly ? "?token=latest" : "");
  }
  return (
    GRAPH + principal(mount) + "/mailFolders/" + enc(mailFolderId(mount)) +
    "/messages/delta?$select=" + enc(mailSelect(mount)) +
    (baselineOnly ? "&$deltatoken=latest" : "")
  );
}

function opGetChanges(credential, mount, params) {
  var resource = resourceOf(mount);
  var token = params.since_token;
  // Only meaningful when there is no token yet — a stored token already IS a
  // resume point and must be used verbatim.
  var baselineOnly = !token && params.baseline_only === true;
  var url = token || initialDeltaUrl(mount, resource, baselineOnly);
  var resp = graphFetch(credential, "GET", url, { context: "get_changes" });
  var body = resp.body || {};
  var values = body.value || [];
  var items =
    resource === "calendar"
      ? calendarChanges(credential, mount, values)
      : values.map(function (v) {
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

// ONE NODE PER SERIES, not one per occurrence.
//
// The two calendar paths disagreed about what an item IS. The full walk reads
// `/events`, which returns single instances and SERIES MASTERS — one item per
// series, carrying `recurrence`. The delta path reads `/calendarView/delta`,
// which returns OCCURRENCES AND EXCEPTIONS expanded across the window — one
// item per instance, each with its own id and no `recurrence`. Since a node is
// keyed on the Graph id, a weekly meeting became ~5 nodes and a daily standup
// ~26, all siblings of the series-master node the full walk had already created
// for the same meeting, with nothing relating them.
//
// calendarView/delta is the only delta a v1.0 calendar has, so the fix is to
// collapse its output rather than abandon it: an occurrence or exception is
// reported as an update of its `seriesMasterId`, deduped within the page.
//
// Two consequences worth stating:
//  * A single recurring series changing produces ONE update no matter how many
//    of its occurrences moved.
//  * The master is fetched only when the page did not already contain it, so
//    the common case (a series edited as a whole) costs no extra request.
function calendarChanges(credential, mount, values) {
  var out = [];
  var emitted = {};
  var i;

  function emit(v) {
    if (!v || !v.id || emitted[v.id]) return;
    emitted[v.id] = true;
    var item = toExternalItem(v, "calendar");
    out.push({ type: "updated", item: item, relative_path: item.external_id });
  }

  // Series masters present in this page, so an occurrence of one of them needs
  // no extra fetch.
  var mastersInPage = {};
  for (i = 0; i < values.length; i++) {
    if (!values[i]["@removed"] && values[i].type === "seriesMaster") {
      mastersInPage[values[i].id] = values[i];
    }
  }

  for (i = 0; i < values.length; i++) {
    var v = values[i];

    if (v["@removed"]) {
      // A removal from calendarView is NOT necessarily a deletion. Microsoft
      // documents that within a date-bound view, `@removed` also covers events
      // that merely moved OUTSIDE the window — so treating every one as a delete
      // silently destroyed events an operator had only rescheduled. We cannot
      // tell the two apart from the delta payload, and deleting real content is
      // far worse than keeping a stale node, so only a removal we can attribute
      // to a whole series or a standalone event is acted on.
      //
      // A removed OCCURRENCE says nothing about its series: the series is still
      // there, and the next full walk reconciles anything genuinely gone.
      if (v.seriesMasterId) continue;
      out.push({ type: "deleted", item: { external_id: v.id }, relative_path: v.id });
      continue;
    }

    if (v.type === "occurrence" || v.type === "exception") {
      var masterId = v.seriesMasterId;
      if (!masterId) {
        // Shouldn't happen, but an occurrence with no master is better carried
        // through as itself than dropped.
        emit(v);
        continue;
      }
      if (emitted[masterId]) continue;
      var master = mastersInPage[masterId] || fetchEvent(credential, mount, masterId);
      // A master we cannot read (deleted between pages, or no access) is skipped
      // rather than materialized from the occurrence, which would reintroduce
      // exactly the per-occurrence nodes this exists to prevent.
      if (master) emit(master);
      continue;
    }

    emit(v);
  }
  return out;
}

// Read one event by id, or null when it is gone. Used to resolve an occurrence
// back to its series master when the delta page did not include it.
function fetchEvent(credential, mount, eventId) {
  var url = GRAPH + principal(mount) + "/events/" + enc(eventId) +
    "?$select=" + enc(EVENT_SELECT);
  var resp = graphFetch(credential, "GET", url, {
    context: "get_changes:series_master",
    rawStatusOk: true,
  });
  if (resp.status === 404) return null;
  raiseForStatus(resp, "get_changes:series_master");
  return resp.body || null;
}

// ---- dispatch -------------------------------------------------------------

function handler(input) {
  var operation = input.operation;
  var params = input.params || {};
  var credential = input.credential;
  var mount = input.mount || {};

  switch (operation) {
    case "capabilities":
      return opCapabilities(input.mount || {});
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
    case "browse":
      return opBrowse(credential, mount, params);
    default:
      throw new Error("Unsupported operation: " + operation);
  }
}
