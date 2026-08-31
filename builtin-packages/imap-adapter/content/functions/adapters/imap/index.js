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
import { connConfig, mountSetting } from "./mount.js";
import {
  mailboxChain,
  mailboxDelimiter,
  mailboxParentPath,
  segment,
  selectable,
  skipByAttribute,
} from "./mailboxes.js";
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

/**
 * SUBTREE MODE. `sync_config.folder_scope: "folder" | "tree"`, default "folder".
 *
 * "tree" only for the literal string, so every mount that exists today — and
 * every one whose operator never touches the key — keeps folder mode byte for
 * byte. `sync_config` is not deny_unknown_fields on the engine side, so the key
 * reaches this adapter through mount.config / mount.sync_config with no engine
 * change and nothing new on the Capabilities struct.
 */
function folderScope(mount) {
  return mountSetting(mount, "folder_scope") === "tree" ? "tree" : "folder";
}

// Cursor family marker. A folder-mode cursor is "<uidvalidity>:<uid>"; a
// tree-mode one is this prefix plus JSON. The prefix is what makes flipping
// folder_scope safe in both directions: neither parser can be fooled by the
// other's token, so the flip re-baselines instead of resuming from a cursor that
// means something else. The engine treats last_sync_token as an opaque string
// owned by the adapter, so carrying a per-mailbox map inside it breaks no
// one-cursor-per-mount rule.
var TREE_CURSOR_PREFIX = "rsn-imaptree-1:";

// A HARD CEILING, and a throw rather than a truncation.
//
// Truncating the mailbox set would hand the engine's full walk a PARTIAL `seen`,
// and reconcile would then delete the mailboxes it never heard about along with
// everything under them. Refusing loudly is the only safe answer, and the
// operator's fix is to mount a subtree instead of the whole account.
var MAX_TREE_MAILBOXES = 50;

// Mailboxes advanced per get_changes call.
//
// EVERY BINDING CALL IS ITS OWN TCP + TLS + LOGIN + LOGOUT (client.rs
// connect_login runs at the top of fetch_since_inner and the session is logged
// out at the bottom), so this number IS the number of logins one call costs.
// Gmail allows 15 simultaneous IMAP connections per account; 5 sequential ones
// is polite, and the rotation index below means the remaining mailboxes are
// reached on the following call rather than never. `fetchSinceMulti` — one
// session, N SELECTs — is the binding change that would let this be 25; the
// code picks it up automatically the day it exists.
var MAILBOXES_PER_CALL = 5;
var MAILBOXES_PER_CALL_MULTI = 25;

function hasFetchSinceMulti() {
  return !!(
    typeof raisin !== "undefined" &&
    raisin.imap &&
    typeof raisin.imap.fetchSinceMulti === "function"
  );
}

/**
 * Every mailbox at or below the mount root, in one stable order, each carrying
 * the segment chain that IS its relative path.
 *
 * Sorted by path so an ancestor is always seen before its descendants — which is
 * what lets a \All / \Trash / \Junk mailbox take its whole subtree out with it
 * — and so the delta's rotation order is the same on every call.
 */
