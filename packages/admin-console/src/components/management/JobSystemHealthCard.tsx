import { useEffect, useState } from 'react'
import { AlertTriangle, CheckCircle, ChevronDown, ChevronRight, HelpCircle, ShieldAlert, Zap } from 'lucide-react'
import GlassCard from '../GlassCard'
import { managementApi, JobSystemHealth, BreakerHealth, CategoryPoolStats } from '../../api/management'

/**
 * Upstream breakers and pool saturation — the "is anything stuck, and why"
 * panel.
 *
 * An open breaker is the headline: it means work is not being ATTEMPTED, which
 * looks identical to an idle system from every other view on this page. Pool
 * saturation is the supporting detail — it distinguishes "the upstream is down"
 * from "the machine is simply busy".
 *
 * When nothing is wrong the card stays deliberately quiet: one muted line, no
 * numbers, details behind a toggle. An operator must be able to tell fine from
 * not fine without reading anything.
 */

/** A pool this close to its permit ceiling is worth showing unprompted. */
const SATURATION_WARN = 0.9

/** Poll interval. Unlike /management/jobs/stats, this endpoint reads only live
 *  process state (breaker registry + pool atomics) and touches no storage, so
 *  polling it is cheap. It has to be polled: a breaker's cooldown is measured
 *  in tens of seconds, and a one-shot snapshot taken before an outage would
 *  show "healthy" for the whole incident. */
const POLL_MS = 5000

function permitsInUse(pool: CategoryPoolStats): number {
  return pool.handler_permits_max - pool.handler_permits_available
}

function saturation(pool: CategoryPoolStats): number {
  if (pool.handler_permits_max === 0) return 0
  return permitsInUse(pool) / pool.handler_permits_max
}

function queued(pool: CategoryPoolStats): number {
  return pool.queue_depth_high + pool.queue_depth_normal + pool.queue_depth_low
}

/**
 * Every tone is a LITERAL class string, never an interpolated one. This project
 * builds Tailwind 4 with no safelist, so a class assembled at runtime
 * (`text-${tone}-400`) is never emitted and silently renders colourless — which
 * on this card would mean an open breaker looking exactly like a closed one.
 */
const BREAKER_TONES = {
  open: {
    frame: 'bg-red-500/10 border-red-500/30',
    icon: 'w-3.5 h-3.5 shrink-0 text-red-400',
    label: 'text-[10px] uppercase tracking-wide font-medium text-red-400',
  },
  half_open: {
    frame: 'bg-yellow-500/10 border-yellow-500/30',
    icon: 'w-3.5 h-3.5 shrink-0 text-yellow-400',
    label: 'text-[10px] uppercase tracking-wide font-medium text-yellow-400',
  },
  closed: {
    frame: 'bg-white/5 border-white/10',
    icon: 'w-3.5 h-3.5 shrink-0 text-zinc-500',
    label: 'text-[10px] uppercase tracking-wide font-medium text-zinc-500',
  },
} as const

function BreakerRow({ breaker }: { breaker: BreakerHealth }) {
  const tone = BREAKER_TONES[breaker.state] ?? BREAKER_TONES.closed

  return (
    <div className={`p-3 rounded-lg border ${tone.frame}`}>
      <div className="flex items-center justify-between gap-3">
        <div className="flex items-center gap-2 min-w-0">
          <ShieldAlert className={tone.icon} />
          <span className="font-mono text-xs text-white truncate">{breaker.key}</span>
        </div>
        <div className="flex items-center gap-3 shrink-0">
          {breaker.consecutive_failures > 0 && (
            <span className="text-[10px] text-zinc-400">
              {breaker.consecutive_failures} consecutive
            </span>
          )}
          {/* Seconds, not a countdown: this card re-polls, and a timer ticking
              against a stale snapshot would keep counting past zero. */}
          {breaker.next_probe_in_secs != null && (
            <span className="text-[10px] text-zinc-400">
              probe in <span className="font-mono text-white">{breaker.next_probe_in_secs}s</span>
            </span>
          )}
          <span className={tone.label}>{breaker.state.replace('_', ' ')}</span>
        </div>
      </div>
      {breaker.last_error && breaker.state !== 'closed' && (
        <div className="mt-2 text-[11px] font-mono text-zinc-400 break-words line-clamp-2">
          {breaker.last_error}
        </div>
      )}
    </div>
  )
}

