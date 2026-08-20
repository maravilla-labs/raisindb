// SPDX-License-Identifier: BSL-1.1

/**
 * Create a connector's preset of mounts in one go.
 *
 * Why this exists: a mount carries exactly one `write_config`, so a connector
 * whose resources need different write modes — Stripe's outbox of checkout
 * commands beside its read-only payments beside its two-way catalogue — is
 * unusable with fewer than one mount per resource. Each of those mounts is ten
 * values that only the adapter author knows (mapper path, `resource`, mode,
 * `command_node_types`, never `webhook`), and the failure mode for getting one
 * wrong is a mount that reports `ok` and writes nothing. Operators rebuilt that
 * set by hand, from a README, on every tenant, and could not reproduce it.
 *
 * The connector template ships the set as `mount_bundles`; this dialog asks
 * only for what is genuinely the operator's to decide — connection, workspace,
 * root folder, which entries — and mints the rest through `planBundle`.
 *
 * It creates ordinary `raisin:VirtualMount` nodes. Once created they owe the
 * bundle nothing: edit or delete them like any other mount.
 */

import { useEffect, useMemo, useState } from 'react'
import { AlertTriangle, Check, Layers, X } from 'lucide-react'
import {
  integrationsApi,
  normalizeMountPath,
  planBundle,
  type Integration,
  type MountBundle,
  type VirtualMount,
} from '../../api/integrations'
import { nodesApi } from '../../api/nodes'
import { workspacesApi } from '../../api/workspaces'
import { writeModeLabel } from '../../utils/mountStatus'

interface Props {
  repo: string
  /** Configured connector instances. Only those carrying bundles are offered. */
  integrations: Integration[]
  /**
   * Package-shipped templates, for the fallback: an instance minted before its
   * package learned about bundles has no `mount_bundles` of its own, and asking
   * the operator to recreate the connector (and re-consent) to get them is not
   * acceptable. The template of the same `provider_type` supplies them instead.
   */
  templates: Integration[]
  existingMounts: VirtualMount[]
  workspaces: string[]
  onClose: () => void
  onCreated: () => void
  onError: (title: string, message?: string) => void
  onSuccess: (title: string, message?: string) => void
}

const field =
  'w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-zinc-500 focus:outline-none focus:border-primary-500'
const labelCls = 'block text-sm text-zinc-300 mb-1'

/** The bundles an instance can use: its own, else its template's. */
export function bundlesFor(i: Integration, templates: Integration[]): MountBundle[] {
  if (i.mount_bundles?.length) return i.mount_bundles
  const t = templates.find((t) => t.provider_type === i.provider_type && t.mount_bundles?.length)
  return t?.mount_bundles || []
}

type RowOutcome = { key: string; status: 'created' | 'exists' | 'failed'; message?: string }

