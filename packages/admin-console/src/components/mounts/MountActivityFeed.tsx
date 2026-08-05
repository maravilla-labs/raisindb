// SPDX-License-Identifier: BSL-1.1

import { useEffect, useRef } from 'react'
import { FilePlus2, FileMinus2, FilePen, Radio, WifiOff } from 'lucide-react'
import type { WsStatus } from '../../api/ws-events'
import { formatRelative } from '../../utils/time'

/**
 * One rendered line of the feed.
 *
 * A burst, not an item. Node events arrive per **batch commit**, not per item —
 * a chunked import commits hundreds of nodes at once and every one of them
 * emits an event within the same few milliseconds. Rendering one row per event
 * would produce a blur of identical lines scrolling past faster than anyone can
 * read, and would imply per-item granularity the transport does not provide.
 * So contiguous events of the same kind are coalesced into a single row that
 * says how many, and names one representative path.
 */
export interface ActivityBurst {
  id: string
  kind: 'created' | 'updated' | 'deleted' | 'processed'
  count: number
  /** Representative path — the first in the burst. */
  samplePath: string
  firstAt: number
  lastAt: number
  /**
   * Cumulative item count at the moment this burst was recorded.
   *
   * The sync's progress events carry running totals, not per-event deltas, so
   * the feed subtracts the previous total to get "how many landed in this
   * commit". Absent on bursts from older sources.
   */
  runningTotal?: number
}

const KIND_META = {
  created: { Icon: FilePlus2, cls: 'text-green-400', bg: 'bg-green-500/10', verb: 'Created' },
  updated: { Icon: FilePen, cls: 'text-blue-400', bg: 'bg-blue-500/10', verb: 'Updated' },
  deleted: { Icon: FileMinus2, cls: 'text-red-400', bg: 'bg-red-500/10', verb: 'Deleted' },
  // Progress events carry ITEMS WALKED, not nodes written — a page of 500 with
  // 499 unchanged etags is 500 processed and 1 created. Labelling that "Created
  // 500 nodes" invented work that never happened and made a healthy re-scan
  // look like a runaway import.
  processed: { Icon: FilePen, cls: 'text-zinc-400', bg: 'bg-white/5', verb: 'Processed' },
} as const

interface MountActivityFeedProps {
  bursts: ActivityBurst[]
  status: WsStatus
  /** Where the feed is watching, for the empty state. */
  watching: string
}

/**
 * Live feed of what the connector is writing right now.
 *
 * Follows `management/LogViewer`'s shape — fixed-height scroller, newest at the
 * bottom, autoscroll while pinned — but the content is bursts rather than log
 * lines, and it carries its own connection indicator because a silent feed has
 * two very different meanings (nothing is happening / the socket is down) and
 * the user must be able to tell which.
 */
export default function MountActivityFeed({ bursts, status, watching }: MountActivityFeedProps) {
  const scrollRef = useRef<HTMLDivElement>(null)
  const pinnedRef = useRef(true)

  // Autoscroll only while the user is already at the bottom — scrolling back to
  // read something must not be yanked away by the next burst.
  useEffect(() => {
    const el = scrollRef.current
    if (el && pinnedRef.current) el.scrollTop = el.scrollHeight
  }, [bursts])

  const onScroll = () => {
    const el = scrollRef.current
    if (!el) return
    pinnedRef.current = el.scrollHeight - el.scrollTop - el.clientHeight < 32
  }

  return (
    <div>
      <div className="flex items-center justify-between mb-2">
        <div className="flex items-center gap-1.5">
          <Radio className="w-3.5 h-3.5 text-zinc-500" />
          <h3 className="text-sm font-medium text-white">Live activity</h3>
        </div>
        <ConnectionPill status={status} />
      </div>

      <div
        ref={scrollRef}
        onScroll={onScroll}
        className="h-56 overflow-y-auto rounded-lg border border-white/10 bg-black/20 p-2 space-y-1"
      >
        {bursts.length === 0 ? (
          <div className="h-full flex items-center justify-center text-center px-4">
            <p className="text-xs text-zinc-500 leading-relaxed">
              {status === 'live' ? (
                <>
                  Watching <span className="font-mono text-zinc-400">{watching}</span>.
                  <br />
                  Writes appear here as the connector makes them.
                </>
              ) : (
                <>
                  Not connected to the live stream.
                  <br />
                  This page is falling back to polling every 5s.
                </>
              )}
            </p>
          </div>
        ) : (
          bursts.map((b) => {
            const meta = KIND_META[b.kind]
            return (
              <div
                key={b.id}
                className="flex items-center gap-2 px-2 py-1.5 rounded animate-fade-in hover:bg-white/5"
              >
                <span
                  className={`inline-flex items-center justify-center w-5 h-5 rounded shrink-0 ${meta.bg}`}
                >
                  <meta.Icon className={`w-3 h-3 ${meta.cls}`} />
                </span>
                <span className="text-xs text-zinc-300 min-w-0 flex-1 truncate">
                  <span className={meta.cls}>{meta.verb}</span>{' '}
                  <span className="tabular-nums">
                    {b.count > 1
                      ? `${b.count.toLocaleString()} ${b.kind === 'processed' ? 'items' : 'nodes'}`
                      : b.kind === 'processed'
                        ? '1 item'
                        : '1 node'}
                  </span>{' '}
                  <span className="text-zinc-500">
                    {b.count > 1 ? 'including ' : 'at '}
                    <span className="font-mono">{b.samplePath}</span>
                  </span>
                </span>
                <span className="text-[10px] text-zinc-600 shrink-0 tabular-nums">
                  {formatRelative(new Date(b.lastAt))}
                </span>
              </div>
            )
          })
        )}
      </div>

      <p className="text-[11px] text-zinc-600 mt-1.5">
        Events arrive per batch commit, so each line is one commit rather than one item.
      </p>
    </div>
  )
}

/** Connected / disconnected indicator, matching the Jobs page idiom. */
export function ConnectionPill({ status }: { status: WsStatus }) {
  if (status === 'live') {
    return (
      <span className="flex items-center gap-1.5 text-[11px] text-green-400">
        <span className="w-1.5 h-1.5 rounded-full bg-green-400 animate-pulse" />
        Live
      </span>
    )
  }
  if (status === 'connecting') {
    return (
      <span className="flex items-center gap-1.5 text-[11px] text-zinc-400">
        <span className="w-1.5 h-1.5 rounded-full bg-zinc-400 animate-pulse" />
        Connecting…
      </span>
    )
  }
  return (
    <span
      className="flex items-center gap-1.5 text-[11px] text-zinc-500"
      title="The live stream is unavailable; this page is polling instead."
    >
      <WifiOff className="w-3 h-3" />
      Polling
    </span>
  )
}
