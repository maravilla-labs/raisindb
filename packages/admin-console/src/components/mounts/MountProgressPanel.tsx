// SPDX-License-Identifier: BSL-1.1

import { CheckCircle2, Layers } from 'lucide-react'
import type { MountState } from '../../api/integrations'
import { backfillProgress, isSyncing } from '../../utils/mountStatus'

interface MountProgressPanelProps {
  state?: MountState
}

/**
 * Import progress for a mount's full walk.
 *
 * Two shapes, because the backend contract has two. When the provider reports
 * `backfill_items_total` there is a real denominator and a bar is honest. When
 * it does not — which is the common case, most providers cannot count up front —
 * there is deliberately NO bar: a progress bar with a made-up denominator is
 * worse than a number, because it implies an ETA that does not exist. The
 * fallback is a large running tally plus an indeterminate shimmer that conveys
 * "moving" without claiming "this far along".
 */
export default function MountProgressPanel({ state }: MountProgressPanelProps) {
  const progress = backfillProgress(state)
  const syncing = isSyncing(state)
  // Trust `backfill_complete` only when the walk is not still going. The flag
  // records that a walk finished ONCE and is not cleared when a new one starts,
  // so on its own it claims "complete" while a fresh walk is demonstrably
  // running — which is how the card read "Full walk complete" while importing
  // five-year-old mail. `progress.done` already applies that rule.
  const complete = progress?.done === true

  if (!progress) {
    return (
      <p className="text-sm text-zinc-500">
        No import has run yet. Trigger a sync to start the first full walk.
      </p>
    )
  }

  const pct = progress.fraction !== null ? Math.round(progress.fraction * 100) : null
  const pendingSubtrees = state?.backfill_stack?.length ?? 0

  return (
    <div className="space-y-3">
      <div className="flex items-baseline justify-between gap-3">
        <div className="flex items-baseline gap-2">
          <span className="text-3xl font-semibold text-white tabular-nums">
            {progress.itemsDone.toLocaleString()}
          </span>
          <span className="text-sm text-zinc-500">
            {progress.itemsTotal
              ? `of ${progress.itemsTotal.toLocaleString()} items`
              : 'items imported'}
          </span>
        </div>
        {pct !== null && (
          <span className="text-lg font-medium text-primary-300 tabular-nums">{pct}%</span>
        )}
      </div>

      {pct !== null ? (
        <div className="w-full bg-white/10 rounded-full h-2 overflow-hidden">
          <div
            className="bg-primary-500 h-2 rounded-full transition-all duration-500"
            style={{ width: `${pct}%` }}
          />
        </div>
      ) : (
        syncing && (
          // Indeterminate: motion without a false denominator.
          <div className="w-full bg-white/10 rounded-full h-2 overflow-hidden">
            <div className="bg-primary-500/60 h-2 w-1/3 rounded-full animate-pulse" />
          </div>
        )
      )}

      <div className="flex items-center gap-4 text-xs flex-wrap">
        {complete ? (
          <span className="flex items-center gap-1.5 text-green-400">
            <CheckCircle2 className="w-3.5 h-3.5" />
            Full walk complete
          </span>
        ) : (
          <span className="text-blue-400">{progress.text}</span>
        )}

        {/* The mount's life AFTER the walk is delta work, which was invisible:
            a settled mount read "Full walk complete / N items" forever while
            incremental syncs kept importing. */}
        {complete && (state?.delta_items_done ?? 0) > 0 && (
          <span className="text-zinc-400">
            {state!.delta_items_done!.toLocaleString()} item
            {state!.delta_items_done === 1 ? '' : 's'} via delta since
          </span>
        )}

        {pendingSubtrees > 0 && (
          <span
            className="flex items-center gap-1.5 text-zinc-500"
            title="Subtrees the walk still has to descend into."
          >
            <Layers className="w-3.5 h-3.5" />
            {pendingSubtrees} subtree{pendingSubtrees === 1 ? '' : 's'} queued
          </span>
        )}

        {state?.backfill_cursor && (
          <span
            className="text-zinc-600 font-mono truncate max-w-[220px]"
            title={`Resume point for the next chunk: ${state.backfill_cursor}`}
          >
            cursor {state.backfill_cursor}
          </span>
        )}
      </div>

      {!progress.itemsTotal && !complete && (
        <p className="text-[11px] text-zinc-600">
          This provider does not report a total up front, so there is no percentage — only the
          count, which should keep rising while the import runs.
        </p>
      )}
    </div>
  )
}
