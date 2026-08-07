// SPDX-License-Identifier: BSL-1.1

import { AlertTriangle, Hourglass, PauseCircle, ShieldAlert, Timer } from 'lucide-react'
import type { MountState, SyncRun } from '../../api/integrations'

interface Props {
  state?: MountState
  lastRun?: SyncRun
}

/**
 * Why this mount has edits it has not pushed.
 *
 * The count alone is the least useful half of the fact. A drain reports
 * `outcome: "ok"` whether it pushed everything or nothing, so a mount with
 * `pending: 1` looks identical whether it ran out of time, was stopped, was
 * blocked by a delete rail, or is standing off after a missing OAuth scope —
 * four situations with four different responses, one of which is "do nothing,
 * it will clear itself".
 *
 * Every one of these is already recorded by the engine; none of it was
 * displayed. Finding out which applied meant reading the server log, and the
 * one that bit hardest — a stop request left set from an earlier incident,
 * making every later run exit immediately and report OK — was invisible from
 * the console entirely.
 *
 * Ordered most-actionable first, and only ONE is shown: a list of maybes is
 * what an operator skims past.
 */
export default function PendingReason({ state, lastRun }: Props) {
  const pending = lastRun?.writeback_pending ?? 0
  if (!pending) return null

  const drain = state?.last_drain
  const now = Math.floor(Date.now() / 1000)
  const standoff = state?.writeback_retry_after
  const standoffSecs = standoff && standoff > now ? standoff - now : 0

  // A configuration failure outranks everything: nothing will be pushed until a
  // human changes something outside this system, so the other reasons are moot.
  if (state?.writeback_status === 'misconfigured' || standoffSecs > 0) {
    return (
      <Line tone="bad" Icon={ShieldAlert}>
        <strong>Nothing can be pushed until the configuration is fixed.</strong>{' '}
        {state?.writeback_last_error || 'The provider refused the write outright.'}
        {standoffSecs > 0 && (
          <> Retrying in about {Math.ceil(standoffSecs / 60)} min, or sooner once it is fixed.</>
        )}
      </Line>
    )
  }

  if (drain?.blocked) {
    return (
      <Line tone="warn" Icon={AlertTriangle}>
        <strong>A delete rail blocked this drain.</strong> Every pending edit is parked and
        nothing was sent. Review the deletes above and release them to continue.
      </Line>
    )
  }

  if (drain?.stopped) {
    return (
      <Line tone="warn" Icon={PauseCircle}>
        <strong>A stop request ended the drain.</strong> Nothing was lost — the edits are queued
        and the next run picks them up.
      </Line>
    )
  }

  if (drain?.truncated) {
    return (
      <Line tone="warn" Icon={Timer}>
        <strong>The run spent its time budget.</strong> This is normal on a large backlog: each
        run pushes what it can and the next continues. Watch that the number falls.
      </Line>
    )
  }

  if ((drain?.parked ?? 0) > 0) {
    return (
      <Line tone="warn" Icon={AlertTriangle}>
        <strong>{drain?.parked} edit(s) are parked on a conflict.</strong> The provider refused
        them as stale and this mount's policy leaves them for a human.
      </Line>
    )
  }

  // Pending with no recorded reason. Said plainly rather than guessed at: the
  // engine records a reason for every early exit, so this means the drain
  // simply has not reached them yet.
  return (
    <Line tone="warn" Icon={Hourglass}>
      Queued for the next drain. If this number does not fall within a few runs, check the
      writeback status above.
    </Line>
  )
}

function Line({
  tone,
  Icon,
  children,
}: {
  tone: 'warn' | 'bad'
  Icon: typeof Hourglass
  children: React.ReactNode
}) {
  const cls = tone === 'bad' ? 'text-red-400' : 'text-amber-400'
  return (
    <p className={`flex items-start gap-1.5 text-xs mt-2 ${cls}`}>
      <Icon className="w-3.5 h-3.5 flex-shrink-0 mt-0.5" />
      <span className="min-w-0">{children}</span>
    </p>
  )
}