function PoolRow({ pool }: { pool: CategoryPoolStats }) {
  const used = permitsInUse(pool)
  const busy = saturation(pool)
  const total = queued(pool)

  return (
    <div className="p-3 bg-white/5 rounded-lg border border-white/10">
      <div className="flex items-center justify-between mb-2">
        <span className="text-xs font-medium text-white">{pool.category}</span>
        <span className="text-[10px] text-zinc-500">{pool.dispatcher_workers} workers</span>
      </div>
      <div className="flex items-center gap-2">
        <div className="flex-1 bg-white/10 rounded-full h-1">
          <div
            className={`h-1 rounded-full transition-all ${busy >= SATURATION_WARN ? 'bg-red-400' : 'bg-primary-400'}`}
            style={{ width: `${Math.min(busy * 100, 100)}%` }}
          />
        </div>
        <span className="text-[10px] font-mono text-zinc-400 w-14 text-right">
          {used}/{pool.handler_permits_max}
        </span>
      </div>
      <div className="mt-2 flex items-center gap-3 text-[10px] text-zinc-500">
        <span>
          {pool.active_handler_tasks.toLocaleString()} in flight
        </span>
        <span>
          {total.toLocaleString()} queued
        </span>
      </div>
    </div>
  )
}

/**
 * THREE states, never two. A poll can fail — an admin JWT expiring mid-session
 * (this endpoint answers 403 to a non-admin token), a network drop, a 5xx, a
 * body we do not recognise, an older server without the route.
 *
 * `unknown` exists because rendering a failed poll as "healthy" would make this
 * panel assert a false all-clear: an indicator vouching for exactly the silence
 * the 2026-09-02 incident was made of, with a green tick on top. Showing
 * nothing is better than that; saying "could not ask" is better still.
 */
type HealthFetchState =
  | { status: 'loading' }
  | { status: 'ok'; health: JobSystemHealth }
  | { status: 'unknown' }

