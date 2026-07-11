/**
 * IMAP virtual-node adapter (native raisin.imap binding).
 *
 * Implements the frozen adapter contract (docs/reference/virtual-node-adapters.md)
 * for mailbox sync against a REAL IMAP server (RFC 3501) over implicit TLS. The
 * sync engine invokes this directly and materializes returned messages as
 * ephemeral nodes under the mount path — the "agents work the inbox" pattern.
 *   input = { operation, params, credential, mount }
 *
 * The IMAP protocol (TLS + LOGIN + UID FETCH) is owned by Rust and reached only
 * through the `raisin.imap.*` binding (fetchSince/listMailboxes/fetchMessage) —
 * no raw socket, no JMAP/HTTP. The binding enforces the function's
 * network_policy on `imaps://host:port` before opening any socket, so the
 * connection is authorized by this node's network_policy.allowed_urls.
 *
 * CREDENTIALS come from input.credential (never logged): `username` (now
 * provided by the engine from the OAuth account subject) plus a secret — app
 * password (`password`/`app_password`) or XOAUTH2 `access_token`.
 * CONNECTION SETTINGS (host/port/tls/mailbox/auth) come from the integration's
 * `mount.api_config` (template defaults) merged with the mount's
 * `mount.sync_config` (per-mount override, which wins when present). A rejected
 * LOGIN surfaces as `[imap:auth_expired]`, re-thrown as code "auth_expired" so
 * the engine runs the reconnect lifecycle.
 */

function coded(message, code) {
  var e = new Error(message);
  e.code = code;
  return e;
}

// The binding throws Error(message) carrying a stable machine tag (e.g.
// "[imap:auth_expired] ..."). Translate reserved tags into the engine's dispatch
// codes; leave everything else transient. Binding messages never leak the password.
function mapImapError(e) {
  var m = (e && e.message) || "";
  if (m.indexOf("[imap:auth_expired]") !== -1) return coded(m, "auth_expired");
  if (m.indexOf("[imap:rate_limited]") !== -1) return coded(m, "rate_limited");
  return e;
}

function imapCall(fn) {
  try {
    return fn();
  } catch (e) {
    throw mapImapError(e);
  }
}

// Resolve the effective connection settings by layering the integration's
// api_config (template defaults) under the mount's sync_config (per-mount
// override), so sync_config wins whenever it supplies a value. api_config names
// the mailbox `default_mailbox`; sync_config names it `mailbox`.
function connConfig(mount) {
  var api = (mount && mount.api_config) || {};
  var sync = (mount && mount.sync_config) || {};
  function pick(key) {
    return sync[key] !== undefined ? sync[key] : api[key];
  }
  return {
    host: pick("host"),
    port: pick("port"),
    tls: pick("tls"),
    auth: pick("auth"),
    mailbox: sync.mailbox !== undefined ? sync.mailbox : api.default_mailbox,
    username: pick("username"),
  };
}

// Build the { host, port, tls, auth, username, password } descriptor: host/port/
// tls/auth from the merged api_config + sync_config, identity from the decrypted
// credential. When the credential carries an OAuth2 `access_token` (and no
// static password), select the native SASL XOAUTH2 handshake; otherwise plain
// LOGIN.
function buildConn(credential, mount) {
  var cfg = connConfig(mount);
  var cred = credential || {};
  var staticPassword = cred.password || cred.app_password;
  var secret = staticPassword || cred.access_token;
  var username = cred.username || cred.user || cfg.username;
  if (!username || !secret) {
    throw new Error("IMAP credential missing username or password/access_token");
  }
  var useXoauth2 =
    cfg.auth === "xoauth2" || (!staticPassword && !!cred.access_token);
  return {
    host: cfg.host,
    port: cfg.port ? Number(cfg.port) : 993,
    tls: cfg.tls === false ? false : true,
    auth: useXoauth2 ? "xoauth2" : "password",
    username: username,
    password: secret,
  };
}

// Mailbox to sync: sync_config.mailbox, else api_config.default_mailbox, else
// the mount's remote_root, else INBOX.
function mailboxOf(mount) {
  return connConfig(mount).mailbox || (mount && mount.remote_root) || "INBOX";
}

