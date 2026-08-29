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
 * `raisin.email.send` — see the send-path section below for what that means for
 * the From address and for the Sent folder.
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

// Resolve the effective connection settings.
//
// `mount.config` is the engine's pre-merged view — api_config < connector
// config < CONNECTION config < sync_config — and is what carries per-connection
// settings, so one connector can serve several mailboxes on different servers.
// It is preferred; api_config and sync_config remain as fallbacks so this
// adapter keeps working against an older engine that sends neither.
//
// api_config names the mailbox `default_mailbox`; everywhere else it is `mailbox`.
function mountSetting(mount, key) {
  var api = (mount && mount.api_config) || {};
  var sync = (mount && mount.sync_config) || {};
  var merged = (mount && mount.config) || {};
  if (merged[key] !== undefined) return merged[key];
  return sync[key] !== undefined ? sync[key] : api[key];
}

function connConfig(mount) {
  var api = (mount && mount.api_config) || {};
  function pick(key) {
    return mountSetting(mount, key);
  }
  var mailbox = pick("mailbox");
  return {
    host: pick("host"),
    port: pick("port"),
    tls: pick("tls"),
    auth: pick("auth"),
    mailbox: mailbox !== undefined ? mailbox : api.default_mailbox,
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

// ---- the SEND path (raisin.email) -----------------------------------------
//
// IMAP CANNOT SEND. RFC 3501 has no submission verb; APPEND writes a copy into
// a mailbox and delivers nothing. So a `submit` mount here does not send over
// the mount's own connection at all — it hands the message to the TENANT's
// configured email provider through `raisin.email.send`, which is the same
// SMTP/relay path every other server-side function uses.
//
// Two consequences, stated here because a reader must not discover them from a
// recipient's inbox:
//
//   * The mail is NOT sent from the mailbox this mount syncs. `from`,
//     `from_name` and `reply_to` come from the tenant's `/config/email` entry —
//     a function chooses WHICH configured sender to post through, never who it
//     appears to be from. Configure an entry whose `from_address` is this
//     mailbox and name it with `sync_config.email_provider`, or replies land
//     somewhere the sender never looks.
//   * No Sent copy is filed. That would be an IMAP APPEND, and the native
//     `raisin.imap` binding has no APPEND (fetchSince / listMailboxes /
//     fetchMessage is the whole surface). See the README's follow-up section.
//
// The IMAP `credential` is deliberately unused on this path: the mailbox
// password has nothing to do with the provider credential, which the email API
// resolves itself from `/config/email` under this function's secret_policy.

// Which of the tenant's configured senders this mount posts through. Absent
// means "the tenant default", which is only usable when the tenant HAS an
// unambiguous default — see resolveSender.
function emailProviderName(mount) {
  var name = mountSetting(mount, "email_provider");
  if (name === undefined || name === null) return null;
  name = String(name).trim();
  return name || null;
}

// `EmailConfig::resolve` ends with `sender.validate()`, and the first thing
// that checks is a non-empty `from_address` — an entry without one is a
// [email:config] refusal at SEND time, not at selection time. The listing
// carries `from_address`, so this is checkable during the probe, and a
// capability that resolves and then throws is the failure this file exists to
// avoid. (`validate` also demands smtp settings for an smtp entry; the listing
// does NOT carry those, so that one case can still only be found at drain time
// — as a terminal `failed` naming it.)
function hasFromAddress(p) {
  return !!(p && p.from_address && String(p.from_address).trim());
}

function noFromAddressReason(p) {
  return (
    "email provider `" +
    ((p && p.name) || "?") +
    "` has no from_address, so a send through it would be refused; set one on /config/email"
  );
}

// Can this mount actually send, and through which sender?
//
//   -> { ok: true, name: string|null }   name null = omit `provider`, use the
//                                        tenant default
//   -> { ok: false, reason: string }
//
// This deliberately RE-IMPLEMENTS the selection rules of `EmailConfig::resolve`
// (crates/raisin-functions/src/runtime/email/mod.rs) against the listing that
// `raisin.email.providers()` returns, instead of just checking that some
// provider exists. The reason is the whole point of this function: `can_submit`
// must be false unless a sender is genuinely resolvable, and the ambiguous
// cases — several enabled entries with no default, an entry that is disabled,
// email switched off for the tenant — all look like "a provider is configured"
// from a distance and then throw at DRAIN time, after the engine has already
// claimed the command. A capability declared with nothing behind it is the
// failure this codebase cares most about.
//
// The listing's `default` flag already folds in `default_provider`, so the
// two spellings cannot disagree here either.
function resolveSender(mount) {
  var wanted = emailProviderName(mount);
  var listing;
  try {
    listing = raisin.email.providers();
  } catch (e) {
    // Denied by the function's own email_policy (the default: deny), or the
    // tenant has no /config/email node at all. Both are operator edits, and
    // the API's message already names what would grant it.
    return {
      ok: false,
      reason:
        "raisin.email.providers() refused: " +
        ((e && e.message) || String(e)) +
        ". An IMAP outbox sends through the tenant's configured email provider, so the " +
        "adapter function needs email_policy.enabled with an allowed_recipients list, and " +
        "secret_policy access to the provider's credential.",
    };
  }
  if (!listing || listing.enabled !== true) {
    return {
      ok: false,
      reason:
        "outbound email is switched off for this tenant (set enabled: true on the " +
        "raisin:EmailConfig node at /config/email, or use the console's Email page)",
    };
  }
  var all = listing.providers || [];
  var names = all
    .map(function (p) {
      return p && p.name;
    })
    .join(", ");

  if (wanted) {
    var match = null;
    for (var i = 0; i < all.length; i++) {
      if (all[i] && String(all[i].name).toLowerCase() === wanted.toLowerCase()) match = all[i];
    }
    if (!match) {
      return {
        ok: false,
        reason:
          "sync_config.email_provider names `" +
          wanted +
          "`, which this tenant has not configured (configured providers: " +
          (names || "none") +
          ")",
      };
    }
    if (match.enabled === false) {
      return { ok: false, reason: "email provider `" + wanted + "` is disabled" };
    }
    if (!hasFromAddress(match)) return { ok: false, reason: noFromAddressReason(match) };
    return { ok: true, name: match.name };
  }

  var enabled = all.filter(function (p) {
    return p && p.enabled !== false;
  });
  if (!enabled.length) {
    return {
      ok: false,
      reason:
        "this tenant has no enabled email provider" +
        (names ? " (configured: " + names + ", all disabled)" : ""),
    };
  }
  var defaults = enabled.filter(function (p) {
    return p.default === true;
  });
  if (defaults.length === 1 || (defaults.length === 0 && enabled.length === 1)) {
    var chosen = defaults.length === 1 ? defaults[0] : enabled[0];
    if (!hasFromAddress(chosen)) return { ok: false, reason: noFromAddressReason(chosen) };
    // `name: null` — the message omits `provider` and the email API picks the
    // same entry by the same rules. Naming it here would freeze today's default
    // into every send.
    return { ok: true, name: null };
  }
  if (defaults.length > 1) {
    return {
      ok: false,
      reason: "several email providers are marked default; exactly one may be (" + names + ")",
    };
  }
  return {
    ok: false,
    reason:
      "several email providers are enabled and none is the default, so a send would be " +
      "ambiguous: set default_provider on /config/email, or name one on this mount with " +
      "sync_config.email_provider (configured: " +
      names +
      ")",
  };
}

// Machine tags the email API puts at the front of its error messages, split by
// what they mean for a COMMAND that may already have left.
//
// Everything listed here is refused BEFORE a socket is opened, or by the
// provider before it looks at the message: the recipient policy, the config,
// local validation, an unimplemented provider, and a credential the provider
// rejected. None of them can have delivered anything, so they are terminal
// `failed` (code "config_error") and a person edits and requeues.
var TERMINAL_EMAIL_TAGS = [
  "[email:policy_denied]",
  "[email:config]",
  "[email:invalid_message]",
  "[email:unsupported]",
  "[email:auth_failed]",
  // NOT an [email:...] tag, and the one refusal this adapter cannot pre-check.
  // Resolving the chosen sender's `credential_ref` runs the FUNCTION's secret
  // policy (api/raisindb/email.rs `authorize_email` -> `impl_secret_resolve`),
  // and `providers()` deliberately carries no credential_ref — so a provider
  // whose credential sits outside `secret_policy.allowed_names` passes the
  // capabilities probe and then refuses every send. It refuses before the
  // socket, so nothing left; without this entry the message carries no word the
  // engine's classifier recognizes, it becomes Transient, and the submit drain
  // parks the command at `unknown` — sending a person to search a Sent folder
  // for a mail that was never attempted, once per command, forever.
  "[secrets:",
];

// Everything NOT listed above — [email:transport], [email:timeout],
// [email:provider_error] — is re-thrown as a PLAIN Error, which the engine
// classifies Transient and the submit drain parks at `unknown`: never resent,
// attributable by attempt_id. That is the correct default for a send. An SMTP
// timeout most often means the message reached the relay and the acknowledgement
// was lost; resending delivers a second copy to a real person, irreversibly.
//
// [email:rate_limited] is the one exception, and for the same reason the engine
// documents: a 429 is the provider refusing to look at the request, which is the
// only answer that proves nothing was sent.
function mapEmailError(e) {
  var m = (e && e.message) || String(e);
  for (var i = 0; i < TERMINAL_EMAIL_TAGS.length; i++) {
    if (m.indexOf(TERMINAL_EMAIL_TAGS[i]) !== -1) return coded(m, "config_error");
  }
  if (m.indexOf("[email:rate_limited]") !== -1) return coded(m, "rate_limited");
  return new Error(
    "submit: the send failed with " +
      m +
      ". Whether the message left is UNKNOWN — a relay timeout often means it did. " +
      "Not retrying; check the provider before requeueing this command."
  );
}

// submit: one queued raisin:OutboundMail command becomes ONE raisin.email.send.
//
// `params` is what the engine's submit drain sends:
//   { payload: { action, body }, external_id?, idempotency_key }
//
// `body` is the message, forwarded VERBATIM but for `provider`: the MAPPER is
// the only authorized translator between node shape and message shape, and an
// adapter that rebuilt it here would silently disagree with any custom mapper
// pointed at the same mount. `provider` is not translation — it is which of the
// tenant's connections this mount posts through, and it lives with the mount's
// other connection settings.
//
// `idempotency_key` is accepted and NOT sent: neither SMTP nor the email API has
// anywhere to put one. `capabilities.supports_idempotency_key` says so, and the
// engine's at-most-once guarantee rests on its own durable claim.
function opSubmit(mount, params) {
  params = params || {};
  var payload = params.payload || {};
  var action = payload.action;
  if (!action) {
    throw coded("submit: params.payload.action is required", "config_error");
  }
  if (action !== "send") {
    throw coded(
      "submit: the IMAP connector can only issue 'send' (got '" +
        action +
        "'). reply, reply_all and forward need In-Reply-To/References headers on the " +
        "outgoing message, and raisin.email.send accepts no headers — a fresh message " +
        "with a Re: subject would break the recipient's thread silently.",
      "config_error"
    );
  }
  var body = payload.body;
  if (!body || typeof body !== "object" || !body.to) {
    throw coded("submit: refusing to issue a command with no message body", "config_error");
  }

  // Resolved per command rather than cached from the capabilities probe: the
  // operator may have disabled the provider since the run started, and the
  // cheap answer to that is a terminal `failed` naming it, not a send through
  // an account that is no longer meant to be used.
  var sender = resolveSender(mount);
  if (!sender.ok) {
    throw coded("submit: " + sender.reason, "config_error");
  }

  var message = {};
  for (var k in body) {
    if (Object.prototype.hasOwnProperty.call(body, k)) message[k] = body[k];
  }
  // Only when a name was resolved. Omitting the key is how the email API is
  // told "the tenant default"; sending null would be read as a provider name.
  if (sender.name) message.provider = sender.name;

  var receipt;
  try {
    receipt = raisin.email.send(message);
  } catch (e) {
    throw mapEmailError(e);
  }

  // An OBJECT must come back: the engine reads a non-object answer as "the
  // outcome is unknown" and parks the command. `message_id` is the provider's
  // receipt — acceptance, not delivery — and is what the engine stores as the
  // command's external id; a provider that returns none leaves the engine to
  // derive one from the node.
  return {
    external_id: receipt && receipt.message_id ? String(receipt.message_id) : null,
    etag: null,
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
