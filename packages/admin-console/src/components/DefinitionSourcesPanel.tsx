import { useEffect, useState } from 'react'
import {
  Layers,
  RefreshCw,
  Download,
  CloudDownload,
  HardDrive,
  Loader2,
  ChevronDown,
  ChevronRight,
} from 'lucide-react'
import GlassCard from './GlassCard'
import {
  systemDefinitionsApi,
  SystemDefinitionsResponse,
  RegistryInfo,
  RegistryEntry,
} from '../api/system-definitions'

interface Props {
  /** Called after a reload or fetch changes what the server offers, so the
   *  page can re-check pending updates for the current repository. */
  onStackChanged?: () => void
  onError: (message: string) => void
  onSuccess: (message: string) => void
}

/**
 * Where this server's built-in definitions come from, and the controls for
 * changing that without a redeploy: reload the on-disk overlay, or pull
 * artifacts from a configured registry into it.
 *
 * Nothing here writes to a repository — that is still the (deliberate) second
 * step through system updates.
 */
export default function DefinitionSourcesPanel({
  onStackChanged,
  onError,
  onSuccess,
}: Props) {
  const [stack, setStack] = useState<SystemDefinitionsResponse | null>(null)
  const [registries, setRegistries] = useState<RegistryInfo[]>([])
  const [catalog, setCatalog] = useState<Record<string, RegistryEntry[]>>({})
  const [openRegistry, setOpenRegistry] = useState<string | null>(null)
  const [busy, setBusy] = useState<string | null>(null)
  const [showOverrides, setShowOverrides] = useState(false)

  useEffect(() => {
    load()
  }, [])

  async function load() {
    try {
      const [defs, regs] = await Promise.all([
        systemDefinitionsApi.get(),
        systemDefinitionsApi.listRegistries(),
      ])
      setStack(defs)
      setRegistries(regs)
    } catch (err) {
      console.error('Failed to load definition sources:', err)
    }
  }

  async function reload() {
    try {
      setBusy('reload')
      const defs = await systemDefinitionsApi.reload()
      setStack(defs)
      onSuccess('Definition overlay reloaded')
      onStackChanged?.()
    } catch (err: any) {
      onError(err.message || 'Failed to reload definitions')
    } finally {
      setBusy(null)
    }
  }

  async function toggleRegistry(name: string) {
    if (openRegistry === name) {
      setOpenRegistry(null)
      return
    }
    setOpenRegistry(name)
    if (catalog[name]) return
    try {
      setBusy(`catalog:${name}`)
      const entries = await systemDefinitionsApi.getCatalog(name)
      setCatalog((prev) => ({ ...prev, [name]: entries }))
    } catch (err: any) {
      onError(err.message || `Failed to read registry '${name}'`)
      setOpenRegistry(null)
    } finally {
      setBusy(null)
    }
  }

  async function fetchEntry(registry: string, resource?: string) {
    try {
      setBusy(`fetch:${registry}:${resource ?? '*'}`)
      const res = await systemDefinitionsApi.fetch(
        registry,
        resource ? [resource] : []
      )
      onSuccess(res.message)
      await load()
      onStackChanged?.()
    } catch (err: any) {
      onError(err.message || 'Fetch failed')
    } finally {
      setBusy(null)
    }
  }

  if (!stack) return null

  const overridden = stack.definitions.filter((d) => d.shadowed.length > 0)

  return (
    <GlassCard className="space-y-4">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-3">
          <Layers className="w-5 h-5 text-amber-400" />
          <div>
            <h2 className="text-lg font-semibold text-white">
              Definition Sources
            </h2>
            <p className="text-sm text-white/60">
              Where this server's built-in definitions come from
            </p>
          </div>
        </div>
        <button
          onClick={reload}
          disabled={busy === 'reload'}
          className="flex items-center gap-2 px-3 py-1.5 text-sm text-white/80 hover:text-white bg-white/5 hover:bg-white/10 rounded-lg transition-colors"
          title="Re-read the overlay directory"
        >
          {busy === 'reload' ? (
            <Loader2 className="w-4 h-4 animate-spin" />
          ) : (
            <RefreshCw className="w-4 h-4" />
          )}
          Reload overlay
        </button>
      </div>

      <div className="flex flex-wrap items-center gap-2">
        {stack.layers.map((layer, i) => (
          <span
            key={layer}
            className="px-2 py-0.5 text-xs rounded bg-white/10 text-white/80"
            title={
              i === 0
                ? 'Lowest precedence — compiled into the binary'
                : 'Overrides layers below it'
            }
          >
            {layer}
          </span>
        ))}
        <span className="text-xs text-white/40">
          (later layers override earlier ones by name)
        </span>
      </div>

      <div className="flex items-start gap-2 text-sm text-white/60">
        <HardDrive className="w-4 h-4 mt-0.5 flex-shrink-0" />
        <div>
          <span className="font-mono text-white/80">{stack.overlay_dir}</span>
          {!stack.overlay_present && (
            <span className="ml-2 text-white/40">
              (not present — using embedded definitions only)
            </span>
          )}
          <div className="text-white/40 mt-0.5">
            Startup auto-apply: {stack.auto_apply}
          </div>
        </div>
      </div>

      {overridden.length > 0 && (
        <div>
          <button
            onClick={() => setShowOverrides((v) => !v)}
            className="flex items-center gap-2 text-sm text-white/70 hover:text-white transition-colors"
          >
            {showOverrides ? (
              <ChevronDown className="w-4 h-4" />
            ) : (
              <ChevronRight className="w-4 h-4" />
            )}
            {overridden.length} definition
            {overridden.length !== 1 ? 's' : ''} overridden
          </button>
          {showOverrides && (
            <div className="mt-2 space-y-1">
              {overridden.map((d) => (
                <div
                  key={d.name}
                  className="flex items-center gap-2 text-sm px-3 py-1.5 rounded bg-white/5"
                >
                  <span className="text-white">{d.name}</span>
                  <span className="text-white/40">from</span>
                  <span className="px-2 py-0.5 text-xs rounded bg-amber-500/20 text-amber-200">
                    {d.layer}
                  </span>
                  <span className="text-white/40 text-xs">
                    shadows {d.shadowed.join(', ')}
                  </span>
                </div>
              ))}
            </div>
          )}
        </div>
      )}

      {registries.length > 0 && (
        <div className="border-t border-white/10 pt-4 space-y-2">
          <div className="flex items-center gap-2 text-sm text-white/70">
            <CloudDownload className="w-4 h-4" />
            Registries
          </div>
          {registries.map((reg) => (
            <div key={reg.name} className="rounded-lg bg-white/5">
              <div className="flex items-center gap-3 p-3">
                <button
                  onClick={() => reg.enabled && toggleRegistry(reg.name)}
                  disabled={!reg.enabled}
                  className="text-white/60 hover:text-white disabled:opacity-30 transition-colors"
                >
                  {openRegistry === reg.name ? (
                    <ChevronDown className="w-4 h-4" />
                  ) : (
                    <ChevronRight className="w-4 h-4" />
                  )}
                </button>
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2">
                    <span className="text-white">{reg.name}</span>
                    {!reg.enabled && (
                      <span className="px-2 py-0.5 text-xs bg-white/10 text-white/50 rounded">
                        disabled
                      </span>
                    )}
                  </div>
                  <div className="text-xs text-white/40 truncate font-mono">
                    {reg.url}
                  </div>
                </div>
                {reg.enabled && (
                  <button
                    onClick={() => fetchEntry(reg.name)}
                    disabled={busy?.startsWith('fetch:')}
                    className="flex items-center gap-1.5 px-3 py-1.5 text-sm bg-white/5 hover:bg-white/10 text-white/80 rounded-lg transition-colors"
                  >
                    <Download className="w-4 h-4" />
                    Fetch all
                  </button>
                )}
              </div>

              {openRegistry === reg.name && (
                <div className="border-t border-white/10 p-3 space-y-1">
                  {busy === `catalog:${reg.name}` && (
                    <Loader2 className="w-4 h-4 animate-spin text-white/60" />
                  )}
                  {(catalog[reg.name] ?? []).map((entry) => (
                    <div
                      key={`${entry.kind}:${entry.name}`}
                      className="flex items-center gap-3 text-sm px-2 py-1.5 rounded hover:bg-white/5"
                    >
                      <span className="px-2 py-0.5 text-xs rounded bg-white/10 text-white/60">
                        {entry.kind}
                      </span>
                      <span className="text-white flex-1">{entry.name}</span>
                      {entry.version && (
                        <span className="text-white/40 text-xs">
                          v{entry.version}
                        </span>
                      )}
                      <button
                        onClick={() => fetchEntry(reg.name, entry.name)}
                        disabled={busy?.startsWith('fetch:')}
                        className="text-white/60 hover:text-white transition-colors"
                        title="Download into the overlay"
                      >
                        <Download className="w-4 h-4" />
                      </button>
                    </div>
                  ))}
                  {catalog[reg.name]?.length === 0 && (
                    <div className="text-sm text-white/40">
                      Registry catalog is empty.
                    </div>
                  )}
                </div>
              )}
            </div>
          ))}
          <p className="text-xs text-white/40">
            Fetching downloads into the overlay directory and verifies each
            artifact's declared SHA256. Nothing is written to a repository until
            you apply the resulting pending updates.
          </p>
        </div>
      )}
    </GlassCard>
  )
}
