/**
 * Microsoft 365 mail mapping function.
 *
 * Called once per external item by the sync engine (adapter contract §6). Pure
 * and fast: it must NOT call raisin.functions.call or perform any I/O — it runs
 * in the sync hot loop. Returning null skips the item.
 *
 * Bidirectional (adapter contract §6.0): it dispatches on input.operation and
 * both directions live here, so node shape and its inverse have one author.
 *
 *   to_node             { external_item, mount }  -> { node_type, name?, properties } | null
 *   to_external         { node, mount, fields? }  -> { payload, external_id? } | null
 *   mapper_capabilities { mount }                 -> { to_external: true }
 *
 * An absent operation means to_node, so the engine's read path is unchanged.
 *
 * Messages map to `raisin:Mail`, a global nodetype shared with the IMAP/Gmail
 * connectors. It declares the indexing that makes mail queryable: Fulltext on
 * subject/body/snippet, Property on from_address, conversation_id, message_id,
 * date, unread, folder — so `GROUP BY conversation_id` and sender filters are
 * index-backed rather than full scans.
 *
 * Not raisin:Asset: a message is metadata plus a body that is only inlined when
 * the mount sets `sync_config.include_body`. Without it the body is absent here
 * and remains available on demand through the adapter's get_content.
 *
 * `name` stays the Graph item id (external_item.name) so distinct messages never
 * collide on a path; the human-readable subject lives in the `subject` property.
 *
 * The engine stamps the reserved __virtual/__mount_id/__external_id/__etag/
 * __synced_at properties on top of whatever is returned, so they are not set here.
 */

function handler(input) {
  switch (input && input.operation) {
    case "to_external":
      return toExternal(input.node, input.mount, input.fields);
    // Probed once per sync run. Without it the mount is reported read-only,
    // which is what every mapper that has not been taught to_external wants.
    case "mapper_capabilities":
      return { to_external: true };
    // Absent operation === "to_node": the engine sent "to_node" long before any
    // mapper switched on it, and a mapper must keep working either way.
    case "to_node":
    default:
      return toNode(input);
  }
}

function toNode(input) {
  var item = input.external_item;
  if (!item || !item.external_id) return null;

  var meta = item.metadata || {};
  var mount = input.mount || {};
  var sync = mount.sync_config || {};

  var props = {
    subject: meta.subject || "(no subject)",
    from: meta.from || null,
    from_address: meta.from_address || null,
    to: meta.to || null,
    cc: meta.cc || null,
    date: meta.date || item.modified_at || null,
    snippet: meta.snippet || null,
    unread: meta.unread === true,
    has_attachments: meta.has_attachments === true,
    importance: meta.importance || null,
    conversation_id: meta.conversation_id || null,
    // RFC 5322 Message-ID — stable across folders and providers, so it is what
    // identifies the same mail seen through both an Inbox and a Sent Items
    // mount. The Graph item id is per-folder and cannot do that.
    message_id: meta.internet_message_id || null,
    // Which mailbox this copy came from, so one query can span several mail
    // mounts and still tell them apart. Mirrors the adapter's remote_root
    // default of "inbox".
    folder: mount.remote_root || sync.remote_root || "inbox",
    web_url: item.web_url || meta.web_url || null,
    provider: "ms-graph",
    mime_type: "message/rfc822",
    // Provider-specific passthrough is preserved verbatim.
    provider_metadata: meta,
  };

  // Only set when the mount opted in and the adapter actually returned one.
  // Writing "" for an absent body would blank a previously synced body and,
  // worse, change the node on every run — defeating the etag skip-write that
  // stops a re-sync from re-firing every downstream trigger.
  if (typeof meta.body === "string") {
    props.body = meta.body;
    props.body_type = meta.body_type || "text";
  }

  return {
    node_type: "raisin:Mail",
    // id, not subject — path stability is owned by the adapter's external_id.
    name: item.name,
    properties: props,
  };
}

/**
 * A mail message is immutable content with mutable STATE, so this mount is
 * `state_only`: only the properties named below ever push, and `fields` (when
 * the engine supplies it) narrows that further to the ones that actually
 * changed. Anything outside the allow-list is dropped rather than guessed at —
 * sending a whole message object where a patch was meant is how a sync
 * overwrites a body it was never asked to touch.
 *
 * `unread` inverts: Graph's property is `isRead`.
 */
var WRITABLE_FIELDS = ["unread"];

function toExternal(node, mount, fields) {
  if (!node) return null;
  var props = node.properties || {};
  var wanted = fields && fields.length ? fields : WRITABLE_FIELDS;

  var payload = {};
  var emitted = 0;
  for (var i = 0; i < wanted.length; i++) {
    var field = wanted[i];
    if (WRITABLE_FIELDS.indexOf(field) === -1) continue;
    if (field === "unread") {
      // Absent is not the same as false: a node that never carried the property
      // must not be pushed as "read".
      if (props.unread === undefined || props.unread === null) continue;
      payload.isRead = props.unread !== true;
      emitted++;
    }
  }

  // Nothing writable in this request: say "not writable" rather than issuing an
  // empty PATCH that touches the message's change key for no reason.
  if (!emitted) return null;

  var out = { payload: payload };
  if (props.__external_id) out.external_id = props.__external_id;
  return out;
}