export default function JobSystemHealthCard() {
  const [state, setState] = useState<HealthFetchState>({ status: 'loading' })
  const [expanded, setExpanded] = useState(false)

  useEffect(() => {
    let cancelled = false
    const fetchHealth = async () => {
      try {
        const response = await managementApi.getJobSystemHealth()
        if (cancelled) return
        // A 2xx whose body is not the shape we expect is also "could not ask".
        // Trusting a malformed body would silently read as zero breakers and
        // zero pools, which renders as healthy.
        if (response.success && Array.isArray(response.data?.breakers) && Array.isArray(response.data?.pools)) {
          setState({ status: 'ok', health: response.data })
        } else {
          setState({ status: 'unknown' })
        }
      } catch {
        // A failed poll DISCARDS the previous answer rather than keeping it.
        // Stale state is the subtle version of the same lie: the upstream can
        // trip during precisely the window we stopped being able to ask.
        if (!cancelled) setState({ status: 'unknown' })
      }
    }
    fetchHealth()
    const interval = setInterval(fetchHealth, POLL_MS)
    return () => {
      cancelled = true
      clearInterval(interval)
    }
  }, [])

  // Before the first answer, occupy no space and make no claim.
  if (state.status === 'loading') return null

  // Could not ask. Amber icon AND amber heading, where the healthy line is
  // grey on grey — so a glance separates "nothing is wrong" from "we do not
  // know", without reading the words.
  if (state.status === 'unknown') {
    return (
      <GlassCard className="mb-4">
        <div className="flex items-center gap-2">
          <HelpCircle className="w-4 h-4 text-yellow-500/80" />
          <h3 className="text-sm font-medium text-yellow-500/80">
            Job system status unavailable
          </h3>
          <span className="text-xs text-zinc-500">
            could not reach the health endpoint — this is not an all-clear
          </span>
        </div>
      </GlassCard>
    )
  }

  const health = state.health
  const trippedBreakers = health.breakers.filter((b) => b.state !== 'closed')
  const saturatedPools = health.pools.filter((p) => saturation(p) >= SATURATION_WARN)
  const healthy = trippedBreakers.length === 0 && saturatedPools.length === 0
  // Breakers that have never failed are noise; ones with a failure streak are
  // worth seeing even while still closed, because that is an outage starting.
  const strainedBreakers = health.breakers.filter(
    (b) => b.state === 'closed' && b.consecutive_failures > 0
  )

  return (
    <GlassCard className="mb-4">
      <button
        onClick={() => setExpanded((prev) => !prev)}
        className="w-full flex items-center justify-between gap-3 text-left"
      >
        <div className="flex items-center gap-2 min-w-0">
          {healthy ? (
            <CheckCircle className="w-4 h-4 text-zinc-500" />
          ) : (
            <AlertTriangle className="w-4 h-4 text-red-400" />
          )}
          <h3 className={`text-sm font-medium ${healthy ? 'text-zinc-400' : 'text-white'}`}>
            {healthy
              ? 'Upstreams and pools healthy'
              : trippedBreakers.length > 0
                ? `${trippedBreakers.length} upstream${trippedBreakers.length === 1 ? '' : 's'} unavailable — jobs are parked, not failing`
                : `${saturatedPools.length} pool${saturatedPools.length === 1 ? '' : 's'} at capacity`}
          </h3>
        </div>
        <div className="flex items-center gap-2 shrink-0">
          {strainedBreakers.length > 0 && healthy && (
            <span className="text-[10px] text-yellow-400 flex items-center gap-1">
              <Zap className="w-3 h-3" />
              {strainedBreakers.length} failing
            </span>
          )}
          {expanded ? (
            <ChevronDown className="w-4 h-4 text-zinc-500" />
          ) : (
            <ChevronRight className="w-4 h-4 text-zinc-500" />
          )}
        </div>
      </button>

      {/* A tripped breaker is never hidden behind the toggle: it is the reason
          nothing is progressing, and burying it is how the last outage stayed
          invisible for half an hour. */}
      {trippedBreakers.length > 0 && (
        <div className="mt-3 space-y-2">
          {trippedBreakers.map((breaker) => (
            <BreakerRow key={breaker.key} breaker={breaker} />
          ))}
        </div>
      )}

      {expanded && (
        <div className="mt-3 space-y-3">
          {health.pools.length > 0 && (
            <div>
              <div className="text-[10px] uppercase tracking-wide text-zinc-500 mb-2">
                Handler pools
              </div>
              <div className="grid grid-cols-1 md:grid-cols-3 gap-3">
                {health.pools.map((pool) => (
                  <PoolRow key={pool.category} pool={pool} />
                ))}
              </div>
            </div>
          )}
          <div>
            <div className="text-[10px] uppercase tracking-wide text-zinc-500 mb-2">
              Upstream breakers
            </div>
            {health.breakers.length === 0 ? (
              <div className="text-xs text-zinc-500">
                No upstream has been called yet on this node. Breakers appear on first use.
              </div>
            ) : (
              <div className="space-y-2">
                {health.breakers
                  .filter((b) => b.state === 'closed')
                  .map((breaker) => (
                    <BreakerRow key={breaker.key} breaker={breaker} />
                  ))}
              </div>
            )}
          </div>
          <div className="text-[10px] text-zinc-600">
            Breakers and pools are per node. On a cluster each node discovers an
            outage separately.
          </div>
        </div>
      )}
    </GlassCard>
  )
}
