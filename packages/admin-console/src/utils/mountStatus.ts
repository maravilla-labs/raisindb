// SPDX-License-Identifier: BSL-1.1

/**
 * Presentation helpers shared by the mounts list and the mount detail view.
 *
 * These were private to `pages/Mounts.tsx`. They are lifted here because the
 * list row and the detail header must always agree: a row that says "OK" while
 * the page it links to says "Degraded" is worse than either alone, and two
 * copies of a status ladder drift the moment one of them gains a state.
 */

import {
  AlertTriangle,
  CheckCircle,
  CircleOff,
  HardDrive,
  Loader2,
  PauseCircle,
  ShieldAlert,
  StopCircle,
  type LucideIcon,
} from 'lucide-react'
import type { MountState, SyncRun, WriteConfig } from '../api/integrations'

/**
 * What a mount's `write_config` actually asks for, in one place.
 *
 * `writeback` and `mode` are INDEPENDENT keys, and the console once read only
 * the legacy `writeback` — so a working `mode: state_only` mount displayed as
 * "off" in the list and on the detail page, and had its nodes badged read-only
 * in the content tree. The same omission returned when `mirror` and `submit`
 * became real: they must be named here, or the modes with the largest blast
 * radius are the ones that look inert. One helper, three call sites, no drift.
 *
 * `writeback: "write_through"` is the deprecated spelling of `mode: "mirror"`
 * and the engine resolves it as one (`wants_mirror`), so it is labelled as one.
 */
export function writeModeLabel(write?: WriteConfig): string {
  if (write?.mode === 'state_only') {
    const n = write.mutable_fields?.length ?? 0
    return `state_only (${n} field${n === 1 ? '' : 's'})`
  }
  if (write?.mode === 'mirror' || write?.writeback === 'write_through') {
    // The delete policy, not the field count, is the number an operator scanning
    // a list needs: it is the one that says whether a local delete reaches the
    // provider. `detach` — the safe default — pushes nothing.
    return `mirror (delete: ${write?.delete_policy || 'connector default'})`
  }
  if (write?.mode === 'submit') return 'submit (outbox)'
  return write?.writeback || 'off'
}

/** True when a mount asks for ANY write mode the engine implements. */
export function mountIsWritable(mount?: { write_config?: WriteConfig }): boolean {
  const w = mount?.write_config
  return (
    w?.mode === 'state_only' ||
    w?.mode === 'mirror' ||
    w?.mode === 'submit' ||
    w?.writeback === 'write_through'
  )
}

export type StatusKind =
  | 'ok'
  | 'syncing'
  | 'stopping'
  | 'interrupted'
  | 'auth_required'
  | 'misconfigured'
  | 'degraded'
  | 'paused'
  | 'disabled'
  | 'idle'

export function statusKind(state?: MountState, enabled: boolean = true): StatusKind {
  // Operator intent wins over whatever the engine's status flag still says: a
  // disabled or paused mount is not "OK" and not "Syncing" — the list is where
  // an operator scans "why is nothing syncing", and those two answers ARE the
  // answer. (The invariant at the top of this file: the row and the page it
  // links to must never disagree.)
  if (!enabled) return 'disabled'
  if (state?.paused) return 'paused'
  const s = (state?.status || '').toLowerCase()
  if (s === 'ok' || s === 'synced') return 'ok'
  // A `syncing` flag whose run is long gone is not a live sync. Saying so is
  // what stops the console showing a permanent spinner over a dead run.
  if (s === 'syncing') {
    if (isStaleSyncing(state)) return 'interrupted'
    // A stop that has been requested but not yet served: the run ends at its
    // next page boundary. Without this the operator clicks Stop again and
    // again, since nothing visibly changed.
    return state?.stop_requested ? 'stopping' : 'syncing'
  }
  if (s === 'auth_required') return 'auth_required'
  // Checked before 'degraded': the operator action is completely different.
  // Degraded means "flaky, it may recover"; misconfigured means "it will never
  // succeed until you change something", so it must not be diluted into the
  // generic failure bucket.
  if (s === 'misconfigured') return 'misconfigured'
  if (s === 'degraded' || s === 'error') return 'degraded'
  if ((state?.consecutive_failures || 0) > 0) return 'degraded'
  return 'idle'
}

export interface StatusMeta {
  label: string
  /** Badge classes — always the `bg-<c>-500/10` + `text-<c>-400` pair. */
  cls: string
  /** Bare accent colour, for dots and rules that carry no background. */
  dot: string
  Icon: LucideIcon
  /** One line explaining what this state means and what to do about it. */
  hint: string
}