function treeMailboxes(boxes, rootPath) {
  var sorted = (boxes || []).slice().sort(function (a, b) {
    return String(a.path) < String(b.path) ? -1 : String(a.path) > String(b.path) ? 1 : 0;
  });
  var skipped = [];
  var out = [];
  for (var i = 0; i < sorted.length; i++) {
    var mbox = sorted[i];
    var delim = mailboxDelimiter(mbox);
    var chain = mailboxChain(mbox.path, delim, rootPath);
    // null = not under this mount's root. SKIP, never "place it at the root".
    if (chain === null) continue;
    var key = chain.join("\u0000");
    var under = false;
    for (var k = 0; k < skipped.length; k++) {
      if (key === skipped[k] || key.indexOf(skipped[k] + "\u0000") === 0) under = true;
    }
    if (under) continue;
    if (skipByAttribute(mbox.flags)) {
      skipped.push(key);
      continue;
    }
    out.push({
      path: mbox.path,
      // The leaf of the chain, so the walk's `item.name` and the delta's path
      // segment are produced by the same sanitizer and cannot disagree.
      name: chain.length ? chain[chain.length - 1] : segment(mbox.name || mbox.path),
      chain: chain,
      flags: mbox.flags || [],
      // \Noselect mailboxes stay in the HIERARCHY (dropping them would orphan
      // their children, which the walk could then never reach) but are never
      // SELECTed for messages.
      selectable: selectable(mbox.flags),
    });
  }
  if (out.length > MAX_TREE_MAILBOXES) {
    throw coded(
      "IMAP tree mount spans " +
        out.length +
        " mailboxes, above the " +
        MAX_TREE_MAILBOXES +
        " ceiling. Each one costs a login per poll; mount a subtree (set the " +
        "mount's mailbox/remote_root deeper) rather than the whole account.",
      "config_error"
    );
  }
  return out;
}

function chainStartsWith(chain, prefix) {
  if (chain.length <= prefix.length) return false;
  for (var i = 0; i < prefix.length; i++) {
    if (chain[i] !== prefix[i]) return false;
  }
  return true;
}

function parseTreeCursor(token) {
  if (!token || String(token).indexOf(TREE_CURSOR_PREFIX) !== 0) return null;
  var body;
  try {
    body = JSON.parse(String(token).slice(TREE_CURSOR_PREFIX.length));
  } catch (e) {
    return null;
  }
  if (!body || typeof body !== "object") return null;
  return {
    r: Number(body.r) || 0,
    p: Number(body.p) || 0,
    // "The baseline could not seed every mailbox in its one call; keep seeding
    // the rest a slice at a time, and emit nothing for them." Carried in the
    // cursor because the baseline is a SINGLE call the engine never pages
    // (full_reconcile.rs capture_delta_baseline keeps `next_token` and drops
    // everything else), so there is nowhere else to remember it.
    s: Number(body.s) || 0,
    m: body.m && typeof body.m === "object" ? body.m : {},
  };
}

function formatTreeCursor(cur) {
  return (
    TREE_CURSOR_PREFIX + JSON.stringify({ v: 1, r: cur.r, p: cur.p, s: cur.s || 0, m: cur.m })
  );
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

/**
 * THE ONE SPELLING OF A TREE-MODE MESSAGE'S IDENTITY.
 *
 * external_id and relative_path are BOTH built from this string, and that is the
 * whole point of it existing. They used to be spelled separately — the id was
 * "<mailbox>|<uidvalidity>:<uid>" and the path leaf was the bare "<uid>" — so a
 * UIDVALIDITY reset (a mailbox restored from backup, a server that renumbers)
 * minted a NEW node id for every message while re-enumerating from uid 0, i.e.
 * at exactly the paths the OLD nodes still occupy. The materializer matches on
 * __external_id, finds no match, and tries to CREATE at an occupied path;
 * add_node refuses that with Error::Conflict, which is item-level, so the item
 * is skipped and the run still reports ok. Every message in the mailbox, every
 * run, for as long as the old nodes live — and they live until the mount's TTL
 * retires them, because a tree mount runs with reconcile_deletes:false and the
 * walk never enumerates messages. Net effect: a restored mailbox silently
 * stops importing.
 *
 * "." and not ":" as the separator because this string is a PATH SEGMENT as well
 * as an id: raisin_core::sanitize_name keeps [a-z0-9-_.] and DROPS anything
 * else, so "100:5" and "10:05" would both slug to "1005" and collide.
 */
function messageKey(msg, validity) {
  return (validity != null ? validity : 0) + "." + msg.uid;
}

// Map a raisin.imap message (fetchSince summary or fetchMessage detail) to a
// normalized ExternalItem. from/to are already-formatted strings from the binding.
function messageToItem(msg, validity, mailboxPath, chain) {
  var subject = msg.subject || "(no subject)";
  // TREE MODE USES A DIFFERENT ID SPACE, and that is not an implementation
  // detail an operator can ignore. IMAP UID and UIDVALIDITY are per MAILBOX, so
  // the bare uid collapses INBOX uid 5 and Archive uid 5 onto ONE node the
  // moment a mount spans more than one mailbox. Namespacing it is the only fix,
  // and it means switching an existing mount from `folder` to `tree` re-imports
  // it once, with new node ids and per-node history restarting — exactly like
  // the ms-graph immutable-ids migration. Stated in the README before anyone
  // flips the switch on a live mount.
  var id =
    chain === null || chain === undefined
      ? String(msg.uid)
      : mailboxPath + "|" + messageKey(msg, validity);
  return {
    external_id: id,
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
      // The mailbox this message was actually fetched from. Folder mode has
      // always had exactly one, and the mapper kept reading it off
      // mount.remote_root; tree mode has N, so the item has to carry its own or
      // every message in the tree claims to live in the mount root.
      mailbox: mailboxPath || null,
      folder_path: chain ? chain.join("/") : null,
      headers: msg.headers || {},
    },
  };
}

