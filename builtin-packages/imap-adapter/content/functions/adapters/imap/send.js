import { coded } from "./common.js";
import { mountSetting } from "./mount.js";

/**
 * The SEND path: one queued outbound-mail command becomes one
 * `raisin.email.send`. Its own module because it is its own TRANSPORT — nothing
 * here speaks IMAP, and its error taxonomy is the email API's tags, which share
 * no vocabulary with the binding's `[imap:...]` ones.
 */
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
export function resolveSender(mount) {
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
export function opSubmit(mount, params) {
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