export const STATUS_META: Record<StatusKind, StatusMeta> = {
  ok: {
    label: 'OK',
    cls: 'bg-green-500/10 text-green-400 border-green-500/20',
    dot: 'bg-green-400',
    Icon: CheckCircle,
    hint: 'The last sync completed without errors.',
  },
  syncing: {
    label: 'Syncing',
    cls: 'bg-blue-500/10 text-blue-400 border-blue-500/20',
    dot: 'bg-blue-400',
    Icon: Loader2,
    hint: 'A sync is running right now.',
  },
  auth_required: {
    label: 'Auth required',
    cls: 'bg-yellow-500/10 text-yellow-400 border-yellow-500/20',
    dot: 'bg-yellow-400',
    Icon: ShieldAlert,
    hint: 'The stored credential was rejected. Reconnect the account.',
  },
  misconfigured: {
    label: 'Misconfigured',
    cls: 'bg-orange-500/10 text-orange-400 border-orange-500/20',
    dot: 'bg-orange-400',
    Icon: AlertTriangle,
    hint: 'This will not succeed until the mount or connector settings change.',
  },
  degraded: {
    label: 'Degraded',
    cls: 'bg-red-500/10 text-red-400 border-red-500/20',
    dot: 'bg-red-400',
    Icon: AlertTriangle,
    hint: 'Recent syncs failed. It may recover on its own.',
  },
  interrupted: {
    label: 'Interrupted',
    cls: 'bg-amber-500/10 text-amber-400 border-amber-500/20',
    dot: 'bg-amber-400',
    Icon: AlertTriangle,
    hint: 'The last sync did not finish — it was most likely cut short by the job timeout. The next scheduled run picks up where it stopped; you can also sync now.',
  },
  stopping: {
    label: 'Stopping',
    cls: 'bg-amber-500/10 text-amber-400 border-amber-500/20',
    dot: 'bg-amber-400',
    Icon: StopCircle,
    hint: 'Stop requested — the running sync ends at its next page boundary.',
  },
  paused: {
    label: 'Paused',
    cls: 'bg-zinc-500/10 text-zinc-300 border-white/10',
    dot: 'bg-zinc-400',
    Icon: PauseCircle,
    hint: 'Scheduling is suspended by an operator. The push subscription stays registered — resume to catch up on everything that arrived in the gap.',
  },
  disabled: {
    label: 'Disabled',
    cls: 'bg-zinc-500/10 text-zinc-400 border-white/10',
    dot: 'bg-zinc-500',
    Icon: CircleOff,
    hint: 'This mount is switched off. Its provider subscription is torn down; enable it to sync again.',
  },
  idle: {
    label: 'Idle',
    cls: 'bg-zinc-500/10 text-zinc-400 border-white/10',
    dot: 'bg-zinc-500',
    Icon: HardDrive,
    hint: 'No sync has run yet.',
  },
}

/** True when the engine reports a sync in flight right now. */
export function isSyncing(state?: MountState): boolean {
  // A paused mount is never syncing, whatever `status` still says.
  //
  // `status` is written by the engine at the START of a run and cleared at the
  // end, so a run that ended abnormally leaves it reading `syncing` forever —
  // which is how a mount showed "Syncing" and "Disabled" at the same time, with
  // an empty run history behind it. Pausing is an explicit statement that
  // nothing is running, so it wins over a stale flag.
  if (state?.paused) return false
  if ((state?.status || '').toLowerCase() !== 'syncing') return false
  // ...and neither is a run that cannot possibly still be going.
  return !isStaleSyncing(state)
}

/**
 * Seconds after which a run claiming to be in flight must be dead.
 *
 * Twice the engine's per-mount lease TTL (600s). The job watchdog kills any sync
 * past its timeout, so nothing legitimately runs this long.
 */
const STALE_SYNC_AFTER_SECONDS = 1200

/**
 * True when `status` says syncing but the run behind it is gone.
 *
 * A killed run never reaches `finalize`, so the flag it set on the way in is
 * never cleared — and the console then greys out "Sync now" on that same flag,
 * removing the one action that would fix it. A webhook mount could sit like this
 * indefinitely, since a live subscription means it is never rescheduled either.
 *
 * Measured from the open run's `started_at`, not `last_attempt_at`: only
 * `finalize` stamps the latter, and `finalize` is precisely what a killed run
 * skips, so it is always stale on the path this detects.
 */
