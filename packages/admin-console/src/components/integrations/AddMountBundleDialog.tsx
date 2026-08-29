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
 *
 * TWO THINGS A BUNDLE CAN DO THAT ONE FORM CANNOT (schema v5), and both are
 * about the same connector — Microsoft 365, whose mail is a mailbox and whose
 * files are assets:
 *
 *  - An entry may name its OWN `target_workspace` (and root). So a bundle can
 *    put mail in `workplace` and drive files in `assets`, where they show up as
 *    raisin:Assets beside every other asset instead of in a parallel library.
 *    Every gate below is therefore per DESTINATION, not per dialog: one
 *    `allowed_node_types` verdict for a bundle spanning two workspaces would be
 *    a verdict about the wrong one.
 *  - A bundle may declare PROMPTS: the values only the operator knows (which
 *    mailbox, which SharePoint site, which drive). Before them, any connector
 *    needing one could not be a bundle at all.
 */

import { useEffect, useMemo, useState } from 'react'
import { AlertTriangle, Check, Layers, X } from 'lucide-react'
import {
  activePrompts,
  integrationsApi,
  missingPromptKeys,
  normalizeMountPath,
  planBundle,
  type Integration,
  type MountBundle,
  type MountBundleEntry,
  type MountBundlePrompt,
  type SyncConfig,
  type VirtualMount,
} from '../../api/integrations'
import { nodesApi } from '../../api/nodes'
import { workspacesApi } from '../../api/workspaces'
import { writeModeLabel } from '../../utils/mountStatus'
import RemotePicker from './RemotePicker'

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

/**
 * One DESTINATION: a workspace plus the root the entries going there hang
 * under. A v4 bundle has exactly one; a bundle whose entries name their own
 * `target_workspace` has several, and each is gated separately.
 */
interface Destination {
  workspace: string
  root: string
  entries: MountBundleEntry[]
}

type RootState = 'unknown' | 'exists' | 'missing'

