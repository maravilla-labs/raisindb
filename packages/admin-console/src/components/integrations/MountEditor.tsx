// SPDX-License-Identifier: BSL-1.1

import { useEffect, useMemo, useState } from 'react'
import { X, Info } from 'lucide-react'
import {
  integrationsApi,
  type VirtualMount,
  type Integration,
  type SyncConfig,
  type WriteConfig,
} from '../../api/integrations'
import { branchesApi } from '../../api/branches'
import { nodesApi } from '../../api/nodes'
import { capabilitiesUnknown } from './CapabilityChips'
import TestConnectionPanel from './TestConnectionPanel'
import CopyableUrlField from './CopyableUrlField'
import type { SetupUrls } from '../../api/integrations'
import { FunctionPicker } from '../../pages/functions/components/editor/FunctionPicker'

interface MountEditorProps {
  repo: string
  mount?: VirtualMount
  integrations: Integration[]
  workspaces: string[]
  onClose: () => void
  onSaved: () => void
  onError: (title: string, message?: string) => void
  onSuccess: (title: string, message?: string) => void
}

const field =
  'w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-white/40 focus:outline-none focus:ring-2 focus:ring-primary-500'

/**
 * Mapper each connector's package ships, keyed by `provider_type` (and by
 * `sync_config.resource` where one connector has several surfaces).
 *
 * When `mapping_function` is empty the engine falls back to a GENERIC built-in
 * Rust mapping (folders → `raisin:Folder`, everything else → `raisin:Node` plus
 * a `meta` object) — *not* the mapper the package ships. So an unset field
 * silently yields shapeless nodes: a mail mount loses from/to/date/unread.
 *
 * This lives here because the package manifest cannot declare per-resource
 * default mappers today (`provides` carries only functions/content/nodetypes);
 * the connector config schema is its proper home.
 */
const DEFAULT_MAPPERS: Record<string, string | Record<string, string>> = {
  'ms-graph': {
    mail: '/mappers/ms-graph-mail',
    calendar: '/mappers/ms-graph-calendar',
    files: '/mappers/ms-graph-files',
  },
  'google-drive': '/mappers/google-drive-default',
  'google-calendar': '/mappers/google-calendar-default',
  imap: '/mappers/imap-default',
  gmail: '/mappers/imap-default',
}

/** Every shipped mapper path, for the free-text field's autocomplete. */
const ALL_MAPPERS = [
  ...new Set(
    Object.values(DEFAULT_MAPPERS).flatMap((v) =>
      typeof v === 'string' ? [v] : Object.values(v),
    ),
  ),
].sort()
const labelCls = 'block text-white text-sm font-medium mb-1.5'