export function isStaleSyncing(state?: MountState): boolean {
  if ((state?.status || '').toLowerCase() !== 'syncing') return false
  const startedAt = state?.last_run?.started_at
  // Syncing with no run record behind it: nothing exists that could finish it.
  if (!startedAt) return true
  return Date.now() / 1000 - startedAt > STALE_SYNC_AFTER_SECONDS
}

/** True when an operator has suspended scheduling. */
export function isPaused(state?: MountState): boolean {
  return state?.paused === true
}

/**
 * True when this mount warrants live polling.
 *
 * Wider than `backfill_cursor` alone, which is **absent between chunks** — the
 * cursor only exists at a resume point, so keying refresh on it stopped
 * refreshing exactly while a chunk was being imported.
 */
export function isActive(state?: MountState): boolean {
  return isSyncing(state) || !!state?.backfill_cursor || (state?.backfill_stack?.length ?? 0) > 0
}

export interface BackfillProgress {
  /** Short line for the list row. */
  text: string
  /** True once the walk has finished. */
  done: boolean
  /** 0–1, only when the provider reported a denominator. */
  fraction: number | null
  itemsDone: number
  itemsTotal: number | null
}

/**
 * Progress of an in-flight full walk (first sync, backfill or remap).
 *
 * A large mailbox imports over many chunked runs and can take hours, so this
 * has to show movement — otherwise a working import is indistinguishable from a
 * stalled one and the natural response is to hit Sync again.
 *
 * `backfill_items_total` is frequently absent: most providers do not report a
 * count up front. An absent denominator is a normal state, so it degrades to a
 * running tally rather than a fake percentage.
 */
export function backfillProgress(state?: MountState): BackfillProgress | null {
  // A queued subtree means the walk is still going, even when the current page
  // wrote nothing. Without this, a walk crossing already-imported ground looked
  // identical to an idle mount — which is exactly how a running import was
  // mistaken for a finished one.
  const queued = state?.backfill_stack?.length ?? 0
  const inFlight = !!state?.backfill_cursor || queued > 0 || isSyncing(state)
  const itemsDone = state?.backfill_items_done ?? 0
  // `backfill_complete` records that a walk finished ONCE. It is not cleared
  // when a new walk starts, so on its own it would claim "complete" while a
  // fresh walk is demonstrably running. Only trust it when nothing is in
  // flight.
  const complete = state?.backfill_complete === true && !inFlight
  if (!inFlight && !itemsDone) return null

  const rawTotal = state?.backfill_items_total
  const itemsTotal = typeof rawTotal === 'number' && rawTotal > 0 ? rawTotal : null
  const fraction = itemsTotal ? Math.min(1, itemsDone / itemsTotal) : null
  const n = itemsDone.toLocaleString()

  if (complete) {
    return { text: `Imported ${n} items`, done: true, fraction, itemsDone, itemsTotal }
  }
  // Say what is being walked, not just how many items have gone by. A run of
  // `0 written / 1200 skipped` is progress through ground already imported, and
  // without the queue depth it reads as a stall.
  const queueNote = queued > 0 ? ` · ${queued} subtree${queued === 1 ? '' : 's'} queued` : ''
  return {
    text: itemsTotal
      ? `Importing… ${n} of ${itemsTotal.toLocaleString()}${queueNote}`
      : `Importing… ${n} items so far${queueNote}`,
    done: false,
    fraction,
    itemsDone,
    itemsTotal,
  }
}

/**
 * Compact descriptor of a mount's push (webhook) SUBSCRIPTION, derived from the
 * engine-managed `state.push_*` fields. Null when the mount has no push
 * subscription (poll-only or never subscribed).
 *
 * This describes only whether a subscription exists and is unexpired. It says
 * nothing about whether notifications are being *accepted* — a mount can sit at
 * `push_status: "active"` while every delivery is rejected, which is exactly
 * how the production stall stayed invisible. For that, read
 * `push_deliveries_ok` / `push_deliveries_rejected`; see
 * {@link pushDeliveryHealth}.
 */
