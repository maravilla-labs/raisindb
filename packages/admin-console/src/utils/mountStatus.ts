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
  HardDrive,
  Loader2,
  ShieldAlert,
  type LucideIcon,
} from 'lucide-react'
import type { MountState, SyncRun } from '../api/integrations'

export type StatusKind = 'ok' | 'syncing' | 'auth_required' | 'misconfigured' | 'degraded' | 'idle'

export function statusKind(state?: MountState): StatusKind {
  const s = (state?.status || '').toLowerCase()
  if (s === 'ok' || s === 'synced') return 'ok'
  if (s === 'syncing') return 'syncing'
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
  return (state?.status || '').toLowerCase() === 'syncing'
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
  const inFlight = !!state?.backfill_cursor || isSyncing(state)
  const itemsDone = state?.backfill_items_done ?? 0
  const complete = state?.backfill_complete === true
  if (!inFlight && !itemsDone) return null

  const rawTotal = state?.backfill_items_total
  const itemsTotal = typeof rawTotal === 'number' && rawTotal > 0 ? rawTotal : null
  const fraction = itemsTotal ? Math.min(1, itemsDone / itemsTotal) : null
  const n = itemsDone.toLocaleString()

  if (!inFlight || complete) {
    return { text: `Imported ${n} items`, done: true, fraction, itemsDone, itemsTotal }
  }
  return {
    text: itemsTotal
      ? `Importing… ${n} of ${itemsTotal.toLocaleString()}`
      : `Importing… ${n} items so far`,
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
export const MODE_LABEL: Record<string, string> = {
  delta: 'Delta',
  full: 'Full',
  remap: 'Remap',
}
