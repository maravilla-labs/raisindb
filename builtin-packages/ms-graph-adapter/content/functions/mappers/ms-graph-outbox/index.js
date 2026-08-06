/**
 * Microsoft 365 OUTBOX mapping function.
 *
 * The write half of a `submit` mount: it turns a command node into the Graph
 * request the adapter issues, and nothing else. Pure and I/O-free, like every
 * mapper — it runs inside the write drain, under the mount lease.
 *
 *   to_external         { node, mount } -> { payload: { action, body },
 *                                            external_id? } | null
 *   mapper_capabilities { mount }       -> { to_external: true }
 *   to_node             anything        -> null
 *
 * WHY THE REVERSE TRANSLATION IS HERE AND NOT IN THE ADAPTER
 *
 * The mapper exists so a user can change node shape without forking the
 * adapter. If the adapter built the Graph message itself, the moment someone
 * pointed a mount at a custom mapper the adapter would send the wrong fields —
 * silently. One relationship, two translations, two files: the bug class this
 * codebase pays for most often. So the adapter forwards `body` verbatim and
 * reads only `action`, which is routing rather than translation.
 *
 * `to_node` returns null on purpose rather than being absent. An outbox is a
 * write-only collection; a mount that also tried to IMPORT from it would
 * materialize sent mail as commands, and a null is the mapper's documented way
 * to say "skip this item".
 *
 * TWO NODE TYPES, ONE MAPPER, because they are one protocol. raisin:OutboundMail
 * and raisin:CalendarAction share a status lifecycle, an at-most-once claim and
 * a terminal state; splitting them across two mappers would be two copies of the
 * same lifecycle assumptions, and the second copy is always the one that drifts.
 */

function handler(input) {
  switch (input && input.operation) {
    case "to_external":
      return toExternal(input.node, input.mount);
    case "mapper_capabilities":
      return { to_external: true };
    default:
      // Includes "to_node" and an absent operation. An outbox imports nothing.
      return null;
  }
}

/** Recipients as Graph wants them, from an array of addresses. */
function recipients(list) {
  if (!Array.isArray(list)) return null;
  var out = [];
  for (var i = 0; i < list.length; i++) {
    var entry = list[i];
    // Tolerates both a bare address and a { address, name } object, because a
    // compose UI naturally produces one and an import naturally produces the
    // other. An entry with neither is DROPPED rather than sent as an empty
    // recipient, which Graph rejects for the whole message.
    var address = typeof entry === "string" ? entry : entry && (entry.address || entry.email);
    if (!address) continue;
    var recip = { emailAddress: { address: String(address) } };
    if (entry && entry.name) recip.emailAddress.name = String(entry.name);
    out.push(recip);
  }
  return out.length ? out : null;
}

/**
 * The message body.
 *
 * HTML wins when both are present: it is the richer representation, and Graph
 * takes ONE contentType. Sending the text half of a message someone composed in
 * HTML is a silent downgrade nobody sees until the recipient does.
 */
function body(props) {
  if (props.body_html) return { contentType: "HTML", content: String(props.body_html) };
  if (props.body_text) return { contentType: "Text", content: String(props.body_text) };
  return null;
}

function mailMessage(props) {
  var message = {};
  var b = body(props);
  if (b) message.body = b;
  if (props.subject) message.subject = String(props.subject);
  var to = recipients(props.to);
  if (to) message.toRecipients = to;
  var cc = recipients(props.cc);
  if (cc) message.ccRecipients = cc;
  var bcc = recipients(props.bcc);
  if (bcc) message.bccRecipients = bcc;
  if (props.importance) message.importance = String(props.importance);
  return message;
}

function toExternal(node, mount) {
  if (!node) return null;
  var props = node.properties || {};
  var action = props.action;
  if (!action) return null;

  // ---- RSVP ---------------------------------------------------------------
  if (action === "accept" || action === "decline" || action === "tentative") {
    if (!props.target_external_id) return null;
    var rsvp = {};
    if (props.comment) rsvp.comment = String(props.comment);
    // Sent EXPLICITLY, never left to the provider's default, and defaulted to
    // true here rather than in the adapter: notifying the organizer is the
    // externally-visible half of an RSVP, so which way it goes must be decided
    // where a person can read it, not inherited from a Graph default that could
    // change.
    rsvp.sendResponse = props.send_response !== false;
    return {
      payload: { action: action, body: rsvp },
      external_id: String(props.target_external_id),
    };
  }

  // ---- mail ---------------------------------------------------------------
  var message = mailMessage(props);

  if (action === "send") {
    if (!message.toRecipients) return null; // nowhere to send it
    var send = { message: message };
    // Default TRUE, matching Graph's own default, and stated rather than
    // inherited: a sent message that is not filed is a message the user cannot
    // find afterwards, and a mount whose `/mail/sent` is read-only would then
    // show a gap it can never explain.
    send.saveToSentItems = mount && mount.config && mount.config.save_to_sent === false
      ? false
      : true;
    return { payload: { action: "send", body: send } };
  }

  if (action === "reply" || action === "reply_all" || action === "forward") {
    if (!props.in_reply_to_external_id) return null;
    // A forward with no recipients cannot be issued; a reply inherits them from
    // the message being answered, which is the entire reason the provider's own
    // reply action is used instead of composing one here.
    if (action === "forward" && !message.toRecipients) return null;
    return {
      payload: { action: action, body: { message: message } },
      external_id: String(props.in_reply_to_external_id),
    };
  }

  // An action this mapper does not know. Null, not a guess: the engine records
  // the command as `failed` with a stated reason, which is the outcome an
  // author can act on. A guess would send something.
  return null;
}
