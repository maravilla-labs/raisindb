/**
 * Microsoft 365 calendar mapping function.
 *
 * Called once per external item by the sync engine (adapter contract §6). Pure
 * and fast: it must NOT call raisin.functions.call or perform any I/O — it runs
 * in the sync hot loop. Returning null skips the item.
 *
 *   input  = { external_item: ExternalItem, mount: { mount_id, mount_path, sync_config } }
 *   return = { node_type, name?, properties } | null
 *
 * Events map to raisin:Event, hoisting the adapter's metadata into the typed
 * raisin:Event properties (start/end/location/attendees/organizer/recurrence/
 * status/url). title = subject.
 *
 * raisin:Event is a STRICT node type: only its declared properties (plus the
 * engine-owned __ reserved ones) may be written, so NO free-form provider_metadata
 * blob is emitted here — every value maps onto a declared column.
 *
 * `name` stays the Graph item id (external_item.name) so distinct events never
 * collide on a path; the human-readable subject lives in the `title` property.
 */

function handler(input) {
  var item = input.external_item;
  if (!item || !item.external_id) return null;

  var meta = item.metadata || {};
  var subject = meta.subject || "(untitled event)";

  return {
    node_type: "raisin:Event",
    // id, not subject — path stability is owned by the adapter's external_id.
    name: item.name,
    properties: {
      title: subject,
      start: meta.start || null,
      end: meta.end || null,
      all_day: meta.all_day === true,
      location: meta.location || null,
      attendees: meta.attendees || [],
      organizer: meta.organizer || null,
      recurrence: meta.recurrence || null,
      status: meta.status || null,
      url: item.web_url || meta.webLink || null,
      calendar_id: (input.mount && input.mount.remote_root) || null,
    },
  };
}
