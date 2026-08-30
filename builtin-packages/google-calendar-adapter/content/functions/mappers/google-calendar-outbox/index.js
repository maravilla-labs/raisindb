/**
 * Google Calendar OUTBOX mapping function.
 *
 * The write half of a `submit` mount: it turns a raisin:CalendarAction command
 * node into the RSVP the adapter's `submit` issues, and nothing else. Pure and
 * I/O-free, like every mapper — it runs inside the write drain, under the mount
 * lease.
 *
 *   to_external         { node, mount } -> { payload: { action, body },
 *                                            external_id } | null
 *   mapper_capabilities { mount }       -> { to_external: true }
 *   to_node             anything        -> null
 *
 * WHY THE REVERSE TRANSLATION IS HERE AND NOT IN THE ADAPTER
 *
 * The mapper exists so a user can change node shape without forking the
 * adapter. If the adapter built the request itself, the moment someone pointed
 * a mount at a custom mapper the adapter would send the wrong fields —
 * silently. One relationship, two translations, two files: the bug class this
 * codebase pays for most often. So the adapter reads only `action` (routing,
 * not translation) and forwards `body` verbatim.
 *
 * `to_node` returns null on purpose rather than being absent. An outbox is a
 * write-only collection; a mount that also tried to IMPORT from it would
 * materialize answered invitations as commands, and a null is the mapper's
 * documented way to say "skip this item".
 *
 * ONE NODE TYPE, unlike the ms-graph outbox twin, which also carries
 * raisin:OutboundMail. There is no Google mail adapter in this package, so a
 * mail branch here would be a command surface with no `submit` behind it —
 * exactly the shape that makes a mount resolve as capable and then throw at
 * drain time. Anything that is not a raisin:CalendarAction returns null.
 *
 * WHAT THIS MAPPER DELIBERATELY DOES NOT EMIT: a Google `attendees` array.
 *
 * Google has no RSVP endpoint; a response is an events.patch of the caller's
 * own attendee row, and that row can only be identified by reading the event
 * (`self === true`). A mapper is I/O-free by contract, so it cannot know which
 * row is the caller's — and the payload it CAN build ("attendees: [just me]")
 * is the one that deletes every other guest, because events.patch documents
 * that "array fields, if specified, overwrite the existing arrays". The payload
 * therefore stays INTENT-shaped and the adapter's read-modify-write turns it
 * into a request. See adapters/google-calendar/submit.js.
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

var RSVP_ACTIONS = { accept: true, decline: true, tentative: true };

function toExternal(node, mount) {
  if (!node) return null;
  // Only an RSVP. A different node type in a submit mount is a misconfiguration
  // to report, not a shape to guess at — a guess here would send something.
  if (node.node_type && node.node_type !== "raisin:CalendarAction") return null;

  var props = node.properties || {};
  var action = props.action;
  if (!action || !RSVP_ACTIONS[action]) return null;
  // The PROVIDER's event id, never a node id: the adapter reads no nodes, so an
  // RSVP addressed by a local id could not be sent at all.
  if (!props.target_external_id) return null;

  var body = {};
  if (props.comment) body.comment = String(props.comment);
  // Sent EXPLICITLY, never left to the provider's default, and defaulted to
  // true here rather than in the adapter: notifying the organizer is the
  // externally-visible half of an RSVP — for Google it IS the RSVP, since
  // `sendUpdates=none` records the response and tells nobody — so which way it
  // goes must be decided where a person can read it. Same key and same default
  // as the ms-graph outbox, so a command node moves between providers unchanged.
  body.send_response = props.send_response !== false;

  return {
    payload: { action: action, body: body },
    external_id: String(props.target_external_id),
  };
}
