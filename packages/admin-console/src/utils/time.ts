// SPDX-License-Identifier: BSL-1.1

/**
 * Shared time formatting.
 *
 * Consolidates the near-identical `formatRelativeTime` / `formatTimeAgo` that
 * had been copied into `pages/management/JobsManagement.tsx` and
 * `pages/agents/AgentDetail.tsx`.
 *
 * The unit distinction here is load-bearing. The mount state contract reports
 * `last_sync_at`, `last_attempt_at`, `push_last_delivery_at` and every
 * `SyncRun` timestamp in **epoch seconds**, while `Date` takes milliseconds —
 * feeding one to the other is what rendered a live mount as "synced 1/21/1970".
 * Functions that take seconds say so in their name; there is no overload that
 * guesses.
 */

/** Milliseconds in a second, named so the `* 1000` conversions read explicitly. */
const MS_PER_SECOND = 1000

/**
 * Epoch **seconds** → Date. Returns null for absent or unparseable input so
 * callers render a dash rather than "Invalid Date".
 */
export function dateFromEpochSeconds(seconds: number | undefined | null): Date | null {
  if (seconds === undefined || seconds === null) return null
  const n = typeof seconds === 'string' ? Number(seconds) : seconds
  if (!Number.isFinite(n) || n <= 0) return null
  const d = new Date(n * MS_PER_SECOND)
  return Number.isNaN(d.getTime()) ? null : d
}

/** Parse an ISO-8601 string (or anything `Date` accepts). Null when invalid. */
export function dateFromIso(value: string | undefined | null): Date | null {
  if (!value) return null
  const d = new Date(value)
  return Number.isNaN(d.getTime()) ? null : d
}

/** `3m ago`, `2h ago`, `4d ago`. Empty input renders as an em dash. */
export function formatRelative(date: Date | null | undefined): string {
  if (!date) return '—'
  const diffSec = Math.floor((Date.now() - date.getTime()) / MS_PER_SECOND)
  if (diffSec < 0) return 'just now' // clock skew between server and browser
  if (diffSec < 10) return 'just now'
  if (diffSec < 60) return `${diffSec}s ago`
  const diffMin = Math.floor(diffSec / 60)
  if (diffMin < 60) return `${diffMin}m ago`
  const diffHour = Math.floor(diffMin / 60)
  if (diffHour < 24) return `${diffHour}h ago`
  const diffDay = Math.floor(diffHour / 24)
  if (diffDay < 30) return `${diffDay}d ago`
  return date.toLocaleDateString()
}

/** Relative time from epoch **seconds**. */
export function formatRelativeSeconds(seconds: number | undefined | null): string {
  return formatRelative(dateFromEpochSeconds(seconds))
}

/** Absolute, locale-formatted timestamp — for `title` tooltips. */
export function formatAbsolute(date: Date | null | undefined): string {
  return date ? date.toLocaleString() : '—'
}

/** Absolute timestamp from epoch **seconds**. */
export function formatAbsoluteSeconds(seconds: number | undefined | null): string {
  return formatAbsolute(dateFromEpochSeconds(seconds))
}

/** `840ms`, `2.4s`, `1m 12s`. */
export function formatDuration(ms: number | undefined | null): string {
  if (ms === undefined || ms === null || !Number.isFinite(ms) || ms < 0) return '—'
  if (ms < 1000) return `${Math.round(ms)}ms`
  const sec = ms / 1000
  if (sec < 60) return `${sec.toFixed(sec < 10 ? 1 : 0)}s`
  const min = Math.floor(sec / 60)
  const rem = Math.round(sec % 60)
  if (min < 60) return `${min}m ${rem}s`
  const hr = Math.floor(min / 60)
  return `${hr}h ${min % 60}m`
}

/**
 * Duration of a run that may still be in flight: prefers the recorded
 * `duration_ms`, otherwise measures from `started_at` to now.
 */
export function formatRunDuration(
  durationMs: number | undefined,
  startedAtSeconds: number | undefined,
): string {
  if (typeof durationMs === 'number') return formatDuration(durationMs)
  const started = dateFromEpochSeconds(startedAtSeconds)
  if (!started) return '—'
  return formatDuration(Date.now() - started.getTime())
}