function mailboxToItem(mbox) {
  var flags = (mbox.flags || []).slice().sort().join(",");
  return {
    external_id: mbox.path,
    name: mbox.name,
    mime_type: null,
    size_bytes: null,
    is_folder: true,
    // mailboxes.js, NOT the old "/"-then-"." guess this line used to make. The
    // guess returned null for every server whose delimiter is neither, which
    // flattened its whole folder tree into the mount root; the replacement reads
    // back the delimiter the binding itself used to split the leaf off the path,
    // so it is exact per mailbox (RFC 2342 allows two namespaces on one server to
    // differ) and identical to the old answer for "/" and ".".
    parent_id: mailboxParentPath(mbox),
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
//
// CONSEQUENCE, and there is no fix available inside this adapter: because the
// walk yields no messages, a FORCED FULL reconcile stages every message node
// under the mount for deletion — `seen` holds mailbox external_ids only, it is
// non-empty so the engine's empty-reconcile guard does not fire, and the stale
// filter exempts only `is_command` nodes. They do not come back: get_changes
// resumes from the highest UID already seen. Survivable only because the
// documented mount layout is `ephemeral: true` with a 24h TTL, i.e. a cache; a
// non-ephemeral IMAP mount loses them permanently. See README, "Forcing a FULL
// resync prunes every message node". Do NOT make this function enumerate
// messages to dodge it — that re-opens the full-vs-delta relative_path
// divergence (a walk would nest them under the mailbox, `INBOX/<subject>`,
// while opGetChanges below emits the BARE subject — or the UID when a message
// has none — so the same message lands at two paths and the engine remaps it on
// every disagreeing run) and costs a full mailbox fetch per run. The fix belongs
// in the engine contract.
function opList(credential, mount, params) {
  var conn = buildConn(credential, mount);
  var boxes = imapCall(function () {
    return raisin.imap.listMailboxes(conn);
  });
  var folderId = (params && params.folder_id) || null;
  if (folderScope(mount) === "tree") return listTree(mount, boxes, folderId);
  var items = (boxes || [])
    .map(mailboxToItem)
    .filter(function (m) {
      return (m.parent_id || null) === folderId;
    });
  return { items: items, next_cursor: null };
}

/**
 * The tree walk: one level of MAILBOXES, scoped to the mount root.
 *
 * Still mailboxes only, deliberately. Enumerating messages here would cost a
 * full mailbox fetch per run AND re-open the full-vs-delta path divergence, so
 * the walk is authoritative for FOLDERS and never for messages — which is
 * exactly why a tree mount must keep `sync_config.reconcile_deletes: false` and
 * why the shipped bundle sets it.
 *
 * The engine drives this with folder_id = the previous item's external_id and
 * accumulates the prefix from each item's `name` (full.rs resolve_item_path), so
 * a folder's walk path is its chain joined with "/" — the same array
 * `opGetChanges` joins for a message's relative_path.
 */
function listTree(mount, boxes, folderId) {
  var rootPath = mailboxOf(mount);
  var all = treeMailboxes(boxes, rootPath);
  var containerChain = [];
  var containerPath = rootPath;
  // THE WALK'S FIRST folder_id IS mount.remote_root, NOT the mailbox this mount
  // actually roots at: full.rs seeds its stack with `ctx.mount.remote_root`,
  // while `mailboxOf` prefers `sync_config.mailbox`. Those two differ the moment
  // an operator answers the shipped bundle's Mailbox prompt, because the prompt
  // writes `sync_config.mailbox` onto an entry whose `remote_root` is INBOX.
  // Without this equivalence the walk looked "INBOX" up among the mailboxes
  // BELOW INBOX.Projects, found none, and returned an empty FIRST page: not one
  // folder node was ever created for a tree mount configured the ordinary way,
  // while the delta went on emitting messages at paths under folders that did
  // not exist.
  var isRoot = !folderId || folderId === rootPath || folderId === (mount && mount.remote_root);
  if (!isRoot) {
    var found = null;
    for (var i = 0; i < all.length; i++) {
      if (all[i].path === folderId) found = all[i];
    }
    // A mailbox that vanished between the parent page and this call. Returning
    // an empty page is right: emitting the whole root here instead would move
    // every mailbox under a folder that no longer exists.
    if (!found) return { items: [], next_cursor: null };
    containerChain = found.chain;
    containerPath = found.path;
  }
  var items = [];
  for (var j = 0; j < all.length; j++) {
    var d = all[j];
    if (d.chain.length !== containerChain.length + 1) continue;
    if (!chainStartsWith(d.chain, containerChain)) continue;
    items.push({
      external_id: d.path,
      name: d.name,
      mime_type: null,
      size_bytes: null,
      is_folder: true,
      parent_id: containerPath,
      created_at: null,
      modified_at: null,
      etag: folderEtag(mount, d),
      web_url: null,
      download_url: null,
      metadata: { path: d.path, flags: d.flags, selectable: d.selectable },
    });
  }
  return { items: items, next_cursor: null };
}

/**
 * ONE spelling of a tree-mode mailbox's etag, for the walk and the delta alike.
 *
 * Built in one place for the same reason `messageKey` is: the two sides compare
 * this string against the stored one to decide whether to write, so a folder
 * whose etag the walk spells differently from the delta is rewritten on every
 * run that changes hands between them.
 */
function folderEtag(mount, d) {
  return "mbx:" + d.path + "|" + d.flags.slice().sort().join(",") + folderEtagBucket(mount);
}

/**
 * Split an item id back into the mailbox and UID to FETCH.
 *
 * Tree mode's id is "<mailboxPath>|<uidvalidity>.<uid>" (see messageKey); folder
 * mode's is the bare uid. Reading the mailbox off the id rather than off the
 * mount is what makes `get` and `get_content` work at all on a tree mount — the
 * mount names one mailbox and the message may be in any of N.
 *
 * ":" is still accepted in the tail: that was the separator before the id and
 * the path leaf were unified, and an id minted by an older build must keep
 * fetching rather than resolve to NaN.
 */
function parseItemRef(mount, itemId) {
  var s = String(itemId == null ? "" : itemId);
  var bar = s.lastIndexOf("|");
  if (bar > 0) {
    var tail = s.slice(bar + 1);
    var sep = Math.max(tail.lastIndexOf("."), tail.lastIndexOf(":"));
    return {
      mailbox: s.slice(0, bar),
      uid: Number(sep >= 0 ? tail.slice(sep + 1) : tail),
    };
  }
  return { mailbox: mailboxOf(mount), uid: Number(s) };
}

function opGet(credential, mount, params) {
  if (!params || params.item_id == null) return null;
  var conn = buildConn(credential, mount);
  var ref = parseItemRef(mount, params.item_id);
  var msg = imapCall(function () {
    return raisin.imap.fetchMessage(conn, ref.uid, { mailbox: ref.mailbox });
  });
  if (!msg) return null;
  var item = messageToItem(msg, null, ref.mailbox, null);
  // The id the caller asked for IS this item's id. Rebuilt from the message
  // alone it would be the bare uid, and on a tree mount the stored id is
  // "<mailbox>|<uidvalidity>.<uid>" — a caller matching the answer against what
  // it asked for would conclude it had been handed a different message.
  item.external_id = String(params.item_id);
  return item;
}

// Message body: plain text preferred, HTML fallback.
function opGetContent(credential, mount, params) {
  var conn = buildConn(credential, mount);
  var ref = parseItemRef(mount, params && params.item_id);
  var msg = imapCall(function () {
    return raisin.imap.fetchMessage(conn, ref.uid, { mailbox: ref.mailbox });
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
  if (folderScope(mount) === "tree") return getChangesTree(credential, mount, params);
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
      item: messageToItem(m, validity, mbox, null),
      relative_path: m.subject || String(m.uid),
    };
  });

  // Never null: nothing new -> highest === tok.uid, validity unchanged, cursor intact.
  return { items: items, next_token: formatToken(validity, highest) };
}

// One mailbox's slice of a tree delta. Split out so the single-call path and a
// future fetchSinceMulti batch produce identical items from identical inputs.
function fetchOne(conn, desc, entry, perBox) {
  var since = entry ? Number(entry.uid) || 0 : 0;
  var res = imapCall(function () {
    return raisin.imap.fetchSince(conn, since, { mailbox: desc.path, limit: perBox });
  });
  // UIDVALIDITY IS COMPARED PER MAILBOX, not globally.
  //
  // Folder mode has one mailbox and so one comparison. Doing that globally in a
  // tree would re-fetch every mailbox in the account the moment ONE of them
  // reset its UID space — N full enumerations to repair one mailbox.
  if (entry && entry.uv != null && res.uidvalidity !== entry.uv) {
    res = imapCall(function () {
      return raisin.imap.fetchSince(conn, 0, { mailbox: desc.path, limit: perBox });
    });
  }
  return res;
}

/**
 * THE FOLDER HIERARCHY MUST BE KEPT ALIVE BY THE DELTA, NOT ONLY BY THE WALK.
 *
 * A tree mount ships `ephemeral: true` + `ttl_seconds: 86400` (see the bundles
 * in content/_raisin__system/connectors/<name>/.node.yaml) because IMAP has no
 * EXPUNGE feed, so a TTL is the only thing that retires a message that is gone
 * from the server. But `ephemeral::cleanup_expired` deletes EVERY mount-owned
 * node whose `__synced_at + ttl` has passed — there is no is_folder exemption —
 * and the walk that created the folder nodes runs exactly ONCE: after
 * `backfill_complete` the engine only ever calls get_changes
 * (delta.rs `want_baseline`, full_reconcile.rs sets the flag). So without this
 * function, 24h after the backfill:
 *
 *   1. every raisin:Folder node under a tree mount is deleted;
 *   2. the next message in that mailbox re-creates the ancestor through
 *      `upsert_deep_node`, as a stub with NO `__mount_id` and none of the
 *      mapper's properties (materializer/node_paths.rs `ensure_folder_chain`);
 *   3. from then on the folder is FOREIGN to the mount, so any later walk that
 *      stages the real folder item is skipped forever — "foreign node occupies
 *      target path, skipping" (materializer/stage.rs, the `!entry.mount_owned`
 *      arm). The mount can never own its own hierarchy again, and a mailbox
 *      deleted upstream can never be pruned, because nothing mount-owned is
 *      left to prune.
 *
 * The same emission is what gives a mailbox CREATED AFTER the backfill a real
 * folder node at all, instead of the same anonymous stub.
 *
 * Costs no extra IMAP call: `listMailboxes` is already made once per
 * get_changes. Emitted only at the START of a rotation round, so it is one
 * folder page per run rather than one per rotation slice.
 */
function treeFolderChanges(mount, all) {
  var byChain = {};
  for (var i = 0; i < all.length; i++) byChain[all[i].chain.join("\u0000")] = all[i].path;
  var rootPath = mailboxOf(mount);
  var out = [];
  for (var j = 0; j < all.length; j++) {
    var d = all[j];
    // The mount root itself. Its relative_path would be "", which delta.rs
    // rejects as "no name and no relative_path" and fails the whole run on.
    if (!d.chain.length) continue;
    var parentKey = d.chain.slice(0, d.chain.length - 1).join("\u0000");
    out.push({
      type: "created",
      item: {
        external_id: d.path,
        name: d.name,
        mime_type: null,
        size_bytes: null,
        is_folder: true,
        // Falls back to the root when the server listed a child without its
        // parent (legal in RFC 3501); upsert_deep_node still builds the chain
        // from relative_path, so the node lands in the right place either way.
        parent_id: d.chain.length > 1 ? byChain[parentKey] || rootPath : rootPath,
        created_at: null,
        modified_at: null,
        // Byte-identical to the walk's etag apart from the bucket, so a folder
        // whose flags did not move is not rewritten by the walk and the delta in
        // turn.
        etag: folderEtag(mount, d),
        web_url: null,
        download_url: null,
        metadata: { path: d.path, flags: d.flags, selectable: d.selectable },
      },
      relative_path: d.chain.join("/"),
    });
  }
  return out;
}

/**
 * The freshness half of a tree folder's etag, or "" when nothing expires.
 *
 * An UNCHANGED etag is not enough to keep a node alive: the materializer's
 * etag skip-write returns `Staged::Skipped` WITHOUT re-stamping `__synced_at`
 * (materializer/stage.rs, `can_skip_unmapped` in batch.rs), so re-emitting a
 * folder with the etag the walk already stored would be a no-op and the node
 * would still be swept at `ttl_seconds`. The etag therefore has to move.
 *
 * Three times per TTL window: often enough that a missed poll, a paused mount
 * or a slow round cannot let a folder cross the expiry line, rare enough that
 * the node is rewritten 3x/day rather than on every 300s poll — a rewrite is a
 * revision and a `node:updated` event, which downstream triggers see.
 *
 * Empty when the mount is not ephemeral or has no TTL: there is nothing to
 * outrun, so those mounts pay no churn at all.
 */
function folderEtagBucket(mount) {
  // `mount.sync_config` and NOT the merged `mount.config`: these two keys are
  // the engine's own, and the sweep this bucket exists to outrun reads them off
  // sync_config. Reading a different place could disagree with it.
  var cfg = (mount && mount.sync_config) || {};
  if (cfg.ephemeral !== true) return "";
  var ttl = Number(cfg.ttl_seconds);
  if (!ttl || ttl <= 0) return "";
  var step = Math.max(1, Math.floor(ttl / 3));
  return "|t" + Math.floor(Date.now() / 1000 / step);
}

/**
 * Tree delta: a per-mailbox cursor map inside the one opaque token, advanced a
 * BOUNDED slice at a time from a persisted rotation index.
 *
 * The rotation index lives in the cursor and not in a local, because without it
 * one busy mailbox consumes `max_items_per_sync` every run and the other 49
 * never advance — and nothing reports it, since the run still ends `ok` with
 * items written.
 *
 * WHAT THIS STILL CANNOT SEE, in tree mode exactly as in folder mode: the
 * binding's fetchSince is a UID SEARCH for UID > cursor. There is no CONDSTORE
 * and no EXPUNGE handling, so a flag change or a deletion on the server is
 * invisible incrementally, and a deleted message only leaves through
 * `ephemeral: true` + `ttl_seconds`. The walk cannot remove it either — it
 * enumerates mailboxes, never messages — which is the whole reason a tree mount
 * ships with `reconcile_deletes: false`.
 */
function getChangesTree(credential, mount, params) {
  var conn = buildConn(credential, mount);
  var rootPath = mailboxOf(mount);
  var limit = limitOf(mount);
  var boxes = imapCall(function () {
    return raisin.imap.listMailboxes(conn);
  });
  var all = treeMailboxes(boxes, rootPath);
  var order = all.filter(function (d) {
    return d.selectable;
  });

  var prev = parseTreeCursor(params && params.since_token);
  var cur = { r: prev ? prev.r : 0, p: prev ? prev.p : 0, s: prev ? prev.s : 0, m: {} };
  var seedCap = hasFetchSinceMulti() ? MAILBOXES_PER_CALL_MULTI : MAILBOXES_PER_CALL;
  // Reconcile the map against the mailboxes that exist NOW. An entry for a
  // mailbox that is gone is dropped and NOTHING is emitted for it: a mailbox
  // also "disappears" when access to it is lost, and its messages are not ours
  // to delete on that evidence.
  for (var i = 0; i < order.length; i++) {
    var have = prev && prev.m ? prev.m[order[i].path] : null;
    cur.m[order[i].path] = have && typeof have === "object" ? have : null;
  }

  // The folder page. Emitted at the START of a round (`p === 0`) and BEFORE any
  // message, because the batch applies in order and a message staged ahead of
  // its own folder is what makes `upsert_deep_node` mint the un-owned stub in
  // the first place. See `treeFolderChanges` for why the delta has to carry
  // folders at all.
  //
  // Not on the baseline call: `capture_delta_baseline` discards items, and the
  // walk that runs immediately before it has just created these nodes.
  var folderItems =
    params && params.baseline_only === true ? [] : cur.p ? [] : treeFolderChanges(mount, all);

  if (!order.length) {
    return { items: folderItems, next_token: formatTreeCursor(cur), has_more: false };
  }

  // BASELINE. The engine's capture_delta_baseline throws the items away and
  // keeps only next_token, so answering it with real messages would fetch every
  // mailbox in the tree and discard the lot. Instead, probe each unseeded
  // mailbox for its current highest UID with a limit of ONE message — the
  // binding keeps the newest `limit` UIDs, so one fetch names the watermark —
  // and emit nothing. Same "from now on" semantics folder mode has had since
  // the first walk; see the README.
  if (params && params.baseline_only === true) {
    // BOUNDED BY THE SAME SLICE AS EVERYTHING ELSE. Seeding all 50 mailboxes
    // here would be 50 sequential TCP+TLS+LOGINs inside ONE function
    // invocation (timeout_ms 180000), and the baseline is the one call with no
    // retry worth the name: capture_delta_baseline logs a warning and leaves
    // the mount on the FULL-WALK path forever if it throws. So seed a slice,
    // set `s`, and let the ordinary polls finish the job — each of those has a
    // durable cursor behind it.
    var seeded = 0;
    for (var b = 0; b < order.length; b++) {
      if (cur.m[order[b].path]) continue;
      if (seeded >= seedCap) {
        cur.s = 1;
        continue;
      }
      var probe = fetchOne(conn, order[b], null, 1);
      cur.m[order[b].path] = { uv: probe.uidvalidity, uid: probe.highestUid };
      seeded++;
    }
    cur.p = 0;
    return { items: [], next_token: formatTreeCursor(cur), has_more: false };
  }

  var n = order.length;
  // Never more than the round has left to do. Without the `n - cur.p` term the
  // last call of a round wraps past the end and re-visits mailboxes it already
  // advanced this round — free in items (their cursors are current) but not in
  // LOGINS, which is the resource that is actually scarce here.
  var remaining = Math.max(1, n - cur.p);
  var slice = Math.min(n, remaining, seedCap);
  // THE FULL BUDGET PER MAILBOX, not a fifth of it, and that is a data-loss
  // decision rather than a tuning one. client.rs `fetch_since_inner` keeps the
  // NEWEST `limit` UIDs above the cursor and reports the highest as the new
  // watermark, so any new message beyond that limit is stepped over and can
  // never be asked for again — the binding has no oldest-first or UID-range
  // fetch. Dividing the budget by the slice size lowered that cliff from
  // `max_items_per_sync` to a fifth of it for no gain: the rotation index, not
  // a smaller page, is what stops one mailbox starving the others, and it does
  // so across CALLS rather than by truncating a mailbox mid-page.
  var perBox = limit;
  var start = ((cur.r % n) + n) % n;
  var items = folderItems.slice();
  // Messages only. The folder page is bounded by the mailbox ceiling and must
  // not eat the message budget on a mount whose `max_items_per_sync` is small —
  // a round that emitted folders and no mail would still advance the rotation,
  // so the mail would be stepped over.
  var emitted = 0;
  var advanced = 0;
  var mailboxHasMore = false;

  for (var k = 0; k < slice; k++) {
    var desc = order[(start + k) % n];
    var entry = cur.m[desc.path];
    // Still seeding: this mailbox existed when the baseline ran but the
    // baseline's slice did not reach it. Probe its watermark and emit nothing,
    // exactly as the baseline would have. Only ever reached under `s`, so a
    // mailbox that appears LATER — or a mount whose cursor is a folder-mode
    // token — still enumerates from uid 0 as before.
    if (!entry && cur.s) {
      var seedRes = fetchOne(conn, desc, null, 1);
      cur.m[desc.path] = { uv: seedRes.uidvalidity, uid: seedRes.highestUid };
      advanced++;
      continue;
    }
    var res = fetchOne(conn, desc, entry, perBox);
    var validity = res.uidvalidity;
    var messages = res.messages || [];
    cur.m[desc.path] = { uv: validity, uid: res.highestUid };
    for (var mi = 0; mi < messages.length; mi++) {
      var msg = messages[mi];
      // relative_path is the mailbox chain plus messageKey — NOT the subject,
      // and NOT the bare uid.
      //
      // Not the subject: folder mode's bare subject is the divergence the README
      // documents — two messages with the same subject collide onto one path,
      // and the walk would place them somewhere else again. The chain comes from
      // the same mailboxes.js call the walk used for the folder item, so the
      // message lands under the folder node the walk created.
      //
      // Not the bare uid: the leaf has to carry the same uidvalidity the
      // external_id carries, or a UIDVALIDITY reset re-enumerates from uid 0
      // under a NEW id at the SAME path — a new node aimed at a path the old
      // node still holds. Both come out of messageKey precisely so they cannot
      // drift apart again.
      var key = messageKey(msg, validity);
      var rel = desc.chain.length ? desc.chain.join("/") + "/" + key : key;
      items.push({
        type: "created",
        item: messageToItem(msg, validity, desc.path, desc.chain),
        relative_path: rel,
      });
      emitted++;
    }
    // Saturated. `has_more` gets the OTHER mailboxes seen sooner; it does NOT
    // recover the messages the binding stepped over above — see `perBox`.
    if (messages.length >= perBox) mailboxHasMore = true;
    advanced++;
    if (emitted >= limit) break;
  }

  // Seeding is over once every mailbox that exists now has a watermark.
  if (cur.s) {
    var stillBlank = false;
    for (var sk in cur.m) {
      if (Object.prototype.hasOwnProperty.call(cur.m, sk) && !cur.m[sk]) stillBlank = true;
    }
    if (!stillBlank) cur.s = 0;
  }
  cur.r = (start + advanced) % n;
  // `p` counts how far into THIS round the rotation has come, so a mount with
  // more mailboxes than one slice still reports has_more:false once every
  // mailbox has been visited. Without it has_more would be true forever and the
  // engine would page until it hit max_items_per_sync on every single run.
  cur.p = cur.p + advanced;
  var roundDone = cur.p >= n;
  if (roundDone) cur.p = 0;
  return {
    items: items,
    next_token: formatTreeCursor(cur),
    has_more: !roundDone || mailboxHasMore,
  };
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