export default function MountEditor({
  repo,
  mount,
  integrations,
  workspaces,
  onClose,
  onSaved,
  onError,
  onSuccess,
}: MountEditorProps) {
  const isEdit = !!mount
  const [name, setName] = useState(mount?.name || '')
  const [title, setTitle] = useState(mount?.title || '')
  const [integrationRef, setIntegrationRef] = useState(mount?.integration_ref || '')
  const [accountRef, setAccountRef] = useState(mount?.account_ref || '')
  const [targetWorkspace, setTargetWorkspace] = useState(mount?.target_workspace || '')
  const [targetBranch, setTargetBranch] = useState(mount?.target_branch || 'main')
  const [branches, setBranches] = useState<string[]>([])
  const [mountPath, setMountPath] = useState(mount?.mount_path || '/')
  // Whether the mount path already exists in the target workspace. The engine
  // materializes INTO this folder, so a missing one is a misconfiguration the
  // operator only discovers as a failing sync — check it here instead.
  const [mountPathState, setMountPathState] = useState<'unknown' | 'exists' | 'missing'>('unknown')
  const [creatingPath, setCreatingPath] = useState(false)
  const [remoteRoot, setRemoteRoot] = useState(mount?.remote_root || '')
  const [mappingFn, setMappingFn] = useState(mount?.mapping_function || '')
  const [enabled, setEnabled] = useState(mount?.enabled ?? true)
  const [sync, setSync] = useState<SyncConfig>(mount?.sync_config || { mode: 'poll', interval_seconds: 300 })
  const [write, setWrite] = useState<WriteConfig>(mount?.write_config || { writeback: 'off', conflict: 'remote_wins' })
  const [includeText, setIncludeText] = useState((mount?.sync_config?.include_patterns || []).join('\n'))
  const [excludeText, setExcludeText] = useState((mount?.sync_config?.exclude_patterns || []).join('\n'))
  const [saving, setSaving] = useState(false)
  // Per-mount push notification URL, resolved from the server once the mount
  // exists (it mints/reuses the mount's push token). Null until fetched.
  const [setupUrls, setSetupUrls] = useState<SetupUrls | null>(null)

  // Load the repo's branches so target_branch can be picked from real branches.
  useEffect(() => {
    let cancelled = false
    branchesApi
      .list(repo)
      .then((bs) => {
        if (cancelled) return
        const names = bs.map((b) => b.name)
        setBranches(names)
        // Keep the current selection if it exists; otherwise fall back to main.
        setTargetBranch((prev) => (names.includes(prev) ? prev : names.includes('main') ? 'main' : names[0] || prev))
      })
      .catch(() => {
        // Non-fatal: fall back to a free-standing default. The picker still lets
        // the user type via the always-present current value.
        if (!cancelled) setBranches([])
      })
    return () => {
      cancelled = true
    }
  }, [repo])

  const selectedIntegration = useMemo(
    () => integrations.find((i) => i.path === integrationRef),
    [integrations, integrationRef]
  )

  // Accounts available for the chosen integration (matched by node path).
  const accounts = selectedIntegration?.connected_accounts || []

  // Provider-shaped controls. ms-graph carries a mail/calendar switch; calendar
  // connectors (google-calendar, or ms-graph in calendar mode) take a time window.
  const providerType = selectedIntegration?.provider_type
  const isMsGraph = providerType === 'ms-graph'
  const isCalendar = providerType === 'google-calendar' || (isMsGraph && sync.resource === 'calendar')
  const windowDaysAhead = sync.window?.days_ahead ?? 90
  const windowDaysBack = sync.window?.days_back ?? 7

  /** Mount path with a leading slash and no trailing one; '/' stays '/'. */
  const normalizedMountPath = useMemo(() => {
    const p = mountPath.trim()
    if (!p || p === '/') return '/'
    const withLead = p.startsWith('/') ? p : `/${p}`
    return withLead.replace(/\/+$/, '') || '/'
  }, [mountPath])

  // Probe the target path as the operator types. Debounced, and cancelled on
  // change so a slow response cannot overwrite the state of a newer path.
  useEffect(() => {
    if (!targetWorkspace || normalizedMountPath === '/') {
      setMountPathState('unknown')
      return
    }
    let cancelled = false
    setMountPathState('unknown')
    const timer = window.setTimeout(() => {
      nodesApi
        .getAtHead(repo, targetBranch || 'main', targetWorkspace, normalizedMountPath)
        .then(() => !cancelled && setMountPathState('exists'))
        .catch((e: any) => {
          if (cancelled) return
          // Only a 404 proves absence. A 403/500 means we do not know, and
          // claiming "does not exist" would push the operator into creating a
          // duplicate folder.
          setMountPathState(e?.status === 404 ? 'missing' : 'unknown')
        })
    }, 400)
    return () => {
      cancelled = true
      window.clearTimeout(timer)
    }
  }, [repo, targetBranch, targetWorkspace, normalizedMountPath])

  async function createMountPath() {
    if (!targetWorkspace || normalizedMountPath === '/') return
    setCreatingPath(true)
    try {
      const segments = normalizedMountPath.split('/').filter(Boolean)
      const leaf = segments.pop() as string
      const parent = segments.length ? `/${segments.join('/')}` : '/'
      await nodesApi.create(repo, targetBranch || 'main', targetWorkspace, parent, {
        name: leaf,
        node_type: 'raisin:Folder',
        properties: { title: leaf },
      })
      setMountPathState('exists')
      onSuccess('Folder created', normalizedMountPath)
    } catch (e: any) {
      // The likeliest cause is an intermediate segment missing; say so rather
      // than echoing a bare 404.
      onError(
        'Could not create the folder',
        e?.message ||
          `Create the parent folders of ${normalizedMountPath} in ${targetWorkspace} first.`,
      )
    } finally {
      setCreatingPath(false)
    }
  }

  const suggestedMapper = useMemo(() => {
    const entry = providerType ? DEFAULT_MAPPERS[providerType] : undefined
    if (!entry) return ''
    if (typeof entry === 'string') return entry
    return entry[sync.resource || 'mail'] || ''
  }, [providerType, sync.resource])

  /**
   * Hierarchy each connector wants by default, keyed the same way as
   * DEFAULT_MAPPERS. Mail groups by thread: a mailbox flattened under one
   * folder is unusable in a tree, and `conversation_id` is the grouping people
   * actually read mail by. Suggested only — always overridable, and an empty
   * value keeps the provider's own structure (right for Drive/OneDrive, which
   * already has real folders worth mirroring).
   */
  const suggestedPathTemplate = useMemo(() => {
    if (isMsGraph && (sync.resource || 'mail') === 'mail') return '{conversation_id}/{name}'
    if (providerType === 'imap' || providerType === 'gmail') return '{conversation_id}/{name}'
    return ''
  }, [providerType, isMsGraph, sync.resource])

  const [showMapperPicker, setShowMapperPicker] = useState(false)

  // With several connections the engine refuses to guess which one a mount
  // means (it used to silently take the first, syncing an arbitrary mailbox),
  // so pin one here. With exactly one there is nothing to get wrong.
  const accountRequired = accounts.length > 1

  // Capability-driven form state. Absent capabilities => unknown => conservative.
  const caps = selectedIntegration?.capabilities
  const capsUnknown = capabilitiesUnknown(caps)
  const canWrite = caps?.can_write === true
  const supportsChanges = caps?.supports_changes === true
  // Push (webhook/hybrid) modes are offered only when the connector advertises
  // supports_push. Otherwise the mode is forced to poll.
  const supportsPush = caps?.supports_push === true

  // Fetch the per-mount notification URL once the mount exists and its connector
  // supports push. New (unsaved) mounts have no id yet — the URL only exists
  // after the mount is created, so we show a hint until then.
  useEffect(() => {
    let cancelled = false
    if (!isEdit || !mount?.id || !supportsPush) {
      setSetupUrls(null)
      return
    }
    integrationsApi
      .getSetupUrls(repo, mount.id)
      .then((s) => {
        if (!cancelled) setSetupUrls(s)
      })
      .catch(() => {
        if (!cancelled) setSetupUrls(null)
      })
    return () => {
      cancelled = true
    }
  }, [repo, isEdit, mount?.id, supportsPush])
  // v1 engine never implements write-through; it stamps writeback_supported=false
  // whenever writeback is requested. Treat a present `false` as authoritative.
  const writebackSupported = mount?.state?.writeback_supported
  const writeImplemented = false // engine has no write-through path in v1

  function patchSync(patch: Partial<SyncConfig>) {
    setSync((prev) => ({ ...prev, ...patch }))
  }

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault()
    if (!name.trim() || !title.trim() || !integrationRef || !targetWorkspace || !mountPath.trim()) {
      onError('Missing fields', 'Name, title, connector, workspace and mount path are required.')
      return
    }
    if (accountRequired && !accountRef) {
      onError(
        'Pick a connection',
        'This connector has several connections, so the mount must say which one to sync. ' +
          'Leaving it unset would make the sync fail as misconfigured.',
      )
      return
    }
    setSaving(true)
    try {
      const lines = (t: string) => t.split(/[\n,]/).map((s) => s.trim()).filter(Boolean)
      const model: VirtualMount = {
        // Carry the server's property bag so the save merges rather than
        // replaces — otherwise the engine-owned `state` (sync cursor, push
        // subscription) is deleted on every mount edit.
        _raw: mount?._raw,
        name: name.trim(),
        title: title.trim(),
        integration_ref: integrationRef,
        account_ref: accountRef || undefined,
        target_workspace: targetWorkspace,
        target_branch: targetBranch || 'main',
        mount_path: mountPath.trim(),
        remote_root: remoteRoot.trim() || undefined,
        mapping_function: mappingFn.trim() || undefined,
        enabled,
        sync_config: {
          ...sync,
          include_patterns: lines(includeText),
          exclude_patterns: lines(excludeText),
        },
        write_config: write,
      }
      if (isEdit) {
        await integrationsApi.updateMount(repo, mount!.name, model)
      } else {
        await integrationsApi.createMount(repo, model)
      }
      onSuccess(isEdit ? 'Mount updated' : 'Mount created', model.title)
      onSaved()
      onClose()
    } catch (e: any) {
      onError('Save failed', e?.message)
    } finally {
      setSaving(false)
    }
  }

  return (
    <div className="fixed inset-0 bg-black/50 backdrop-blur-sm flex items-center justify-center z-50 p-4 overscroll-none">
      <div className="bg-zinc-900 border border-white/10 rounded-xl shadow-2xl max-w-2xl w-full max-h-[90vh] overflow-y-auto overscroll-contain">
        <div className="flex items-center justify-between p-6 border-b border-white/10">
          <h2 className="text-2xl font-bold text-white">{isEdit ? 'Edit Mount' : 'New Mount'}</h2>
          <button onClick={onClose} className="p-2 hover:bg-white/10 rounded-lg transition-colors">
            <X className="w-5 h-5 text-white/60" />
          </button>
        </div>

        <form onSubmit={handleSubmit} className="p-6 space-y-5">
          <div className="grid grid-cols-2 gap-4">
            <div>
              <label className={labelCls}>Name *</label>
              <input className={field} value={name} disabled={isEdit} onChange={(e) => setName(e.target.value)} placeholder="team-drive" />
            </div>
            <div>
              <label className={labelCls}>Title *</label>
              <input className={field} value={title} onChange={(e) => setTitle(e.target.value)} placeholder="Team Drive" />
            </div>
            <div>
              <label className={labelCls}>Connector *</label>
              <select
                className={field}
                value={integrationRef}
                onChange={(e) => {
                  setIntegrationRef(e.target.value)
                  setAccountRef('')
                }}
              >
                <option value="">Select connector…</option>
                {integrations.map((i) => (
                  <option key={i.path} value={i.path}>
                    {i.title}
                  </option>
                ))}
              </select>
            </div>
            <div>
              <label className={labelCls}>
                Connection{accountRequired ? ' *' : ''}
              </label>
              <select className={field} value={accountRef} onChange={(e) => setAccountRef(e.target.value)} disabled={!integrationRef}>
                <option value="">
                  {accountRequired ? 'Select a connection…' : accounts.length === 1 ? 'Use the only connection' : 'Default / none'}
                </option>
                {accounts.map((a) => (
                  <option key={a.id} value={a.id}>
                    {a.label || a.subject || a.id}
                  </option>
                ))}
              </select>
            </div>
            <div>
              <label className={labelCls}>Target workspace *</label>
              <select className={field} value={targetWorkspace} onChange={(e) => setTargetWorkspace(e.target.value)}>
                <option value="">Select workspace…</option>
                {workspaces.map((w) => (
                  <option key={w} value={w}>
                    {w}
                  </option>
                ))}
              </select>
            </div>
            <div>
              <label className={labelCls}>Target branch *</label>
              <select className={field} value={targetBranch} onChange={(e) => setTargetBranch(e.target.value)}>
                {!branches.includes(targetBranch) && targetBranch && (
                  <option value={targetBranch}>{targetBranch}</option>
                )}
                {branches.map((b) => (
                  <option key={b} value={b}>
                    {b}
                  </option>
                ))}
              </select>
              <p className="mt-1 text-xs text-zinc-500">
                The mount only syncs into this branch. Creating other branches will not run this mount.
              </p>
            </div>
            <div>
              <label className={labelCls}>Mount path *</label>
              <input className={field} value={mountPath} onChange={(e) => setMountPath(e.target.value)} placeholder="/documents/shared" />
              <p className="mt-1 text-xs text-zinc-500">
                The folder in <span className="font-mono">{targetWorkspace}</span> that synced
                items are written <em>into</em> — the mount&rsquo;s root, not a node it creates.
                Everything below it is owned by this mount: items that vanish upstream are
                deleted here, so give the mount its own folder rather than pointing it at a
                path you also edit by hand.
              </p>
              {mountPathState === 'missing' && (
                <p className="mt-1 text-xs text-amber-400 flex items-start gap-1.5">
                  <span>
                    <span className="font-mono">{normalizedMountPath}</span> does not exist in{' '}
                    <span className="font-mono">{targetWorkspace}</span> yet.
                  </span>
                  <button
                    type="button"
                    onClick={createMountPath}
                    disabled={creatingPath}
                    className="underline hover:text-amber-300 disabled:opacity-50 whitespace-nowrap"
                  >
                    {creatingPath ? 'Creating…' : 'Create it'}
                  </button>
                </p>
              )}
              {mountPathState === 'exists' && normalizedMountPath !== '/' && (
                <p className="mt-1 text-xs text-green-400">This folder exists.</p>
              )}
              {normalizedMountPath === '/' && (
                <p className="mt-1 text-xs text-amber-400">
                  Mounting at the workspace root mixes synced items in with all your other
                  content, and mount-owned deletes then operate at the top level. A dedicated
                  subfolder is strongly preferred.
                </p>
              )}
            </div>
            <div>
              <label className={labelCls}>Folder hierarchy</label>
              <input
                className={field}
                value={sync.path_template ?? ''}
                onChange={(e) => setSync({ ...sync, path_template: e.target.value })}
                placeholder={suggestedPathTemplate || 'flat — keep the provider’s own structure'}
              />
              <p className="mt-1 text-xs text-zinc-500">
                Where each item is filed under the mount path. Placeholders read the item&rsquo;s
                fields: <code className="font-mono">{'{conversation_id}/{name}'}</code> gives one
                folder per mail thread, <code className="font-mono">{'{date:%Y}/{date:%m}/{name}'}</code>{' '}
                groups by year and month. Leave blank to keep the provider&rsquo;s own structure.
                {suggestedPathTemplate && !sync.path_template && (
                  <>
                    {' '}
                    <button
                      type="button"
                      className="underline hover:text-zinc-300"
                      onClick={() => setSync({ ...sync, path_template: suggestedPathTemplate })}
                    >
                      Use {suggestedPathTemplate}
                    </button>
                  </>
                )}
              </p>
              {isEdit && (sync.path_template ?? '') !== (mount?.sync_config?.path_template ?? '') && (
                <p className="mt-1 text-xs text-amber-400">
                  Changing this only affects items synced from now on. Already-synced nodes keep
                  their current path — the engine matches them by external id and updates them in
                  place, so it will not move them.
                </p>
              )}
            </div>
            <div>
              <label className={labelCls}>Remote root</label>
              <input className={field} value={remoteRoot} onChange={(e) => setRemoteRoot(e.target.value)} placeholder="folder id / mailbox" />
            </div>
            <div>
              <label className={labelCls}>Mapping function</label>
              <div className="flex gap-2">
                <input
                  className={field}
                  value={mappingFn}
                  onChange={(e) => setMappingFn(e.target.value)}
                  placeholder={suggestedMapper || '/mappers/… (built-in default)'}
                  list="mount-mapper-options"
                />
                <button
                  type="button"
                  onClick={() => setShowMapperPicker(true)}
                  className="px-3 py-2 bg-white/5 hover:bg-white/10 border border-white/10 text-white text-sm rounded-lg whitespace-nowrap"
                >
                  Browse…
                </button>
              </div>
              <datalist id="mount-mapper-options">
                {ALL_MAPPERS.map((p) => (
                  <option key={p} value={p} />
                ))}
              </datalist>
              {!mappingFn.trim() && suggestedMapper && (
                <p className="mt-1 text-xs text-amber-400">
                  Leaving this empty uses the generic built-in mapping, not this connector's
                  mapper — synced items become plain nodes.{' '}
                  <button
                    type="button"
                    className="underline hover:text-amber-300"
                    onClick={() => setMappingFn(suggestedMapper)}
                  >
                    Use {suggestedMapper}
                  </button>
                </p>
              )}
            </div>
          </div>

          {showMapperPicker && (
            <FunctionPicker
              currentFunctionPath={mappingFn || undefined}
              onSelect={(p) => {
                setMappingFn(p)
                setShowMapperPicker(false)
              }}
              onClose={() => setShowMapperPicker(false)}
            />
          )}

          <fieldset className="border border-white/10 rounded-lg p-4 space-y-4">
            <legend className="px-2 text-sm font-semibold text-zinc-300">Sync</legend>
            <div className="grid grid-cols-3 gap-4">
              <div>
                <label className={labelCls}>Mode</label>
                <select
                  className={field}
                  value={supportsPush ? sync.mode || 'poll' : 'poll'}
                  disabled={!supportsPush}
                  onChange={(e) => patchSync({ mode: e.target.value as SyncConfig['mode'] })}
                >
                  <option value="poll">poll</option>
                  {/* Webhook / hybrid modes require the connector to support push. */}
                  {supportsPush && <option value="webhook">webhook</option>}
                  {supportsPush && <option value="hybrid">hybrid</option>}
                </select>
                {supportsPush && (
                  <p className="mt-1 text-[11px] text-zinc-500">
                    webhook = push-only; hybrid = push + safety-net polling
                  </p>
                )}
              </div>
              <div>
                <label className={labelCls}>Interval (s)</label>
                <input
                  type="number"
                  className={field}
                  value={sync.interval_seconds ?? 300}
                  onChange={(e) => patchSync({ interval_seconds: Number(e.target.value) })}
                />
              </div>
              <div>
                <label className={labelCls}>Max items / sync</label>
                <input
                  type="number"
                  className={field}
                  value={sync.max_items_per_sync ?? 500}
                  onChange={(e) => patchSync({ max_items_per_sync: Number(e.target.value) })}
                />
              </div>
            </div>
            {integrationRef && (
              <p
                className="flex items-start gap-1.5 text-xs text-zinc-500"
                title={
                  supportsChanges
                    ? 'This connector exposes a delta API, so each sync fetches only what changed.'
                    : 'This connector has no delta API, so the engine re-lists everything and diffs it every sync.'
                }
              >
                <Info className="w-3.5 h-3.5 flex-shrink-0 mt-0.5" />
                {capsUnknown
                  ? 'Sync strategy unknown — run Test connection. Until then the mount full-reconciles on every sync.'
                  : supportsChanges
                    ? 'Incremental (delta) sync available — the engine syncs only changes.'
                    : 'Delta sync unavailable — this connector can only full-reconcile (re-list) on every sync.'}
              </p>
            )}
            {(isMsGraph || isCalendar) && (
              <div className="grid grid-cols-3 gap-4">
                {isMsGraph && (
                  <div>
                    <label className={labelCls}>Resource</label>
                    <select
                      className={field}
                      value={sync.resource || 'mail'}
                      onChange={(e) => patchSync({ resource: e.target.value as SyncConfig['resource'] })}
                    >
                      <option value="mail">Mail</option>
                      <option value="calendar">Calendar</option>
                      <option value="files">OneDrive</option>
                    </select>
                  </div>
                )}
                {isCalendar && (
                  <>
                    <div>
                      <label className={labelCls}>Days ahead</label>
                      <input
                        type="number"
                        min={0}
                        className={field}
                        value={windowDaysAhead}
                        onChange={(e) => patchSync({ window: { ...sync.window, days_ahead: Number(e.target.value) } })}
                      />
                    </div>
                    <div>
                      <label className={labelCls}>Days back</label>
                      <input
                        type="number"
                        min={0}
                        className={field}
                        value={windowDaysBack}
                        onChange={(e) => patchSync({ window: { ...sync.window, days_back: Number(e.target.value) } })}
                      />
                    </div>
                  </>
                )}
              </div>
            )}
            <div className="grid grid-cols-2 gap-4">
              <div>
                <label className={labelCls}>Include patterns</label>
                <textarea className={`${field} resize-none font-mono text-xs`} rows={2} value={includeText} onChange={(e) => setIncludeText(e.target.value)} />
              </div>
              <div>
                <label className={labelCls}>Exclude patterns</label>
                <textarea className={`${field} resize-none font-mono text-xs`} rows={2} value={excludeText} onChange={(e) => setExcludeText(e.target.value)} />
              </div>
            </div>
            <div className="flex items-center gap-6">
              <label className="flex items-center gap-2 text-white text-sm">
                <input type="checkbox" checked={sync.ephemeral || false} onChange={(e) => patchSync({ ephemeral: e.target.checked })} className="w-4 h-4 rounded" />
                Ephemeral
              </label>
              <div className="flex items-center gap-2">
                <label className="text-white text-sm">TTL (s)</label>
                <input
                  type="number"
                  className={`${field} w-28`}
                  value={sync.ttl_seconds ?? ''}
                  onChange={(e) => patchSync({ ttl_seconds: e.target.value ? Number(e.target.value) : null })}
                />
              </div>
            </div>
          </fieldset>

          {/*
            Push notification URL — shown only for push-capable connectors. This
            is the URL the operator pastes into the provider's webhook/push
            subscription (required for Gmail Pub/Sub; automatic for Microsoft
            Graph and Google Calendar). Only exists once the mount is saved.
          */}
          {supportsPush && (
            <fieldset className="border border-white/10 rounded-lg p-4 space-y-3">
              <legend className="px-2 text-sm font-semibold text-zinc-300">Push notifications</legend>
              {setupUrls?.notification_url ? (
                <CopyableUrlField
                  label="Notification URL"
                  value={setupUrls.notification_url}
                  warn={!setupUrls.base_url_configured}
                  helper={
                    setupUrls.base_url_configured ? (
                      'Paste this into your provider’s webhook/subscription config. Required for Gmail (Pub/Sub); automatic for Microsoft Graph and Google Calendar.'
                    ) : (
                      <>
                        RAISINDB_BASE_URL is not set on the server, so this URL carries a{' '}
                        <code className="font-mono">{'{base}'}</code> placeholder. Set RAISINDB_BASE_URL
                        and re-open this mount to get the real notification URL.
                      </>
                    )
                  }
                />
              ) : (
                <p className="flex items-start gap-1.5 text-xs text-zinc-500">
                  <Info className="w-3.5 h-3.5 flex-shrink-0 mt-0.5" />
                  {isEdit
                    ? 'The notification URL will appear here once the mount is saved.'
                    : 'Save this mount first, then re-open it to get the push notification URL to paste into your provider.'}
                </p>
              )}
            </fieldset>
          )}

          {/*
            Write-through section is capability-driven. It is shown only when the
            connector advertises can_write, the engine reports writeback_supported,
            AND the engine actually implements write-through. In v1 the engine has
            no write-through path (writeImplemented=false), so this section is
            always hidden and we surface a one-line note instead of a dead control.
            The stored write_config is preserved untouched on save.
          */}
          {writeImplemented && canWrite && writebackSupported !== false && !capsUnknown ? (
            <fieldset className="border border-white/10 rounded-lg p-4">
              <legend className="px-2 text-sm font-semibold text-zinc-300">Write-through</legend>
              <div className="grid grid-cols-2 gap-4">
                <div>
                  <label className={labelCls}>Writeback</label>
                  <select className={field} value={write.writeback || 'off'} onChange={(e) => setWrite((p) => ({ ...p, writeback: e.target.value as WriteConfig['writeback'] }))}>
                    <option value="off">off (read-only)</option>
                    <option value="write_through">write_through</option>
                  </select>
                </div>
                <div>
                  <label className={labelCls}>Conflict</label>
                  <select className={field} value={write.conflict || 'remote_wins'} onChange={(e) => setWrite((p) => ({ ...p, conflict: e.target.value as WriteConfig['conflict'] }))}>
                    <option value="remote_wins">remote_wins</option>
                    <option value="error">error</option>
                  </select>
                </div>
              </div>
            </fieldset>
          ) : (
            <p className="flex items-start gap-1.5 text-xs text-zinc-500">
              <Info className="w-3.5 h-3.5 flex-shrink-0 mt-0.5" />
              {capsUnknown
                ? 'Capabilities unknown — run Test connection. This mount is read-only until then.'
                : !canWrite
                  ? 'This connector is read-only.'
                  : 'Write-through is not yet implemented in the engine — this mount is read-only.'}
            </p>
          )}

          {selectedIntegration?.path && (
            <div className="border border-white/10 rounded-lg p-4 space-y-2">
              <h3 className="text-sm font-semibold text-white">Test connection</h3>
              <TestConnectionPanel
                repo={repo}
                request={{
                  integration_path: selectedIntegration.path,
                  account_id: accountRef || undefined,
                  remote_root: remoteRoot.trim() || undefined,
                  // Forward the surface selector so a calendar/files mount is
                  // probed against what it actually syncs, not the mailbox.
                  sync_config: sync,
                }}
                onError={onError}
              />
            </div>
          )}

          <label className="flex items-center gap-2 text-white text-sm">
            <input type="checkbox" checked={enabled} onChange={(e) => setEnabled(e.target.checked)} className="w-4 h-4 rounded" />
            Enabled
          </label>

          <div className="flex flex-col-reverse md:flex-row gap-3 pt-2">
            <button type="button" onClick={onClose} className="flex-1 px-6 py-3 bg-white/5 hover:bg-white/10 border border-white/10 text-white rounded-lg transition-colors">
              Cancel
            </button>
            <button type="submit" disabled={saving} className="flex-1 px-6 py-3 bg-primary-500 hover:bg-primary-600 text-white rounded-lg transition-colors disabled:opacity-50">
              {saving ? 'Saving…' : isEdit ? 'Update' : 'Create'}
            </button>
          </div>
        </form>
      </div>
    </div>
  )
}
