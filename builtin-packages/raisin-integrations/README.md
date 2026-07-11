# Integration Support

Provider-agnostic support functions for virtual mounts (connectors).

## `webhook-refresh`

`/lib/raisin/integrations/webhook-refresh` is the webhook target for a mount.
Point a `raisin:Trigger` (with a generated `webhook_id`) at this function and
configure the external provider to POST to that webhook URL whenever a mounted
resource changes.

On each call the handler resolves the target mount id — from the query string
(`?mount_id=…`), the request body (`{ "mount_id": … }`), the resolved path
params, or a top-level `mount_id` — and calls
`raisin.integrations.sync_now(mountId, mode?)`. That enqueues a deduped
`VirtualMountSync` job, so a burst of provider notifications collapses into a
single in-flight sync. `mode` defaults to `"delta"`; pass `"full"` for a full
re-sync.

Response: `{ ok, mount_id, job_id, status }` where `status` is `"queued"` or
`"already_running"` (`job_id` is `null` when an in-flight sync deduped the
request). When no mount id can be resolved it returns
`{ ok: false, reason }` without throwing, so the webhook still answers `200` and
the provider does not retry-storm.
