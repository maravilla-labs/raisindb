// SPDX-License-Identifier: BSL-1.1

import { useEffect, useRef, useState } from 'react'
import { Activity, CheckCircle2, XCircle, Loader2, Clock } from 'lucide-react'
import {
  integrationsApi,
  deriveStages,
  type TestConnectionRequest,
  type TestConnectionResult,
  type TestConnectionStages,
} from '../../api/integrations'
import CapabilityChips from './CapabilityChips'

interface TestConnectionPanelProps {
  repo: string
  request: TestConnectionRequest
  /** Disabled reason; when set the button is disabled and the text explains why. */
  disabledReason?: string
  /** Run the test once automatically when the panel mounts. */
  autoRun?: boolean
  /** Called after a completed test with the fresh result (e.g. to refresh caches). */
  onTested?: (result: TestConnectionResult) => void
  onError: (title: string, message?: string) => void
}

const STAGE_LABELS: Array<{ key: keyof TestConnectionStages; label: string }> = [
  { key: 'adapter_loaded', label: 'Adapter loaded' },
  { key: 'auth_valid', label: 'Auth valid' },
  { key: 'listing_succeeded', label: 'Listing succeeded' },
]

/**
 * "Test connection" button plus a diagnostic results panel. Runs the server
 * probe (capabilities + bounded list), then renders which of the three stages
 * passed, latency, the resolved capability chips, a probe sample, and — on
 * failure — the error code and message. This is the connector debugging surface.
 */
export default function TestConnectionPanel({
  repo,
  request,
  disabledReason,
  autoRun,
  onTested,
  onError,
}: TestConnectionPanelProps) {
  const [testing, setTesting] = useState(false)
  const [result, setResult] = useState<TestConnectionResult | null>(null)
  const autoRan = useRef(false)

  async function runTest() {
    setTesting(true)
    try {
      const res = await integrationsApi.testConnection(repo, request)
      setResult(res)
      onTested?.(res)
    } catch (e: any) {
      // Transport-level failure (endpoint unreachable, 5xx, etc.) — synthesize a
      // result so the panel still shows a useful diagnostic.
      const synthetic: TestConnectionResult = {
        ok: false,
        error: { code: e?.code, message: e?.message || 'Connection test failed' },
      }
      setResult(synthetic)
      onError('Test connection failed', e?.message)
    } finally {
      setTesting(false)
    }
  }

  useEffect(() => {
    if (autoRun && !autoRan.current && !disabledReason) {
      autoRan.current = true
      void runTest()
    }
  }, [autoRun, disabledReason])

  return (
    <div className="space-y-3">
      <button
        type="button"
        onClick={runTest}
        disabled={testing || !!disabledReason}
        title={disabledReason || 'Probe the connector: capabilities + a bounded list'}
        className="inline-flex items-center gap-2 px-3 py-1.5 bg-white/5 hover:bg-white/10 border border-white/10 text-white text-sm rounded-lg transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
      >
        {testing ? <Loader2 className="w-4 h-4 animate-spin" /> : <Activity className="w-4 h-4" />}
        Test connection
      </button>
      {disabledReason && <p className="text-xs text-zinc-500">{disabledReason}</p>}

      {result && (
        <div
          className={`rounded-lg border p-3 space-y-3 ${
            result.ok ? 'border-green-500/30 bg-green-500/5' : 'border-red-500/30 bg-red-500/5'
          }`}
        >
          <div className="flex items-center gap-2">
            {result.ok ? (
              <span className="inline-flex items-center gap-1 text-green-400 text-sm font-medium">
                <CheckCircle2 className="w-4 h-4" /> Connection OK
              </span>
            ) : (
              <span className="inline-flex items-center gap-1 text-red-400 text-sm font-medium">
                <XCircle className="w-4 h-4" /> Connection failed
              </span>
            )}
            {typeof result.latency_ms === 'number' && (
              <span className="inline-flex items-center gap-1 text-xs text-zinc-400">
                <Clock className="w-3 h-3" /> {result.latency_ms} ms
              </span>
            )}
            {result.auth && (
              <span
                className="inline-flex items-center gap-1 text-xs text-zinc-400"
                title="Credential state reported by the server"
              >
                auth: {result.auth}
              </span>
            )}
          </div>

          {/* Stage breakdown — the first failing stage localizes the problem.
              Derived client-side: the server reports auth/probe/error, not stages. */}
          <ol className="space-y-1">
            {STAGE_LABELS.map(({ key, label }) => {
              const passed = deriveStages(result)[key]
              return (
                <li key={key} className="flex items-center gap-2 text-xs">
                  {passed ? (
                    <CheckCircle2 className="w-3.5 h-3.5 text-green-400 flex-shrink-0" />
                  ) : (
                    <XCircle className="w-3.5 h-3.5 text-red-400 flex-shrink-0" />
                  )}
                  <span className={passed ? 'text-zinc-300' : 'text-zinc-500'}>{label}</span>
                </li>
              )
            })}
          </ol>

          {result.error && (
            <div className="rounded border border-red-500/30 bg-red-500/10 px-3 py-2">
              {result.error.code && (
                <div className="text-xs font-mono text-red-300">{result.error.code}</div>
              )}
              <div className="text-xs text-red-200 break-words">{result.error.message}</div>
            </div>
          )}

          {result.capabilities && (
            <div>
              <div className="text-[10px] uppercase tracking-wider text-zinc-500 mb-1">Capabilities</div>
              <CapabilityChips capabilities={result.capabilities} />
            </div>
          )}

          {result.probe && result.probe.sample.length > 0 && (
            <div>
              <div className="text-[10px] uppercase tracking-wider text-zinc-500 mb-1">
                Probe sample ({result.probe.sample.length} of {result.probe.items_seen})
              </div>
              <ul className="flex flex-wrap gap-1">
                {result.probe.sample.slice(0, 12).map((name, i) => (
                  <li
                    key={`${name}-${i}`}
                    className="px-1.5 py-0.5 rounded bg-white/5 border border-white/10 text-[11px] text-zinc-300 truncate max-w-[160px]"
                    title={name}
                  >
                    {name}
                  </li>
                ))}
              </ul>
            </div>
          )}
        </div>
      )}
    </div>
  )
}
