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
 *
 * SENDING does not go over IMAP, because IMAP cannot send. A `submit` mount
 * (an outbox) hands the message to the tenant's configured email provider via
 * `raisin.email.send` — see `send.js` for what that means for the From address
 * and for the Sent folder.
 */

import { coded } from "./common.js";
import { connConfig } from "./mount.js";
import { opSubmit, resolveSender } from "./send.js";
import { opRenew, opSubscribe, opUnsubscribe, pubsubTopic } from "./gmail-push.js";

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
  var caps = {
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
    // No send here can carry a provider-side idempotency key: SMTP has nothing
    // at all, and the email API exposes nothing either. Declared honestly and
    // never as an aspiration — the only thing a false `true` would change is
    // what an operator believes about a duplicate they are looking at.
    supports_idempotency_key: false,
  };

  // can_submit ONLY when a sender is actually resolvable RIGHT NOW. The probe
  // is one node read (`/config/email`), runs once per sync run, and can never
  // fail the run — resolveSender catches everything.
  //
  // `can_write` rides with it because the engine's `missing_submit_ops` demands
  // both: it is the umbrella flag saying this adapter changes anything at the
  // provider at all, and can_submit without it is a self-contradiction. It does
  // NOT make the mount a mirror — can_create/can_update/can_delete stay absent,
  // so a `mirror` or `state_only` mount is still refused, with those names in
  // the reason. Writing FLAGS back (UID STORE) is what would make this mount
  // state_only, and that needs a Rust binding this adapter does not have.
  var sender = resolveSender(mount);
  if (sender.ok) {
    caps.can_write = true;
    caps.can_submit = true;
  } else {
    // The engine's typed Capabilities drops keys it does not know, so this
    // string does not reach the mount's writeback_last_error today — the engine
    // writes its own ("adapter does not declare can_submit"). It is carried
    // anyway because it is the ONLY place the actual cause is stated, and it is
    // visible wherever the adapter is invoked directly (the console's function
    // runner, an adapter test). The same reason is thrown, terminally, by
    // `submit` if a command is ever queued against a mount in this state.
    caps.submit_unavailable_reason = sender.reason;
  }
  return caps;
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

// No MIRROR surface: the mount is never the remote object. create/update/delete
// are gated off by capabilities (none of the three flags is declared), but
// guarded anyway. `submit` is the one write this adapter has, and it issues a
// command through the tenant's email provider rather than mirroring anything.
function opUnsupported(operation) {
  throw new Error(
    "Operation not supported by the IMAP adapter: " +
      operation +
      ". IMAP is a read protocol; the only write this connector has is `submit` " +
      "(an outbox mount, which sends through the tenant's configured email provider)."
  );
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

export function handler(input) {
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
    // The outbox. Never reached unless capabilities resolved a sender, but it
    // re-resolves one anyway rather than trusting a probe from earlier in the run.
    case "submit":
      return opSubmit(mount, params);
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