/** Node types a destination materialises, including the folders along its path. */
function neededTypes(dest: Destination): string[] {
  const need = new Set<string>(['raisin:Folder'])
  for (const e of dest.entries) for (const t of e.node_types || []) need.add(t)
  return [...need]
}

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
  /** Prompt answers, by prompt key. `''` means unanswered — see `planBundle`. */
  const [answers, setAnswers] = useState<Record<string, string>>({})
  const [pickerFor, setPickerFor] = useState<MountBundlePrompt | null>(null)
  /** `allowed_node_types` per destination workspace; `null` = could not be read. */
  const [allowedByWs, setAllowedByWs] = useState<Record<string, string[] | null>>({})
  const [gateLoading, setGateLoading] = useState(false)
  const [rootStates, setRootStates] = useState<Record<string, RootState>>({})
  const [creatingRoot, setCreatingRoot] = useState('')
  const [saving, setSaving] = useState(false)
  const [outcomes, setOutcomes] = useState<RowOutcome[] | null>(null)

  // Seed the operator-facing defaults from the bundle, once per bundle pick.
  useEffect(() => {
    if (!bundle) return
    setWorkspace((w) => w || bundle.default_workspace || '')
    setRoot((r) => r || bundle.default_root || '')
    setKeys(new Set(bundle.mounts.filter((m) => m.default).map((m) => m.key)))
    setAnswers({})
    setOutcomes(null)
  }, [bundle?.id])

  useEffect(() => {
    // One connection needs no choice; it is the same rule the mount editor uses.
    if (accounts.length === 1) setAccountRef(accounts[0].id)
  }, [integration?.path])

  const normalizedRoot = normalizeMountPath(root)

  const selectedEntries = bundle ? bundle.mounts.filter((m) => keys.has(m.key)) : []

  // Where the selected entries actually land. An entry may override both the
  // workspace and the root, so this is a list, and everything downstream —
  // the type gate, the folder probe, the path shown on each row — is computed
  // per destination rather than once for the dialog.
  const destinations = useMemo<Destination[]>(() => {
    if (!workspace) return []
    const byKey = new Map<string, Destination>()
    for (const e of selectedEntries) {
      const ws = e.target_workspace || workspace
      const r = normalizeMountPath(e.root_override || normalizedRoot)
      const k = `${ws}:${r}`
      const existing = byKey.get(k)
      if (existing) existing.entries.push(e)
      else byKey.set(k, { workspace: ws, root: r, entries: [e] })
    }
    return [...byKey.values()]
  }, [selectedEntries, workspace, normalizedRoot])

  const gateWorkspaces = useMemo(
    () => [...new Set(destinations.map((d) => d.workspace))].sort().join(','),
    [destinations],
  )

  // The workspace gate. `allowed_node_types` is enforced on every write, and a
  // workspace that rejects every item still finishes `outcome: "ok"` — then
  // flips `backfill_complete`, so the misconfiguration becomes a permanently
  // empty mount. Checking here is the difference between a red line now and a
  // silent failure later.
  useEffect(() => {
    const list = gateWorkspaces ? gateWorkspaces.split(',') : []
    if (!list.length) {
      setAllowedByWs({})
      setGateLoading(false)
      return
    }
    let cancelled = false
    setGateLoading(true)
    Promise.all(
      list.map((w) =>
        workspacesApi
          .get(repo, w)
          .then((ws) => [w, ws.allowed_node_types || []] as const)
          .catch(() => [w, null] as const),
      ),
    ).then((pairs) => {
      if (cancelled) return
      setAllowedByWs(Object.fromEntries(pairs))
      setGateLoading(false)
    })
    return () => {
      cancelled = true
    }
  }, [repo, gateWorkspaces])

  const destKey = useMemo(
    () => destinations.map((d) => `${d.workspace}:${d.root}`).sort().join('|'),
    [destinations],
  )

  // Does each destination's root folder exist? Same probe as the mount editor:
  // only a 404 proves absence.
  useEffect(() => {
    const list = destKey ? destKey.split('|') : []
    const probes = list.filter((k) => !k.endsWith(':/'))
    if (!probes.length) {
      setRootStates({})
      return
    }
    let cancelled = false
    const timer = window.setTimeout(() => {
      Promise.all(
        probes.map((k) => {
          const idx = k.indexOf(':')
          const ws = k.slice(0, idx)
          const path = k.slice(idx + 1)
          return nodesApi
            .getAtHead(repo, 'main', ws, path)
            .then(() => [k, 'exists' as RootState] as const)
            .catch((e: any) => [k, (e?.status === 404 ? 'missing' : 'unknown') as RootState] as const)
        }),
      ).then((pairs) => {
        if (!cancelled) setRootStates(Object.fromEntries(pairs))
      })
    }, 400)
    return () => {
      cancelled = true
      window.clearTimeout(timer)
    }
  }, [repo, destKey])

  async function createRoot(ws: string, path: string) {
    if (path === '/') return
    const key = `${ws}:${path}`
    setCreatingRoot(key)
    try {
      const segments = path.split('/').filter(Boolean)
      const leaf = segments.pop() as string
      const parent = segments.length ? `/${segments.join('/')}` : '/'
      await nodesApi.create(repo, 'main', ws, parent, {
        name: leaf,
        node_type: 'raisin:Folder',
        properties: { title: leaf },
      })
      setRootStates((prev) => ({ ...prev, [key]: 'exists' }))
      onSuccess('Folder created', `${ws}:${path}`)
    } catch (e: any) {
      onError('Could not create the folder', e?.message)
    } finally {
      setCreatingRoot('')
    }
  }

  // ---- prompts ----

  const prompts = useMemo(
    () => (bundle ? activePrompts(bundle, [...keys], answers) : []),
    [bundle, keys, answers],
  )
  const missingAnswers = useMemo(
    () => (bundle ? missingPromptKeys(bundle, [...keys], answers) : []),
    [bundle, keys, answers],
  )

  // What the RemotePicker browses THROUGH. A SharePoint library listing needs
  // the site the operator just picked, and that answer is not on a mount yet —
  // so the answers collected so far are projected into a sync_config for it.
  const promptSyncConfig = useMemo(() => {
    const cfg: Record<string, unknown> = {}
    for (const p of prompts) {
      const v = (answers[p.key] || '').trim()
      if (!v) continue
      if (p.target.startsWith('sync_config.')) cfg[p.target.slice('sync_config.'.length)] = v
    }
    return cfg as SyncConfig
  }, [prompts, answers])

  // planBundle THROWS on a bundle whose prompt names a target outside the
  // closed set. Caught here so a bad template is a red line in the dialog
  // rather than a blank modal.
  const plan = useMemo(() => {
    if (!(integration && bundle && workspace)) {
      return { mounts: [] as VirtualMount[], error: null as string | null }
    }
    try {
      return {
        mounts: planBundle({
          integration,
          bundle,
          keys: [...keys],
          account_ref: accountRef || undefined,
          target_workspace: workspace,
          root: normalizedRoot,
          answers,
        }),
        error: null as string | null,
      }
    } catch (e: any) {
      return { mounts: [] as VirtualMount[], error: e?.message || 'this bundle could not be planned' }
    }
  }, [integration, bundle, keys, accountRef, workspace, normalizedRoot, answers])
  const planned = plan.mounts

  // ---- pre-flight ----

  /** Node types a destination materialises that its workspace refuses. */
  function missingTypesFor(dest: Destination): string[] {
    const allowed = allowedByWs[dest.workspace]
    if (!allowed) return []
    return neededTypes(dest).filter((t) => !allowed.includes(t))
  }

  const blockedDestinations = destinations.filter((d) => missingTypesFor(d).length > 0)
  // Every destination has an answer (a list, or `null` for "could not read").
  // Without this, the render between choosing entries and the gate resolving
  // would offer a Create button that has checked nothing.
  const gateResolved = destinations.every((d) => d.workspace in allowedByWs)

  const duplicates = useMemo(() => {
    const taken = new Map(existingMounts.map((m) => [`${m.target_workspace}:${m.mount_path}`, m]))
    const names = new Set(existingMounts.map((m) => m.name))
    return planned.filter((p) => taken.has(`${p.target_workspace}:${p.mount_path}`) || names.has(p.name))
  }, [planned, existingMounts])

  const account = accounts.find((a) => a.id === accountRef)
  // The one provider-specific check left, and it stays because a PROMPT cannot
  // express it: `checkout_success_url` is read off the CONNECTION, which the
  // dialog can inspect but `planBundle` — pure, and given only the bundle and
  // the operator's choices — cannot. Everything else Stripe used to need
  // special-cased here is now data on the bundle.
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
    !plan.error &&
    !accountMissing &&
    missingAnswers.length === 0 &&
    blockedDestinations.length === 0 &&
    gateResolved &&
    !gateLoading &&
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

  const rootFor = (e: MountBundleEntry) => normalizeMountPath(e.root_override || normalizedRoot)
  const pathFor = (e: MountBundleEntry) => normalizeMountPath(`${rootFor(e)}/${e.subpath}`)
  const wsFor = (e: MountBundleEntry) => e.target_workspace || workspace

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
                      {normalizedRoot === '/' && (
                        <p className="mt-1 text-xs text-amber-400">Give the bundle its own folder rather than the workspace root.</p>
                      )}
                      {bundle.mounts.some((m) => m.target_workspace) && (
                        <p className="mt-1 text-xs text-zinc-500">
                          Some entries in this bundle land elsewhere on purpose — see Destinations.
                        </p>
                      )}
                    </div>
                  </div>

                  {prompts.length > 0 && (
                    <div className="space-y-3">
                      <label className={labelCls}>Details</label>
                      <div className="grid grid-cols-1 sm:grid-cols-2 gap-4">
                        {prompts.map((pr) => {
                          const value = answers[pr.key] || ''
                          const blocking = missingAnswers.includes(pr.key)
                          return (
                            <div key={pr.key}>
                              <label className={labelCls}>
                                {pr.title} {pr.required ? '*' : ''}
                              </label>
                              {pr.type === 'select' ? (
                                <select
                                  className={field}
                                  value={value}
                                  onChange={(ev) => setAnswers({ ...answers, [pr.key]: ev.target.value })}
                                >
                                  <option value="">Choose…</option>
                                  {(pr.options || []).map((o) => (
                                    <option key={o} value={o}>
                                      {o}
                                    </option>
                                  ))}
                                </select>
                              ) : (
                                <div className="flex gap-2">
                                  <input
                                    className={field}
                                    value={value}
                                    onChange={(ev) => setAnswers({ ...answers, [pr.key]: ev.target.value })}
                                  />
                                  {pr.type === 'remote' && pr.browse && integration.path && (
                                    <button
                                      type="button"
                                      onClick={() => setPickerFor(pr)}
                                      className="px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-zinc-300 hover:bg-white/10 text-sm whitespace-nowrap"
                                    >
                                      Browse…
                                    </button>
                                  )}
                                </div>
                              )}
                              {pr.help && <p className="mt-1 text-xs text-zinc-500">{pr.help}</p>}
                              {blocking && (
                                <p className="mt-1 text-xs text-amber-400">
                                  Needed by the mounts you selected.
                                </p>
                              )}
                            </div>
                          )
                        })}
                      </div>
                    </div>
                  )}

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
                                {wsFor(e) || '<workspace>'}:{pathFor(e)}
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

                  {/* Pre-flight, per DESTINATION. Each of these is a failure that
                      would otherwise only show up as a mount that looks fine and
                      does nothing. */}
                  {destinations.length > 0 && (
                    <div>
                      <label className={labelCls}>Destinations</label>
                      <ul className="space-y-2">
                        {destinations.map((d) => {
                          const key = `${d.workspace}:${d.root}`
                          const missing = missingTypesFor(d)
                          const unknownGate = allowedByWs[d.workspace] === null
                          const rootState = rootStates[key] || 'unknown'
                          return (
                            <li key={key} className="border border-white/10 rounded-lg p-3 text-xs">
                              <div className="font-mono text-zinc-300">
                                {d.workspace}:{d.root}
                                <span className="text-zinc-500">
                                  {' '}
                                  · {d.entries.length} mount{d.entries.length === 1 ? '' : 's'}
                                </span>
                              </div>
                              {unknownGate && (
                                <p className="mt-1 text-amber-400">
                                  Could not read this workspace&rsquo;s allowed node types; the gate check is
                                  skipped.
                                </p>
                              )}
                              {missing.length > 0 && (
                                <div className="mt-1 text-red-400 flex items-start gap-2">
                                  <AlertTriangle className="w-4 h-4 flex-shrink-0" />
                                  <div>
                                    <span className="font-mono">{d.workspace}</span> does not allow{' '}
                                    {missing.map((t) => (
                                      <span key={t} className="font-mono">
                                        {t}{' '}
                                      </span>
                                    ))}
                                    — every item would be rejected while the sync reports{' '}
                                    <span className="font-mono">ok</span>. Install or reinstall the
                                    connector&rsquo;s package (its workspace patches add these), or pick another
                                    workspace.
                                  </div>
                                </div>
                              )}
                              {rootState === 'missing' && (
                                <p className="mt-1 text-amber-400">
                                  <span className="font-mono">{d.root}</span> does not exist yet.{' '}
                                  <button
                                    type="button"
                                    onClick={() => createRoot(d.workspace, d.root)}
                                    disabled={creatingRoot === key}
                                    className="underline hover:text-amber-300 disabled:opacity-50"
                                  >
                                    {creatingRoot === key ? 'Creating…' : 'Create it'}
                                  </button>
                                </p>
                              )}
                              {rootState === 'exists' && <p className="mt-1 text-green-400">This folder exists.</p>}
                            </li>
                          )
                        })}
                      </ul>
                    </div>
                  )}
                  {plan.error && (
                    <div className="text-xs text-red-400 flex items-start gap-2 bg-red-500/5 border border-red-500/20 rounded-lg p-3">
                      <AlertTriangle className="w-4 h-4 flex-shrink-0" />
                      <div>
                        This bundle is malformed and cannot be instantiated: {plan.error}. It is shipped by the
                        connector&rsquo;s package, so this is a packaging bug rather than something to work around
                        here.
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

        {/* Browsing is an input affordance only: whatever is picked is written
            into the answer as an id, and the text field beside it stays usable —
            a provider may list a container the credential cannot actually read. */}
        {pickerFor && integration?.path && (
          <RemotePicker
            repo={repo}
            integrationPath={integration.path}
            accountId={accountRef || undefined}
            kind={pickerFor.browse as string}
            syncConfig={promptSyncConfig}
            searchable={pickerFor.browse === 'mailbox' || pickerFor.browse === 'user' || pickerFor.browse === 'site'}
            title={pickerFor.title}
            onSelect={(item) => {
              setAnswers((prev) => ({ ...prev, [pickerFor.key]: item.id }))
              setPickerFor(null)
            }}
            onClose={() => setPickerFor(null)}
          />
        )}

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