export default function AddMountBundleDialog({
  repo,
  integrations,
  templates,
  existingMounts,
  workspaces,
  onClose,
  onCreated,
  onError,
  onSuccess,
}: Props) {
  const candidates = useMemo(
    () => integrations.filter((i) => bundlesFor(i, templates).length > 0),
    [integrations, templates],
  )
  const [integration, setIntegration] = useState<Integration | null>(
    candidates.length === 1 ? candidates[0] : null,
  )
  const bundles = integration ? bundlesFor(integration, templates) : []
  const [bundleId, setBundleId] = useState<string>('')
  const bundle = bundles.find((b) => b.id === bundleId) || (bundles.length === 1 ? bundles[0] : null)

  const accounts = integration?.connected_accounts || []
  const [accountRef, setAccountRef] = useState('')
  const [workspace, setWorkspace] = useState('')
  const [root, setRoot] = useState('')
  const [keys, setKeys] = useState<Set<string>>(new Set())
  const [allowed, setAllowed] = useState<string[] | null>(null)
  const [allowedState, setAllowedState] = useState<'idle' | 'loading' | 'ok' | 'unknown'>('idle')
  const [rootState, setRootState] = useState<'unknown' | 'exists' | 'missing'>('unknown')
  const [creatingRoot, setCreatingRoot] = useState(false)
  const [saving, setSaving] = useState(false)
  const [outcomes, setOutcomes] = useState<RowOutcome[] | null>(null)

  // Seed the operator-facing defaults from the bundle, once per bundle pick.
  useEffect(() => {
    if (!bundle) return
    setWorkspace((w) => w || bundle.default_workspace || '')
    setRoot((r) => r || bundle.default_root || '')
    setKeys(new Set(bundle.mounts.filter((m) => m.default).map((m) => m.key)))
    setOutcomes(null)
  }, [bundle?.id])

  useEffect(() => {
    // One connection needs no choice; it is the same rule the mount editor uses.
    if (accounts.length === 1) setAccountRef(accounts[0].id)
  }, [integration?.path])

  // The workspace gate. `allowed_node_types` is enforced on every write, and a
  // workspace that rejects every item still finishes `outcome: "ok"` — then
  // flips `backfill_complete`, so the misconfiguration becomes a permanently
  // empty mount. Checking here is the difference between a red line now and a
  // silent failure later.
  useEffect(() => {
    if (!workspace) {
      setAllowed(null)
      setAllowedState('idle')
      return
    }
    let cancelled = false
    setAllowedState('loading')
    workspacesApi
      .get(repo, workspace)
      .then((ws) => {
        if (cancelled) return
        setAllowed(ws.allowed_node_types || [])
        setAllowedState('ok')
      })
      .catch(() => {
        if (cancelled) return
        setAllowed(null)
        setAllowedState('unknown')
      })
    return () => {
      cancelled = true
    }
  }, [repo, workspace])

  const normalizedRoot = normalizeMountPath(root)

  // Does the root folder exist? Same probe as the mount editor: only a 404
  // proves absence.
  useEffect(() => {
    if (!workspace || normalizedRoot === '/') {
      setRootState('unknown')
      return
    }
    let cancelled = false
    const timer = window.setTimeout(() => {
      nodesApi
        .getAtHead(repo, 'main', workspace, normalizedRoot)
        .then(() => !cancelled && setRootState('exists'))
        .catch((e: any) => !cancelled && setRootState(e?.status === 404 ? 'missing' : 'unknown'))
    }, 400)
    return () => {
      cancelled = true
      window.clearTimeout(timer)
    }
  }, [repo, workspace, normalizedRoot])

  async function createRoot() {
    if (!workspace || normalizedRoot === '/') return
    setCreatingRoot(true)
    try {
      const segments = normalizedRoot.split('/').filter(Boolean)
      const leaf = segments.pop() as string
      const parent = segments.length ? `/${segments.join('/')}` : '/'
      await nodesApi.create(repo, 'main', workspace, parent, {
        name: leaf,
        node_type: 'raisin:Folder',
        properties: { title: leaf },
      })
      setRootState('exists')
      onSuccess('Folder created', normalizedRoot)
    } catch (e: any) {
      onError('Could not create the folder', e?.message)
    } finally {
      setCreatingRoot(false)
    }
  }

  const planned = useMemo(
    () =>
      integration && bundle && workspace
        ? planBundle({
            integration,
            bundle,
            keys: [...keys],
            account_ref: accountRef || undefined,
            target_workspace: workspace,
            root: normalizedRoot,
          })
        : [],
    [integration, bundle, keys, accountRef, workspace, normalizedRoot],
  )

  // ---- pre-flight ----

  const selectedEntries = bundle ? bundle.mounts.filter((m) => keys.has(m.key)) : []

  /** Node types the selected entries materialise that the workspace refuses. */
  const missingTypes = useMemo(() => {
    if (!allowed) return []
    const need = new Set<string>(['raisin:Folder'])
    for (const e of selectedEntries) for (const t of e.node_types || []) need.add(t)
    return [...need].filter((t) => !allowed.includes(t))
  }, [allowed, selectedEntries])

  const duplicates = useMemo(() => {
    const taken = new Map(existingMounts.map((m) => [`${m.target_workspace}:${m.mount_path}`, m]))
    const names = new Set(existingMounts.map((m) => m.name))
    return planned.filter((p) => taken.has(`${p.target_workspace}:${p.mount_path}`) || names.has(p.name))
  }, [planned, existingMounts])

  const account = accounts.find((a) => a.id === accountRef)
  const needsReturnUrl =
    selectedEntries.some((e) => e.sync_config?.resource === 'checkout_sessions') &&
    // Per-connection config lives on the account entry (`connection_config_type`
    // values); the TS type does not name it, the server does.
    !((account as unknown as { config?: Record<string, unknown> } | undefined)?.config?.checkout_success_url)

  const accountMissing = accounts.length > 1 && !accountRef
  const canCreate =
    !!integration &&
    !!bundle &&
    !!workspace &&
    planned.length > 0 &&
    !accountMissing &&
    missingTypes.length === 0 &&
    allowedState !== 'loading' &&
    !saving

  async function create() {
    if (!canCreate || !bundle) return
    setSaving(true)
    const results: RowOutcome[] = []
    const dupSet = new Set(duplicates.map((d) => d.name))
    // Sequential, and no rollback. A mount that exists is a mount that syncs;
    // the row list below says exactly which ones landed and which to retry.
    for (const m of planned) {
      const key = m.name.slice(integration!.name.length + 1)
      if (dupSet.has(m.name)) {
        results.push({ key, status: 'exists' })
        continue
      }
      try {
        await integrationsApi.createMount(repo, m)
        results.push({ key, status: 'created' })
      } catch (e: any) {
        results.push({ key, status: 'failed', message: e?.message || 'request failed' })
      }
    }
    setOutcomes(results)
    setSaving(false)
    const created = results.filter((r) => r.status === 'created').length
    const failed = results.filter((r) => r.status === 'failed').length
    if (created > 0) onCreated()
    if (failed === 0) {
      onSuccess(
        `${created} mount${created === 1 ? '' : 's'} created`,
        'Check that each first walk reports written > 0, not just ok.',
      )
    } else {
      onError(`${failed} of ${results.length} mounts failed`, 'See the list for the reason on each.')
    }
  }

  const pathFor = (subpath: string) => normalizeMountPath(`${normalizedRoot}/${subpath}`)

  return (
    <div className="fixed inset-0 bg-black/50 backdrop-blur-sm flex items-center justify-center z-50 p-4 overscroll-none">
      <div className="bg-zinc-900 border border-white/10 rounded-xl shadow-2xl max-w-3xl w-full max-h-[90vh] overflow-y-auto overscroll-contain">
        <div className="flex items-center justify-between p-6 border-b border-white/10">
          <h2 className="text-2xl font-bold text-white">Add mount bundle</h2>
          <button onClick={onClose} className="p-2 hover:bg-white/10 rounded-lg transition-colors">
            <X className="w-5 h-5 text-white/60" />
          </button>
        </div>

        <div className="p-6 space-y-5">
          {candidates.length === 0 ? (
            <p className="text-zinc-400 text-sm">
              None of the configured connectors ships a mount bundle. A bundle is declared by the
              connector&rsquo;s package; connectors without one are set up mount by mount.
            </p>
          ) : (
            <>
              {candidates.length > 1 && (
                <div>
                  <label className={labelCls}>Connector *</label>
                  <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
                    {candidates.map((i) => {
                      const selected = integration?.path === i.path
                      return (
                        <button
                          type="button"
                          key={i.path || i.name}
                          onClick={() => {
                            setIntegration(i)
                            setBundleId('')
                            setAccountRef('')
                          }}
                          className={`flex items-center gap-3 px-4 py-3 rounded-lg border text-left transition-colors ${
                            selected
                              ? 'bg-primary-500/10 border-primary-500/50'
                              : 'bg-white/5 border-white/10 hover:bg-white/10'
                          }`}
                        >
                          <Layers className="w-5 h-5 text-sky-400 flex-shrink-0" />
                          <div className="min-w-0">
                            <div className="text-white text-sm truncate">{i.title || i.name}</div>
                            <div className="text-zinc-500 text-xs truncate">{i.provider_type}</div>
                          </div>
                        </button>
                      )
                    })}
                  </div>
                </div>
              )}

              {integration && bundles.length > 1 && (
                <div>
                  <label className={labelCls}>Bundle *</label>
                  <select className={field} value={bundle?.id || ''} onChange={(e) => setBundleId(e.target.value)}>
                    <option value="">Choose…</option>
                    {bundles.map((b) => (
                      <option key={b.id} value={b.id}>
                        {b.title}
                      </option>
                    ))}
                  </select>
                </div>
              )}

              {integration && bundle && (
                <>
                  {bundle.description && <p className="text-sm text-zinc-400">{bundle.description}</p>}

                  <div className="grid grid-cols-1 sm:grid-cols-3 gap-4">
                    <div>
                      <label className={labelCls}>Connection {accounts.length > 1 ? '*' : ''}</label>
                      {accounts.length === 0 ? (
                        <p className="text-xs text-amber-400">
                          This connector has no connected account yet. The mounts can be created, but
                          will not sync until one is connected.
                        </p>
                      ) : (
                        <select className={field} value={accountRef} onChange={(e) => setAccountRef(e.target.value)}>
                          {accounts.length > 1 && <option value="">Choose…</option>}
                          {accounts.map((a) => (
                            <option key={a.id} value={a.id}>
                              {a.label || a.subject || a.id}
                            </option>
                          ))}
                        </select>
                      )}
                    </div>
                    <div>
                      <label className={labelCls}>Target workspace *</label>
                      <select className={field} value={workspace} onChange={(e) => setWorkspace(e.target.value)}>
                        <option value="">Choose…</option>
                        {workspaces.map((w) => (
                          <option key={w} value={w}>
                            {w}
                          </option>
                        ))}
                      </select>
                      {bundle.default_workspace && workspace !== bundle.default_workspace && (
                        <p className="mt-1 text-xs text-zinc-500">
                          The connector suggests <span className="font-mono">{bundle.default_workspace}</span>.
                        </p>
                      )}
                    </div>
                    <div>
                      <label className={labelCls}>Root folder *</label>
                      <input className={field} value={root} onChange={(e) => setRoot(e.target.value)} placeholder={bundle.default_root || '/'} />
                      {rootState === 'missing' && (
                        <p className="mt-1 text-xs text-amber-400">
                          <span className="font-mono">{normalizedRoot}</span> does not exist yet.{' '}
                          <button type="button" onClick={createRoot} disabled={creatingRoot} className="underline hover:text-amber-300 disabled:opacity-50">
                            {creatingRoot ? 'Creating…' : 'Create it'}
                          </button>
                        </p>
                      )}
                      {rootState === 'exists' && <p className="mt-1 text-xs text-green-400">This folder exists.</p>}
                      {normalizedRoot === '/' && (
                        <p className="mt-1 text-xs text-amber-400">Give the bundle its own folder rather than the workspace root.</p>
                      )}
                    </div>
                  </div>

                  <div>
                    <label className={labelCls}>Mounts</label>
                    <ul className="divide-y divide-white/5 border border-white/10 rounded-lg overflow-hidden">
                      {bundle.mounts.map((e) => {
                        const on = keys.has(e.key)
                        const outcome = outcomes?.find((o) => o.key === e.key)
                        const dup = duplicates.some((d) => d.name === `${integration.name}-${e.key}`)
                        return (
                          <li key={e.key} className={`flex items-start gap-3 px-4 py-3 ${on ? 'bg-white/[0.03]' : ''}`}>
                            <input
                              type="checkbox"
                              className="mt-1"
                              checked={on}
                              disabled={!!outcomes}
                              onChange={(ev) => {
                                const next = new Set(keys)
                                if (ev.target.checked) next.add(e.key)
                                else next.delete(e.key)
                                setKeys(next)
                              }}
                            />
                            <div className="min-w-0 flex-1">
                              <div className="flex items-center gap-2 flex-wrap">
                                <span className="text-white text-sm">{e.title}</span>
                                <span className="text-[10px] uppercase tracking-wide px-1.5 py-0.5 rounded bg-white/10 text-zinc-300">
                                  {writeModeLabel(e.write_config)}
                                </span>
                                {e.required_by?.map((r) => (
                                  <span key={r} className="text-[10px] px-1.5 py-0.5 rounded bg-sky-500/10 text-sky-300">
                                    needed by {r}
                                  </span>
                                ))}
                              </div>
                              <div className="text-xs text-zinc-500 font-mono mt-0.5">
                                {workspace || '<workspace>'}:{pathFor(e.subpath)}
                                {e.sync_config?.resource ? ` · ${e.sync_config.resource}` : ''}
                              </div>
                              {on && dup && !outcome && (
                                <div className="text-xs text-amber-400 mt-1">
                                  A mount with this name or path already exists — it will be skipped.
                                </div>
                              )}
                              {outcome && (
                                <div
                                  className={`text-xs mt-1 flex items-center gap-1 ${
                                    outcome.status === 'created' ? 'text-green-400' : outcome.status === 'exists' ? 'text-zinc-400' : 'text-red-400'
                                  }`}
                                >
                                  {outcome.status === 'created' && <Check className="w-3 h-3" />}
                                  {outcome.status === 'failed' && <AlertTriangle className="w-3 h-3" />}
                                  {outcome.status === 'created' ? 'created' : outcome.status === 'exists' ? 'already existed, skipped' : outcome.message}
                                </div>
                              )}
                            </div>
                          </li>
                        )
                      })}
                    </ul>
                  </div>

                  {/* Pre-flight. Each of these is a failure that would otherwise
                      only show up as a mount that looks fine and does nothing. */}
                  {workspace && allowedState === 'unknown' && (
                    <p className="text-xs text-amber-400">
                      Could not read the workspace&rsquo;s allowed node types; the gate check is skipped.
                    </p>
                  )}
                  {missingTypes.length > 0 && (
                    <div className="text-xs text-red-400 flex items-start gap-2 bg-red-500/5 border border-red-500/20 rounded-lg p-3">
                      <AlertTriangle className="w-4 h-4 flex-shrink-0" />
                      <div>
                        <span className="font-mono">{workspace}</span> does not allow{' '}
                        {missingTypes.map((t) => (
                          <span key={t} className="font-mono">
                            {t}{' '}
                          </span>
                        ))}
                        — every item would be rejected while the sync reports <span className="font-mono">ok</span>. Install or
                        reinstall the connector&rsquo;s package (its workspace patches add these), or pick another workspace.
                      </div>
                    </div>
                  )}
                  {needsReturnUrl && (
                    <p className="text-xs text-amber-400">
                      The selected connection has no <span className="font-mono">checkout_success_url</span>. Checkout sessions
                      will fail to create unless each one carries its own <span className="font-mono">success_url</span>. Set it
                      on the connection after this.
                    </p>
                  )}
                  {accountMissing && <p className="text-xs text-amber-400">Choose which connection these mounts sync through.</p>}
                </>
              )}
            </>
          )}
        </div>

        <div className="flex justify-end gap-2 p-6 border-t border-white/10">
          <button onClick={onClose} className="px-4 py-2 text-zinc-300 hover:bg-white/10 rounded-lg transition-colors">
            {outcomes ? 'Close' : 'Cancel'}
          </button>
          {!outcomes && (
            <button
              onClick={create}
              disabled={!canCreate}
              className="px-4 py-2 bg-primary-500 hover:bg-primary-600 disabled:opacity-50 disabled:cursor-not-allowed text-white rounded-lg transition-colors"
            >
              {saving ? 'Creating…' : `Create ${planned.length} mount${planned.length === 1 ? '' : 's'}`}
            </button>
          )}
        </div>
      </div>
    </div>
  )
}