function limitOf(mount) {
  var cfg = (mount && mount.sync_config) || {};
  var n = Number(cfg.max_items_per_sync);
  if (!n || n <= 0) return 200;
  return Math.min(n, 1000);
}

function parseToken(token) {
  if (!token) return { validity: null, uid: 0 };
  var parts = String(token).split(":");
  var validity = parts.length > 1 ? Number(parts[0]) : null;
  var uid = Number(parts[parts.length - 1]) || 0;
  if (isNaN(validity)) validity = null;
  return { validity: validity, uid: uid };
}

function formatToken(validity, uid) {
  return String(validity) + ":" + String(uid);
}

function isSeen(flags) {
  var f = flags || [];
  for (var i = 0; i < f.length; i++) {
    if (String(f[i]).replace(/\\/g, "").toLowerCase() === "seen") return true;
  }
  return false;
}

// Stable-when-unchanged etag: uid + sorted flags (read/unread change re-materializes)
// + uidvalidity (mailbox reset changes it).
function messageEtag(msg, validity) {
  var flags = (msg.flags || []).slice().sort().join(",");
  return (validity != null ? validity + ":" : "") + msg.uid + "|" + flags;
}

// Map a raisin.imap message (fetchSince summary or fetchMessage detail) to a
// normalized ExternalItem. from/to are already-formatted strings from the binding.
function messageToItem(msg, validity, mailboxPath) {
  var subject = msg.subject || "(no subject)";
  return {
    external_id: String(msg.uid),
    name: subject,
    mime_type: "message/rfc822",
    size_bytes: null,
    is_folder: false,
    parent_id: mailboxPath || null,
    created_at: msg.date || null,
    modified_at: msg.date || null,
    etag: messageEtag(msg, validity),
    web_url: null,
    download_url: null,
    metadata: {
      subject: subject,
      from: msg.from || null,
      to: msg.to || null,
      date: msg.date || null,
      snippet: msg.snippet || null,
      message_id: msg.message_id || null,
      thread_id: msg.thread_id || null,
      unread: !isSeen(msg.flags),
      flags: msg.flags || [],
      uid: msg.uid,
      uidvalidity: validity != null ? validity : null,
      headers: msg.headers || {},
    },
  };
}

// Derive a parent path from a hierarchical mailbox path (best-effort: IMAP
// delimiters vary, so try "/" and "."). Null when the mailbox is top-level.
function mailboxParent(path, name) {
  if (!path || !name || path === name) return null;
  var delims = ["/", "."];
  for (var i = 0; i < delims.length; i++) {
    var suffix = delims[i] + name;
    if (path.length > suffix.length && path.slice(-suffix.length) === suffix) {
      return path.slice(0, path.length - suffix.length);
    }
  }
  return null;
}

function mailboxToItem(mbox) {
  var flags = (mbox.flags || []).slice().sort().join(",");
  return {
    external_id: mbox.path,
    name: mbox.name,
    mime_type: null,
    size_bytes: null,
    is_folder: true,
    parent_id: mailboxParent(mbox.path, mbox.name),
    created_at: null,
    modified_at: null,
    etag: "mbx:" + mbox.path + "|" + flags,
    web_url: null,
    download_url: null,
    metadata: { path: mbox.path, flags: mbox.flags || [] },
  };
}