export function pushIndicator(state?: MountState): { text: string; cls: string; title?: string } | null {
  const status = (state?.push_status || '').toLowerCase()
  if (!status) return null
  const failed = status === 'failed' || status === 'error' || !!state?.push_last_error
  const active = status === 'active'
  const cls = failed ? 'text-red-400' : active ? 'text-green-400' : 'text-zinc-400'
  let text = `Push: ${failed ? 'failed' : status}`
  if (!failed && state?.push_expires_at) {
    // ISO-8601 string, unlike every other timestamp in MountState.
    const d = new Date(state.push_expires_at)
    if (!Number.isNaN(d.getTime())) text += `, renews ${d.toLocaleDateString()}`
  }
  return { text, cls, title: state?.push_last_error || undefined }
}

export type DeliveryHealth = 'none' | 'healthy' | 'rejecting' | 'silent'

/**
 * Health of webhook *delivery*, as distinct from subscription lifecycle.
 *
 * - `none` — nothing has ever been delivered; there may be no subscription.
 * - `rejecting` — the most recent notification was REJECTED. The provider
 *   thinks it is delivering and the subscription looks active, but nothing is
 *   getting through. This is the failure that reads as "silently stalled".
 * - `silent` — deliveries were once accepted but the last one is old, while the
 *   subscription is still nominally active.
 * - `healthy` — the most recent notification was accepted.
 */
export function pushDeliveryHealth(state?: MountState, silentAfterHours = 24): DeliveryHealth {
  const ok = state?.push_last_delivery_at
  const rejected = state?.push_last_rejected_at
  if (!ok && !rejected) return 'none'
  if (rejected && (!ok || rejected >= ok)) return 'rejecting'
  if (ok) {
    const ageHours = (Date.now() / 1000 - ok) / 3600
    if (ageHours > silentAfterHours) return 'silent'
    return 'healthy'
  }
  return 'none'
}

export interface OutcomeMeta {
  label: string
  cls: string
  dot: string
}

/** Badge styling for a {@link SyncRun} outcome. */
export const RUN_OUTCOME_META: Record<string, OutcomeMeta> = {
  running: { label: 'Running', cls: 'bg-blue-500/10 text-blue-400', dot: 'bg-blue-400' },
  ok: { label: 'OK', cls: 'bg-green-500/10 text-green-400', dot: 'bg-green-400' },
  error: { label: 'Error', cls: 'bg-red-500/10 text-red-400', dot: 'bg-red-400' },
  auth_required: { label: 'Auth required', cls: 'bg-yellow-500/10 text-yellow-400', dot: 'bg-yellow-400' },
  misconfigured: { label: 'Misconfigured', cls: 'bg-orange-500/10 text-orange-400', dot: 'bg-orange-400' },
  skipped: { label: 'Skipped', cls: 'bg-zinc-500/10 text-zinc-400', dot: 'bg-zinc-500' },
  // A run the job watchdog killed, or one cut short by a restart. Amber rather
  // than red: nothing failed and nothing was lost — the resume point survives —
  // but a mount whose runs keep being interrupted is never finishing an import,
  // and that has to be visible. Before this outcome existed such a run simply
  // stayed `Running` forever, so the mount looked busy instead of stuck.
  interrupted: {
    label: 'Interrupted',
    cls: 'bg-amber-500/10 text-amber-400',
    dot: 'bg-amber-400',
  },
}

export function runOutcomeMeta(run: SyncRun): OutcomeMeta {
  return (
    RUN_OUTCOME_META[run.outcome] ?? {
      label: run.outcome || 'unknown',
      cls: 'bg-zinc-500/10 text-zinc-400',
      dot: 'bg-zinc-500',
    }
  )
}

/** How a run was started, phrased for a timeline row. */
export const TRIGGER_LABEL: Record<string, string> = {
  push: 'Webhook',
  schedule: 'Scheduled',
  manual: 'Manual',
  unknown: 'Unknown trigger',
}

/** What kind of walk a run performed. */
/**
 * Labels for the phase a run ACTUALLY executed.
 *
 * Not the mode it was requested with — the two diverge routinely. A run asked
 * for `delta` executes a full reconcile whenever the provider rejects a stale
 * cursor, and a mount with an unfinished import runs a delta pass followed by a
 * backfill chunk on every tick. The history used to show the request, so runs
 * labelled "Delta" would be seen writing five-year-old mail with no explanation
 * available.
 */
export const MODE_LABEL: Record<string, string> = {
  delta: 'Delta',
  full: 'Full walk',
  // Delta first so new items arrive within one interval, then one chunk of the
  // backfill so history fills in behind them.
  'delta+backfill': 'Delta + backfill',
  remap: 'Remap',
}
