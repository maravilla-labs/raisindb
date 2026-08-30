/**
 * IMAP OUTBOX mapping function.
 *
 * The write half of a `submit` mount: it turns a raisin:OutboundMail command
 * node into the message the adapter hands to `raisin.email.send`, and nothing
 * else. Pure and I/O-free, like every mapper — it runs inside the write drain,
 * under the mount lease.
 *
 *   to_external         { node, mount } -> { payload: { action, body } } | null
 *   mapper_capabilities { mount }       -> { to_external: true }
 *   to_node             anything        -> null
 *
 * WHY AN IMAP MOUNT CAN SEND AT ALL
 *
 * It cannot: IMAP is a READ protocol with no submission verb. The send goes out
 * through the TENANT's configured email provider (`/config/email`), which the
 * adapter reaches with `raisin.email.send`. Two consequences a reader must not
 * have to discover from a recipient's inbox:
 *
 *   * The message is NOT sent from the mailbox this mount syncs. `from`,
 *     `from_name` and `reply_to` come from the tenant's provider entry — a
 *     function never chooses who it appears to be from. Point the mount's
 *     `email_provider` at an entry whose `from_address` is the mailbox, or the
 *     reply lands somewhere the sender never sees.
 *   * There is no Sent copy. IMAP APPEND would write one and the native
 *     `raisin.imap` binding has no APPEND today (fetchSince / listMailboxes /
 *     fetchMessage are the whole surface), so the sent message exists only at
 *     the provider — and, once the mount's mailbox is the same account, in
 *     whatever copy that provider files itself.
 *
 * WHY THE REVERSE TRANSLATION IS HERE AND NOT IN THE ADAPTER
 *
 * The mapper exists so a user can change node shape without forking the
 * adapter. If the adapter built the message itself, the moment someone pointed
 * a mount at a custom mapper the adapter would send the wrong fields —
 * silently. One relationship, two translations, two files: the bug class this
 * codebase pays for most often. So the adapter forwards `body` verbatim and
 * reads only `action`, which is routing rather than translation.
 *
 * The one thing the adapter DOES add to the body is `provider`. That is a
 * connection setting — which of the tenant's configured senders this mount
 * posts through — and it belongs with the mount's other connection settings for
 * the same reason the IMAP host does, not on every command node.
 *
 * `to_node` returns null on purpose rather than being absent. An outbox is a
 * write-only collection; a mount that also tried to IMPORT from it would
 * materialize sent mail as fresh commands to send, and a null is the mapper's
 * documented way to say "skip this item".
 */

function handler(input) {
  switch (input && input.operation) {
    case "to_external":
      return toExternal(input.node);
    case "mapper_capabilities":
      return { to_external: true };
    default:
      // Includes "to_node" and an absent operation. An outbox imports nothing.
      return null;
  }
}

/**
 * Recipients as `raisin.email.send` wants them: an array of bare addresses.
 *
 * Tolerates a single string and a `{ address | email, name }` object, because a
 * compose UI naturally produces one and an import naturally produces the other.
 * An entry with no address is DROPPED rather than sent as an empty recipient,
 * which the engine's own validation refuses for the whole message.
 *
 * The display name is discarded, unlike the Graph mapper: the email API takes
 * addresses only, and "Ada <ada@example.org>" in an address slot fails the
 * `contains('@')` check as a whole string on some providers and arrives as a
 * malformed header on others.
 */
function recipients(list) {
  var input = Array.isArray(list) ? list : list ? [list] : [];
  var out = [];
  for (var i = 0; i < input.length; i++) {
    var entry = input[i];
    var address = typeof entry === "string" ? entry : entry && (entry.address || entry.email);
    if (!address) continue;
    address = String(address).trim();
    if (!address) continue;
    out.push(address);
  }
  return out.length ? out : null;
}

/**
 * A plain-text rendering of an HTML body.
 *
 * NOT a nicety. `raisin.email.send` requires a non-empty `text` and refuses an
 * HTML-only message ([email:invalid_message] "empty text body"), so a command
 * composed in a rich editor — which is every command a compose UI produces —
 * would fail at drain time with a message about a field the author never saw.
 * A text alternative is also what keeps the mail out of a spam filter.
 *
 * Deliberately crude: block tags become newlines, everything else is stripped,
 * and the five XML entities are decoded. Anything cleverer is an HTML parser
 * living in a mapper that must stay pure and fast.
 */
function htmlToText(html) {
  if (!html) return "";
  return String(html)
    .replace(/<(script|style)[\s\S]*?<\/\1>/gi, "")
    .replace(/<br\s*\/?>/gi, "\n")
    .replace(/<\/(p|div|tr|li|h[1-6])>/gi, "\n")
    .replace(/<[^>]*>/g, "")
    .replace(/&nbsp;/gi, " ")
    .replace(/&lt;/gi, "<")
    .replace(/&gt;/gi, ">")
    .replace(/&quot;/gi, '"')
    .replace(/&#39;/gi, "'")
    .replace(/&amp;/gi, "&")
    .replace(/\n{3,}/g, "\n\n")
    .trim();
}

function toExternal(node) {
  if (!node) return null;
  var props = node.properties || {};
  var action = props.action;
  if (!action) return null;

  // Only `send` is actually sendable. reply / reply_all / forward are refused
  // rather than approximated: threading them correctly needs In-Reply-To and
  // References headers on the outgoing message, and `raisin.email.send` accepts
  // { to, cc, bcc, subject, text, html, attachments, provider } and no headers
  // at all. A "reply" that is really a fresh message with a Re: subject breaks
  // the recipient's thread silently.
  //
  // The refusal is the ADAPTER's, not a null here. `map_command` runs before the
  // adapter is ever called, and a null makes the engine settle the command with
  // its OWN generic text — "the mount's mapping function declined this command
  // (to_external returned null) — either it is not finished being authored, or
  // it has already been sent and must not be sent again" — so an author who
  // queued a reply was told their command was unfinished or a duplicate, both
  // wrong. Passing the action through reaches `opSubmit`, which throws a
  // `config_error` naming the real reason (the missing headers) and which the
  // engine settles terminally with the adapter's own text. Cost: the command is
  // claimed (queued -> sending) before being refused — the same outcome, with a
  // reason the author can act on.
  //
  // No body is built for these: `opSubmit` rejects on `action` before it looks
  // at `body`, and a half-built message would only invite the guess this refusal
  // exists to avoid.
  if (action !== "send") {
    return { payload: { action: String(action), body: null } };
  }

  var to = recipients(props.to);
  if (!to) return null; // nowhere to send it

  // Both required and both non-empty at the API. Declining here rather than
  // letting the send refuse them keeps the reason local ("this command is not
  // sendable") instead of arriving as a provider validation error.
  var subject = props.subject ? String(props.subject) : "";
  if (!subject) return null;

  var html = props.body_html ? String(props.body_html) : null;
  var text = props.body_text ? String(props.body_text) : htmlToText(html);
  if (!text) return null;

  var message = { to: to, subject: subject, text: text };
  if (html) message.html = html;
  var cc = recipients(props.cc);
  if (cc) message.cc = cc;
  var bcc = recipients(props.bcc);
  if (bcc) message.bcc = bcc;

  // NOT carried: `importance`, and attachments. `importance` has no field in
  // the email API's message shape, and an attachment must name a source the
  // server can read (base64, or a node + property) — the raisin:Asset children
  // an inbound IMAP message maps to carry no `file`, by design, so there is
  // nothing to point at yet. Both are silently absent rather than half-sent.
  return { payload: { action: "send", body: message } };
}
