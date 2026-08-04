// SPDX-License-Identifier: BSL-1.1

import { useState } from 'react'
import {
  AlertOctagon,
  ChevronDown,
  ChevronRight,
  Clock,
  Hand,
  CalendarClock,
  Webhook,
  HelpCircle,
  type LucideIcon,
} from 'lucide-react'
import type { SyncRun } from '../../api/integrations'
import { MODE_LABEL, TRIGGER_LABEL, runOutcomeMeta } from '../../utils/mountStatus'
import { formatAbsoluteSeconds, formatRelativeSeconds, formatRunDuration } from '../../utils/time'

const TRIGGER_ICON: Record<string, LucideIcon> = {
  push: Webhook,
  schedule: CalendarClock,
  manual: Hand,
  unknown: HelpCircle,
}

interface MountRunTimelineProps {
  runs: SyncRun[]
}

/**
 * Run history as a vertical timeline, newest first.
 *
 * Modelled on `management/FlowStepTimeline` — same rail-and-node shape — but a
 * sync run is not a flow step: it carries four independent counters and a
 * trigger, and there is no ordering between runs beyond time. So the row leads
 * with *how it started* (webhook / scheduled / manual), which is the first thing
 * you need when a mount stops updating: no `push` rows means notifications
 * stopped arriving, whatever the subscription claims.
 */
export default function MountRunTimeline({ runs }: MountRunTimelineProps) {
  const [expanded, setExpanded] = useState<number | null>(null)

  if (runs.length === 0) {
    return (
      <p className="text-sm text-zinc-500 py-2">
        No runs recorded yet. History starts accumulating from the next sync.
      </p>
    )
  }

  return (
    <div className="relative">
      {/* Rail. Stops short of the last node so it does not dangle. */}
      <div className="absolute left-[7px] top-2 bottom-4 w-px bg-white/10" aria-hidden />

      <ul className="space-y-1">
        {runs.map((run, i) => {
          const meta = runOutcomeMeta(run)
          const TriggerIcon = TRIGGER_ICON[run.trigger] ?? HelpCircle
          const isOpen = expanded === i
          const running = run.outcome === 'running'
          const hasDetail = !!run.error || run.state_write_skipped === true

          return (
            <li key={`${run.started_at}-${i}`} className="relative pl-6">
              {/* Node */}
              <span
                className={`absolute left-0 top-[11px] w-[15px] h-[15px] rounded-full border-2 border-zinc-900 ${meta.dot} ${
                  running ? 'animate-pulse' : ''
                }`}
                aria-hidden
              />

              <div
                className={`rounded-lg border transition-colors ${
                  isOpen ? 'border-white/15 bg-white/[0.04]' : 'border-transparent hover:bg-white/[0.03]'
                }`}
              >
                <button
                  type="button"
                  onClick={() => setExpanded(isOpen ? null : i)}
                  disabled={!hasDetail}
                  className="w-full text-left px-2.5 py-2 flex items-center gap-3 disabled:cursor-default"
                >
                  <TriggerIcon className="w-3.5 h-3.5 text-zinc-500 shrink-0" />

                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2 flex-wrap">
                      <span className="text-sm text-white">
                        {TRIGGER_LABEL[run.trigger] ?? run.trigger}
                      </span>
                      <span className="text-xs text-zinc-500">
                        {MODE_LABEL[run.mode] ?? run.mode}
                      </span>
                      <span className={`px-1.5 py-0.5 text-[10px] rounded ${meta.cls}`}>
                        {meta.label}
                      </span>
                      {run.state_write_skipped === true && (
                        <span className="px-1.5 py-0.5 text-[10px] rounded bg-orange-500/10 text-orange-400 flex items-center gap-1">
                          <AlertOctagon className="w-2.5 h-2.5" />
                          Record not saved
                        </span>
                      )}
                    </div>

                    <div className="flex items-center gap-2.5 mt-1 text-[11px] text-zinc-500 flex-wrap tabular-nums">
                      <span title={formatAbsoluteSeconds(run.started_at)}>
                        {formatRelativeSeconds(run.started_at)}
                      </span>
                      <span className="flex items-center gap-1">
                        <Clock className="w-2.5 h-2.5" />
                        {formatRunDuration(run.duration_ms, run.started_at)}
                      </span>
                      {run.written > 0 && <span className="text-green-400/80">+{run.written}</span>}
                      {run.deleted > 0 && <span className="text-red-400/80">−{run.deleted}</span>}
                      {run.skipped > 0 && <span>{run.skipped} skipped</span>}
                      {run.failed > 0 && (
                        <span className="text-red-400">{run.failed} failed</span>
                      )}
                      {run.written === 0 &&
                        run.deleted === 0 &&
                        run.failed === 0 &&
                        run.outcome === 'ok' && <span>no changes</span>}
                    </div>
                  </div>

                  {hasDetail &&
                    (isOpen ? (
                      <ChevronDown className="w-4 h-4 text-zinc-500 shrink-0" />
                    ) : (
                      <ChevronRight className="w-4 h-4 text-zinc-600 shrink-0" />
                    ))}
                </button>

                {isOpen && (
                  <div className="px-2.5 pb-2.5 pt-0 space-y-2">
                    {run.state_write_skipped === true && (
                      <p className="text-xs text-orange-300/90 bg-orange-500/10 border border-orange-500/20 rounded p-2">
                        This run executed but its record did not persist, so its counters may
                        understate what actually happened. Totals elsewhere on this page exclude it.
                      </p>
                    )}
                    {run.error && (
                      <pre className="text-xs text-red-300/90 bg-red-500/10 border border-red-500/20 rounded p-2 whitespace-pre-wrap break-words font-mono">
                        {run.error}
                      </pre>
                    )}
                    <div className="grid grid-cols-2 sm:grid-cols-4 gap-2 text-[11px]">
                      {(
                        [
                          ['Written', run.written],
                          ['Skipped', run.skipped],
                          ['Deleted', run.deleted],
                          ['Failed', run.failed],
                        ] as const
                      ).map(([label, value]) => (
                        <div key={label} className="rounded bg-white/5 px-2 py-1.5">
                          <div className="text-zinc-500">{label}</div>
                          <div className="text-zinc-200 tabular-nums">{value.toLocaleString()}</div>
                        </div>
                      ))}
                    </div>
                    <div className="text-[11px] text-zinc-500 tabular-nums">
                      {run.items_done.toLocaleString()} items processed · started{' '}
                      {formatAbsoluteSeconds(run.started_at)}
                      {run.finished_at ? ` · finished ${formatAbsoluteSeconds(run.finished_at)}` : ''}
                    </div>
                  </div>
                )}
              </div>
            </li>
          )
        })}
      </ul>
    </div>
  )
}
