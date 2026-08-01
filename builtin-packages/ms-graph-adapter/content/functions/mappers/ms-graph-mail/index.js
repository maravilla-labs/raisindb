/**
 * Microsoft 365 mail mapping function.
 *
 * Called once per external item by the sync engine (adapter contract §6). Pure
 * and fast: it must NOT call raisin.functions.call or perform any I/O — it runs
 * in the sync hot loop. Returning null skips the item.
 *
 *   input  = { external_item: ExternalItem, mount: { mount_id, mount_path, sync_config } }
 *   return = { node_type, name?, properties } | null
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