function opCapabilities(mount) {
  // Push (Gmail Pub/Sub watch) is offered ONLY when the mount configures a
  // pubsub_topic. Plain IMAP mounts (any RFC 3501 server) carry no topic and so
  // report supports_push:false — the engine keeps polling them. This keeps the
  // shared adapter generic: nothing Gmail-specific is forced on a non-Gmail mount.
  var canPush = !!pubsubTopic(mount);
  return {
    can_read: true,
    can_write: false,
    can_create_folders: false,
    supports_changes: true,
    supports_webhooks: canPush,
    supports_search: false,
    supports_push: canPush,
    // Ephemeral default: inbox messages expire after a day unless re-seen.
    default_ttl: 86400,
    max_file_size: null,
  };
}

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
function pubsubTopic(mount) {
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
function opSubscribe(credential, mount) {
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
function opRenew(credential, mount, params) {
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
function opUnsubscribe(credential) {
  gmailFetch(credential, "POST", "/stop");
  return { ok: true };
}

// Enumerate mailboxes (folders). Messages arrive via get_changes
// (supports_changes: true), so list returns only the folder structure.
function opList(credential, mount, params) {
  var conn = buildConn(credential, mount);
  var boxes = imapCall(function () {
    return raisin.imap.listMailboxes(conn);
  });
  var folderId = (params && params.folder_id) || null;
  var items = (boxes || [])
    .map(mailboxToItem)
    .filter(function (m) {
      return (m.parent_id || null) === folderId;
    });
  return { items: items, next_cursor: null };
}

function opGet(credential, mount, params) {
  if (!params || params.item_id == null) return null;
  var conn = buildConn(credential, mount);
  var mbox = mailboxOf(mount);
  var msg = imapCall(function () {
    return raisin.imap.fetchMessage(conn, Number(params.item_id), { mailbox: mbox });
  });
  if (!msg) return null;
  return messageToItem(msg, null, mbox);
}

// Message body: plain text preferred, HTML fallback.
function opGetContent(credential, mount, params) {
  var conn = buildConn(credential, mount);
  var mbox = mailboxOf(mount);
  var msg = imapCall(function () {
    return raisin.imap.fetchMessage(conn, Number(params.item_id), { mailbox: mbox });
  });
  if (!msg) return { content: "", mime_type: "text/plain" };
  if (msg.text) return { content: msg.text, mime_type: "text/plain" };
  if (msg.html) return { content: msg.html, mime_type: "text/html" };
  return { content: msg.snippet || "", mime_type: "text/plain" };
}

// Read-only adapter: writes are gated off by capabilities, but guard anyway.
function opUnsupported(operation) {
  throw new Error("Operation not supported by the IMAP adapter (read-only): " + operation);
}

// Incremental delta. since_token encodes "uidvalidity:uid"; fetch UID > cursor,
// forcing a full resync from 0 on a UIDVALIDITY change. NEVER returns next_token
// null — the (possibly unchanged) cursor is always returned.
function opGetChanges(credential, mount, params) {
  var conn = buildConn(credential, mount);
  var mbox = mailboxOf(mount);
  var limit = limitOf(mount);
  var tok = parseToken(params && params.since_token);

  var res = imapCall(function () {
    return raisin.imap.fetchSince(conn, tok.uid, { mailbox: mbox, limit: limit });
  });
  var validity = res.uidvalidity;
  var messages = res.messages || [];
  var highest = res.highestUid;

  // UIDVALIDITY reset: the UID space changed, so the cursor is meaningless.
  // Re-list from UID 0 and re-emit — engine upserts are idempotent (matched by
  // external_id, skip-write by etag).
  if (tok.validity !== null && validity !== tok.validity) {
    var full = imapCall(function () {
      return raisin.imap.fetchSince(conn, 0, { mailbox: mbox, limit: limit });
    });
    validity = full.uidvalidity;
    messages = full.messages || [];
    highest = full.highestUid;
  }

  var items = messages.map(function (m) {
    return {
      type: "created",
      item: messageToItem(m, validity, mbox),
      relative_path: m.subject || String(m.uid),
    };
  });

  // Never null: nothing new -> highest === tok.uid, validity unchanged, cursor intact.
  return { items: items, next_token: formatToken(validity, highest) };
}

function handler(input) {
  var operation = input.operation;
  var params = input.params || {};
  var credential = input.credential;
  var mount = input.mount || {};

  switch (operation) {
    case "capabilities":
      return opCapabilities(mount);
    case "list":
      return opList(credential, mount, params);
    case "get":
      return opGet(credential, mount, params);
    case "get_content":
      return opGetContent(credential, mount, params);
    case "create":
    case "update":
    case "delete":
      return opUnsupported(operation);
    case "get_changes":
      return opGetChanges(credential, mount, params);
    // Push lifecycle (Gmail Pub/Sub watch). No-op-guarded by capabilities:
    // the engine only calls these when supports_push is true (pubsub_topic set).
    case "subscribe":
      return opSubscribe(credential, mount);
    case "renew":
      return opRenew(credential, mount, params);
    case "unsubscribe":
      return opUnsubscribe(credential);
    default:
      throw new Error("Unsupported operation: " + operation);
  }
}
