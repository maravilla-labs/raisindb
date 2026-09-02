import { useEffect, useState } from 'react'
import { AlertTriangle, CheckCircle, HelpCircle } from 'lucide-react'
import GlassCard from '../GlassCard'
import { managementApi, TenantJobHealth } from '../../api/management'

/**
 * Is MY background processing degraded, and how much of MY work is waiting.
 *
 * # What this card used to show, and why it does not
 *
 * It rendered upstream breaker keys, their consecutive-failure counts, their
 * next-probe timers and host-wide pool saturation. All four are shared across
 * every tenant on the host: a breaker is keyed by upstream, so its failure
 * streak and probe timer are a fingerprint of OTHER tenants' traffic against
 * that provider, and the pools count everyone's work at once. This console
 * authenticates as a tenant admin, so none of it may be here. The operator view
 * lives at `/management/admin/jobs/health` behind the superadmin token.
 *
 * What is left is what a tenant can act on: a degraded bit tied to work THIS
 * tenant actually has parked, and this tenant's own queue depth.
 */

/** Poll interval. This endpoint reads only live process state (the activity
 *  tracker and the scheduler's queues) and touches no storage, so polling it is
 *  cheap. It HAS to be polled: an outage's cooldown is measured in tens of
 *  seconds, and a one-shot snapshot taken before one would show "healthy" for
 *  the whole incident. */
const POLL_MS = 5000

/**
 * THREE states, never two. A poll can fail — an admin JWT expiring mid-session,
 * a network drop, a 5xx, a body we do not recognise, an older server without
 * the route.
 *
 * `unknown` exists because rendering a failed poll as "healthy" would make this
 * panel assert a false all-clear: an indicator vouching for exactly the silence
 * the 2026-09-02 incident was made of, with a green tick on top. Showing
 * nothing is better than that; saying "could not ask" is better still.
 */
type HealthFetchState =
  | { status: 'loading' }
  | { status: 'ok'; health: TenantJobHealth }
  | { status: 'unknown' }

/** A body that is not the shape we expect is "could not ask", not "healthy".
 *  Trusting a malformed one reads as `degraded: undefined` — falsy, i.e. an
 *  all-clear invented from a response we did not understand. */
function isTenantJobHealth(value: unknown): value is TenantJobHealth {
  if (typeof value !== 'object' || value === null) return false
  const health = value as Partial<TenantJobHealth>
  return typeof health.degraded === 'boolean' && typeof health.queued?.total === 'number'
}

export default function TenantJobHealthCard() {
  const [state, setState] = useState<HealthFetchState>({ status: 'loading' })

  useEffect(() => {
    let cancelled = false
    const fetchHealth = async () => {
      try {
        const response = await managementApi.getTenantJobHealth()
        if (cancelled) return
        if (response.success && isTenantJobHealth(response.data)) {
          setState({ status: 'ok', health: response.data })
        } else {
          setState({ status: 'unknown' })
        }
      } catch {
        // A failed poll DISCARDS the previous answer rather than keeping it.
        // Stale state is the subtle version of the same lie: processing can
        // stall during precisely the window we stopped being able to ask.
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

  // Could not ask. Amber icon AND amber heading, where the healthy line is grey
  // on grey — so a glance separates "nothing is wrong" from "we do not know",
  // without reading the words.
  if (state.status === 'unknown') {
    return (
      <GlassCard className="mb-4">
        <div className="flex items-center gap-2">
          <HelpCircle className="w-4 h-4 text-yellow-500/80" />
          <h3 className="text-sm font-medium text-yellow-500/80">
            Background processing status unavailable
          </h3>
          <span className="text-xs text-zinc-500">
            could not reach the health endpoint — this is not an all-clear
          </span>
        </div>
      </GlassCard>
    )
  }

  const { degraded, queued } = state.health
  const waiting = queued.total.toLocaleString()

  return (
    <GlassCard className="mb-4">
      <div className="flex items-center justify-between gap-3">
        <div className="flex items-center gap-2 min-w-0">
          {degraded ? (
            <AlertTriangle className="w-4 h-4 text-red-400" />
          ) : (
            <CheckCircle className="w-4 h-4 text-zinc-500" />
          )}
          <h3 className={`text-sm font-medium ${degraded ? 'text-white' : 'text-zinc-400'}`}>
            {degraded
              ? 'Background processing is paused — your jobs are parked, not failing'
              : 'Background processing healthy'}
          </h3>
        </div>
        {/* Depth only, and only this tenant's. A share-of-machine figure would
            be an inference about every other tenant's backlog. */}
        <span className="text-xs text-zinc-500 shrink-0">
          {queued.total === 0 ? 'nothing queued' : `${waiting} queued`}
        </span>
      </div>

      {degraded && (
        <div className="mt-2 text-[11px] text-zinc-400">
          An upstream your jobs depend on is not responding. Work is held and
          resumes automatically when it recovers — nothing is lost.
        </div>
      )}

      {queued.total > 0 && (
        <div className="mt-2 flex items-center gap-3 text-[10px] text-zinc-500">
          <span>{queued.high.toLocaleString()} high</span>
          <span>{queued.normal.toLocaleString()} normal</span>
          <span>{queued.low.toLocaleString()} low</span>
        </div>
      )}
    </GlassCard>
  )
}
