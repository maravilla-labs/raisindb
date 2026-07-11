/**
 * Integration webhook-refresh handler.
 *
 * A provider webhook (a raisin:Trigger with a nanoid webhook_id) points at this
 * function. When the external provider notifies RaisinDB that a mounted resource
 * changed, this handler resolves the target mount id and enqueues a deduped
 * "sync now" (a VirtualMountSync job) so the change is pulled on demand instead
 * of waiting for the next scheduled sweep.
 *
 * The mount id is looked up, in order, from:
 *   - the webhook query string   (?mount_id=... -> input.http.query.mount_id)
 *   - the webhook body           (input.http.body.mount_id)
 *   - the resolved path params   (input.http.params.mount_id)
 *   - a top-level input.mount_id (async trigger jobs / manual invocation)
 *
 * Entrypoint: handler(input) — exactly one argument.
 */

function pick(obj, key) {
  if (obj && typeof obj === "object" && obj[key] != null) {
    return obj[key];
  }
  return undefined;
}

function resolveMountId(input) {
  var http = pick(input, "http") || {};
  return (
    pick(http.query, "mount_id") ||
    pick(http.body, "mount_id") ||
    pick(http.params, "mount_id") ||
    pick(input, "mount_id")
  );
}

function handler(input) {
  input = input || {};

  var mountId = resolveMountId(input);
  if (!mountId) {
    // Nothing to sync — surface a clear, non-throwing result so the webhook
    // still returns 200 to the provider (avoids retry storms).
    return {
      ok: false,
      reason: "no mount_id in webhook query, body, params, or input",
    };
  }

  // Optional explicit mode ("delta" | "full"); defaults to delta downstream.
  var http = pick(input, "http") || {};
  var mode =
    pick(http.query, "mode") || pick(http.body, "mode") || pick(input, "mode");

  var result = raisin.integrations.sync_now(mountId, mode);

  return {
    ok: true,
    mount_id: mountId,
    job_id: result.job_id,
    status: result.status,
  };
}
