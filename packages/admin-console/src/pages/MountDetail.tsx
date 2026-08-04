// SPDX-License-Identifier: BSL-1.1

import { useCallback, useEffect, useMemo, useState } from 'react'
import { Link, useNavigate, useParams } from 'react-router-dom'
import {
  Activity,
  AlertTriangle,
  ArrowLeft,
  CheckCircle2,
  FileMinus2,
  FilePlus2,
  FileWarning,
  History,
  Pencil,
  RefreshCw,
  SkipForward,
  Trash2,
  Wand2,
  Webhook,
  X,
} from 'lucide-react'
import GlassCard from '../components/GlassCard'
import ConfirmDialog from '../components/ConfirmDialog'
import { useToast, ToastContainer } from '../components/Toast'
import MountEditor from '../components/integrations/MountEditor'
import TestConnectionPanel from '../components/integrations/TestConnectionPanel'
import CapabilityChips from '../components/integrations/CapabilityChips'
import MountProgressPanel from '../components/mounts/MountProgressPanel'
import MountRunTimeline from '../components/mounts/MountRunTimeline'
import MountPushPanel from '../components/mounts/MountPushPanel'
import MountStatCard from '../components/mounts/MountStatCard'
import MountActivityFeed, {
  ConnectionPill,
  type ActivityBurst,
} from '../components/mounts/MountActivityFeed'
import {
  integrationsApi,
  SYSTEM_WORKSPACE,
  type Integration,
  type VirtualMount,
} from '../api/integrations'
import { workspacesApi } from '../api/workspaces'
import type { NodeEvent } from '../api/ws-events'
import { useNodeEvents } from '../hooks/useNodeEvents'
import { isActive, isSyncing, STATUS_META, statusKind } from '../utils/mountStatus'
import { formatAbsoluteSeconds, formatRelativeSeconds } from '../utils/time'

/** Poll interval used whenever the live socket is not carrying events. */
const POLL_MS = 5000

/**
 * Bursts older than this are dropped, and the list is capped, so a multi-hour
 * import does not grow an unbounded array in a tab left open overnight.
 */
const MAX_BURSTS = 60
/** Events this close together are treated as one batch commit. */
const BURST_WINDOW_MS = 1500

export default function MountDetail() {
  const { repo, mountName } = useParams<{ repo: string; mountName: string }>()
  const navigate = useNavigate()
  const { toasts, error: showError, success: showSuccess, info: showInfo, closeToast } = useToast()

  const [mount, setMount] = useState<VirtualMount | null>(null)
  const [integrations, setIntegrations] = useState<Integration[]>([])
  const [workspaces, setWorkspaces] = useState<string[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)

  const [showEditor, setShowEditor] = useState(false)
  const [showTest, setShowTest] = useState(false)
  const [confirmDelete, setConfirmDelete] = useState(false)
  const [confirmRemap, setConfirmRemap] = useState(false)
  const [enqueueing, setEnqueueing] = useState(false)
  const [bursts, setBursts] = useState<ActivityBurst[]>([])

  const integration = useMemo(
    () => integrations.find((i) => i.path === mount?.integration_ref),
    [integrations, mount?.integration_ref],
  )

  // ---- loading ----

  /**
   * `silent` refreshes leave the rendered page in place. Only the very first
   * load shows a skeleton — a poll that swaps the page for "Loading…" every 5
   * seconds is the "it keeps reloading itself" complaint, and it also throws
   * away scroll position and any open run in the timeline.
   */
  const load = useCallback(
    async (silent = false) => {
      if (!repo || !mountName) return
      if (!silent) setLoading(true)
      try {
        const m = await integrationsApi.getMount(repo, mountName)
        setMount(m)
        setError(null)
      } catch (e: any) {
        // A failed silent refresh must not blank a page that is already
        // showing good data — the mount keeps syncing regardless of this page.
        if (!silent) setError(e?.message || 'This mount could not be loaded.')
      } finally {
        if (!silent) setLoading(false)
      }
    },
    [repo, mountName],
  )

  useEffect(() => {
    load()
  }, [load])

  // Decorations. Failures here are non-fatal: they cost the connector name and
  // the editor's workspace dropdown, not the page.
  useEffect(() => {
    if (!repo) return
    integrationsApi.listIntegrations(repo).then(setIntegrations).catch(() => {})
    workspacesApi
      .list(repo)
      .then((ws) => setWorkspaces(ws.map((w) => w.name)))
      .catch(() => {})
  }, [repo])

  // ---- realtime ----

  /**
   * 1. The mount's own config node. Mount state is written as an ordinary node
   *    update, so every status/progress/run write shows up here and the page
   *    can re-read itself the moment the engine records anything.
   */
  const stateStatus = useNodeEvents(
    repo,
    {
      workspace: SYSTEM_WORKSPACE,
      node_type: 'raisin:VirtualMount',
      event_types: ['node:updated'],
    },
    useCallback(
      (event: NodeEvent) => {
        // Filtering by node id here rather than by path: `path` on the event is
        // the config-node path, but one subscription covers every mount in the
        // repo, and re-reading on another mount's write would be pointless load.
        if (mount?.id && event.payload.node_id && event.payload.node_id !== mount.id) return
        void load(true)
      },
      [load, mount?.id],
    ),
    !!mount,
  )

  /**
   * 2. The materialized items, so the page shows what the connector is doing
   *    right now rather than only what it has recorded.
   *
   * The subscription cannot filter by branch — the transport has no branch
   * filter at all — so events for this path on OTHER branches would otherwise
   * be attributed to this mount. `target_branch` is compared in the handler.
   */
  const itemFilters = useMemo(
    () => ({
      workspace: mount?.target_workspace,
      // Glob, not a prefix: `**` is the recursive form and `/x/**` excludes `/x`.
      path: mount?.mount_path
        ? `${mount.mount_path.replace(/\/+$/, '')}/**`
        : undefined,
      event_types: ['node:created', 'node:updated', 'node:deleted'],
    }),
    [mount?.target_workspace, mount?.mount_path],
  )

  const onItemEvent = useCallback(
    (event: NodeEvent) => {
      const p = event.payload
      if (mount?.target_branch && p.branch && p.branch !== mount.target_branch) return

      // Connector writes only. `metadata` is `HashMap<String, JsonValue>` and
      // may be absent entirely, so this is a soft filter: when the key is
      // missing we still show the event rather than rendering an empty feed on
      // a producer that does not stamp an actor.
      const actor = p.metadata?.actor
      if (typeof actor === 'string' && actor !== 'virtual-mount-sync') return

      const kind: ActivityBurst['kind'] =
        event.event_type === 'node:created'
          ? 'created'
          : event.event_type === 'node:deleted'
            ? 'deleted'
            : 'updated'
      const path = p.path || p.node_id || '(unknown)'
      const now = Date.now()

      setBursts((prev) => {
        const last = prev[prev.length - 1]
        // Coalesce into the trailing burst when it is the same kind and still
        // within the commit window — see ActivityBurst for why this is per
        // batch, not per item.
        if (last && last.kind === kind && now - last.lastAt < BURST_WINDOW_MS) {
          const merged = { ...last, count: last.count + 1, lastAt: now }
          return [...prev.slice(0, -1), merged]
        }
        const next: ActivityBurst = {
          id: `${now}-${Math.random().toString(36).slice(2, 8)}`,
          kind,
          count: 1,
          samplePath: path,
          firstAt: now,
          lastAt: now,
        }
        return [...prev, next].slice(-MAX_BURSTS)
      })
    },
    [mount?.target_branch],
  )

  const itemStatus = useNodeEvents(
    repo,
    itemFilters,
    onItemEvent,
    !!mount?.target_workspace && !!mount?.mount_path,
  )

  const live = stateStatus === 'live' || itemStatus === 'live'

  /**
   * Polling fallback. Kept whenever the socket is not live, and also while a
   * sync is in flight — the engine batches its state writes, so a run can be
   * progressing between node:updated events.
   */
  const active = isActive(mount?.state)
  useEffect(() => {
    // A live socket alone is not enough while a sync is running: the engine
    // batches its state writes, so progress advances between `node:updated`
    // events and a purely event-driven page would sit still mid-chunk.
    if (live && !active) return
    const t = window.setInterval(() => void load(true), POLL_MS)
    return () => window.clearInterval(t)
  }, [live, active, load])

  // Re-render on a timer so relative timestamps ("3m ago") stay honest on an
  // idle page that is neither polling nor receiving events.
  const [, setTick] = useState(0)
  useEffect(() => {
    const t = window.setInterval(() => setTick((n) => n + 1), 30000)
    return () => window.clearInterval(t)
  }, [])

  // ---- actions ----

  const syncing = isSyncing(mount?.state)
  // Disabled while the engine actually reports a sync, not merely for the ~100ms
  // the enqueue POST is in flight. The old button re-enabled immediately and
  // could be clicked indefinitely against a running job.
  const syncDisabled = enqueueing || syncing

  const enqueue = async (mode: 'delta' | 'full' | 'remap', label: string) => {
    if (!repo || !mount?.id) return
    setEnqueueing(true)
    try {
      const res = await integrationsApi.syncMount(repo, mount.id, mode)
      if (res.status === 'already_running') {
        // Informational, not a success: nothing was queued.
        showInfo('Already syncing', 'A run is in progress; this request was not queued.')
      } else {
        showSuccess(`${label} queued`, mount.title)
      }
      window.setTimeout(() => void load(true), 1200)
    } catch (e: any) {
      showError(`${label} failed`, e?.message)
    } finally {
      setEnqueueing(false)
    }
  }

  const handleSync = () =>
    enqueue(
      integration?.capabilities?.supports_changes === true ? 'delta' : 'full',
      'Sync',
    )

  const handleDelete = async () => {
    if (!repo || !mount?.id) return
    try {
      // Goes through the dedicated endpoint, which unregisters the provider
      // webhook BEFORE removing the node — a generic node delete leaks the
      // subscription. See `integrationsApi.deleteMount`.
      const res = await integrationsApi.deleteMount(repo, mount.id)
      // Only a mount that HAD a subscription can leak one; `unsubscribed: false`
      // is also the ordinary answer for a poll-mode mount.
      if (res.unsubscribed === false && !!mount.state?.push_subscription_id) {
        showInfo(
          'Deleted, but the provider subscription may remain',
          'The webhook could not be unregistered. If notifications keep arriving, remove the subscription at the provider.',
        )
      } else {
        showSuccess('Deleted', mount.title)
      }
      navigate(`/${repo}/mounts`)
    } catch (e: any) {
      showError('Delete failed', e?.message)
    } finally {
      setConfirmDelete(false)
    }
  }

  // ---- render ----

  const backLink = (
    <Link
      to={`/${repo}/mounts`}
      className="inline-flex items-center gap-1.5 text-sm text-zinc-400 hover:text-white transition-colors mb-4"
    >
      <ArrowLeft className="w-4 h-4" />
      Mounts
    </Link>
  )

  if (loading) {
    return (
      <div className="animate-fade-in">
        {backLink}
        <div className="animate-pulse space-y-4">
          <div className="h-9 bg-white/10 rounded w-72" />
          <div className="h-4 bg-white/10 rounded w-96" />
          <div className="grid gap-4 lg:grid-cols-3">
            <div className="h-64 bg-white/5 rounded-xl lg:col-span-2" />
            <div className="h-64 bg-white/5 rounded-xl" />
          </div>
        </div>
      </div>
    )
  }

  if (error || !mount) {
    return (
      <div className="animate-fade-in">
        {backLink}
        <GlassCard>
          <div className="text-center py-12">
            <AlertTriangle className="w-16 h-16 text-amber-400 mx-auto mb-4" />
            <h3 className="text-xl font-semibold text-white mb-2">Could not load this mount</h3>
            <p className="text-zinc-400 max-w-lg mx-auto">{error || 'The mount was not found.'}</p>
            <p className="text-zinc-500 text-sm mt-2">
              It keeps syncing on the server regardless of this page.
            </p>
            <button
              onClick={() => void load()}
              className="mt-4 px-4 py-2 bg-white/5 hover:bg-white/10 border border-white/10 text-white text-sm rounded-lg transition-colors"
            >
              Retry
            </button>
          </div>
        </GlassCard>
      </div>
    )
  }

  const kind = statusKind(mount.state)
  const meta = STATUS_META[kind]
  const lastRun = mount.state?.last_run
  const recentRuns = mount.state?.recent_runs ?? []

  return (
    <div className="animate-fade-in pb-8">
      {backLink}

      {/* ---- Header ---- */}
      <div className="flex flex-wrap items-start justify-between gap-4 mb-6">
        <div className="min-w-0">
          <div className="flex items-center gap-3 flex-wrap">
            <h1 className="text-3xl font-bold text-white">{mount.title}</h1>
            <span
              className={`inline-flex items-center gap-1.5 px-2.5 py-1 text-xs rounded-full border ${meta.cls}`}
              title={meta.hint}
            >
              <meta.Icon className={`w-3.5 h-3.5 ${kind === 'syncing' ? 'animate-spin' : ''}`} />
              {meta.label}
            </span>
            {syncing && (
              <span className="flex items-center gap-1.5 text-xs text-blue-400">
                <span className="relative flex w-2 h-2">
                  <span className="animate-ping absolute inline-flex w-full h-full rounded-full bg-blue-400 opacity-75" />
                  <span className="relative inline-flex w-2 h-2 rounded-full bg-blue-400" />
                </span>
                syncing now
              </span>
            )}
            {!mount.enabled && (
              <span className="px-2 py-0.5 text-xs rounded-full bg-zinc-500/10 text-zinc-400 border border-white/10">
                Disabled
              </span>
            )}
          </div>

          <div className="flex items-center gap-3 mt-2 flex-wrap text-sm">
            <span className="font-mono text-zinc-400">
              {mount.target_workspace}:{mount.mount_path}
            </span>
            <span className="text-zinc-600">·</span>
            <span className="text-zinc-500">branch {mount.target_branch}</span>
            {integration && (
              <>
                <span className="text-zinc-600">·</span>
                <span className="px-2 py-0.5 text-xs rounded-full bg-primary-500/10 text-primary-300 border border-primary-500/20">
                  {integration.title || integration.provider_type}
                </span>
              </>
            )}
            <ConnectionPill status={live ? 'live' : stateStatus} />
          </div>

          <p className="text-xs text-zinc-500 mt-1.5">{meta.hint}</p>
        </div>

        <div className="flex items-center gap-1.5 flex-wrap">
          <button
            onClick={handleSync}
            disabled={syncDisabled}
            title={syncing ? 'A sync is already running' : 'Sync now'}
            className="flex items-center gap-1.5 px-3 py-2 text-sm bg-primary-500 hover:bg-primary-600 disabled:opacity-40 disabled:cursor-not-allowed text-white rounded-lg transition-colors"
          >
            <RefreshCw className={`w-4 h-4 ${syncing || enqueueing ? 'animate-spin' : ''}`} />
            {syncing ? 'Syncing…' : 'Sync now'}
          </button>
          <button
            onClick={() => setShowTest(true)}
            className="flex items-center gap-1.5 px-3 py-2 text-sm text-zinc-300 hover:text-white bg-white/5 hover:bg-white/10 border border-white/10 rounded-lg transition-colors"
          >
            <Activity className="w-4 h-4" />
            Test
          </button>
          <button
            onClick={() => setConfirmRemap(true)}
            disabled={syncDisabled}
            className="flex items-center gap-1.5 px-3 py-2 text-sm text-zinc-300 hover:text-amber-300 bg-white/5 hover:bg-white/10 border border-white/10 rounded-lg transition-colors disabled:opacity-40"
          >
            <Wand2 className="w-4 h-4" />
            Remap
          </button>
          <button
            onClick={() => setShowEditor(true)}
            className="flex items-center gap-1.5 px-3 py-2 text-sm text-zinc-300 hover:text-white bg-white/5 hover:bg-white/10 border border-white/10 rounded-lg transition-colors"
          >
            <Pencil className="w-4 h-4" />
            Edit
          </button>
          <button
            onClick={() => setConfirmDelete(true)}
            className="flex items-center gap-1.5 px-3 py-2 text-sm text-zinc-400 hover:text-red-400 bg-white/5 hover:bg-red-500/10 border border-white/10 rounded-lg transition-colors"
          >
            <Trash2 className="w-4 h-4" />
          </button>
        </div>
      </div>

      {/* ---- Error banner: the loudest thing on the page when present ---- */}
      {mount.state?.last_error && (
        <div className="mb-4 rounded-xl border border-red-500/20 bg-red-500/10 p-4">
          <div className="flex items-start gap-2.5">
            <AlertTriangle className="w-5 h-5 text-red-400 mt-0.5 shrink-0" />
            <div className="min-w-0">
              <div className="flex items-center gap-2 flex-wrap">
                <h3 className="text-sm font-medium text-red-300">Last error</h3>
                <span
                  className="text-xs text-red-400/70"
                  title={formatAbsoluteSeconds(mount.state.last_attempt_at)}
                >
                  {formatRelativeSeconds(mount.state.last_attempt_at)}
                </span>
                {(mount.state.consecutive_failures ?? 0) > 1 && (
                  <span className="px-1.5 py-0.5 text-[10px] rounded bg-red-500/20 text-red-300">
                    {mount.state.consecutive_failures} consecutive failures
                  </span>
                )}
              </div>
              <p className="text-sm text-red-200/90 mt-1 break-words">{mount.state.last_error}</p>
            </div>
          </div>
        </div>
      )}

      <div className="grid gap-4 lg:grid-cols-3 items-start">
        {/* ---- Left column ---- */}
        <div className="lg:col-span-2 space-y-4">
          <GlassCard>
            <h2 className="text-sm font-medium text-white mb-3">Import progress</h2>
            <MountProgressPanel state={mount.state} />
          </GlassCard>

          <GlassCard>
            <div className="flex items-center justify-between mb-3 gap-3 flex-wrap">
              <h2 className="text-sm font-medium text-white">Last run</h2>
              {lastRun && (
                <span
                  className="text-xs text-zinc-500"
                  title={formatAbsoluteSeconds(lastRun.started_at)}
                >
                  {formatRelativeSeconds(lastRun.started_at)}
                </span>
              )}
            </div>
            {lastRun ? (
              <>
                {lastRun.state_write_skipped === true && (
                  <p className="mb-3 text-xs text-orange-300/90 bg-orange-500/10 border border-orange-500/20 rounded p-2 flex items-start gap-2">
                    <FileWarning className="w-3.5 h-3.5 mt-0.5 shrink-0" />
                    <span>
                      This run's record did not persist, so these counters may understate what
                      actually happened.
                    </span>
                  </p>
                )}
                <div className="grid grid-cols-2 sm:grid-cols-4 gap-2.5">
                  <MountStatCard
                    label="Written"
                    value={lastRun.written}
                    Icon={FilePlus2}
                    tone="good"
                    hint="Nodes created or updated by this run"
                  />
                  <MountStatCard
                    label="Skipped"
                    value={lastRun.skipped}
                    Icon={SkipForward}
                    hint="Items unchanged since the last sync"
                  />
                  <MountStatCard
                    label="Deleted"
                    value={lastRun.deleted}
                    Icon={FileMinus2}
                    tone="warn"
                    hint="Nodes removed because they are gone remotely"
                  />
                  <MountStatCard
                    label="Failed"
                    value={lastRun.failed}
                    Icon={AlertTriangle}
                    tone="bad"
                    hint="Items that could not be materialized"
                  />
                </div>
              </>
            ) : (
              <p className="text-sm text-zinc-500">No run has been recorded yet.</p>
            )}
          </GlassCard>

          <GlassCard>
            <div className="flex items-center gap-1.5 mb-3">
              <History className="w-3.5 h-3.5 text-zinc-500" />
              <h2 className="text-sm font-medium text-white">Run history</h2>
              {recentRuns.length > 0 && (
                <span className="text-xs text-zinc-500">({recentRuns.length})</span>
              )}
            </div>
            <MountRunTimeline runs={recentRuns} />
          </GlassCard>
        </div>

        {/* ---- Right column ---- */}
        <div className="space-y-4">
          <GlassCard>
            <MountActivityFeed
              bursts={bursts}
              status={itemStatus}
              watching={`${mount.target_workspace}:${mount.mount_path}`}
            />
          </GlassCard>

          <GlassCard>
            <div className="flex items-center gap-1.5 mb-3">
              <Webhook className="w-3.5 h-3.5 text-zinc-500" />
              <h2 className="text-sm font-medium text-white">Webhook health</h2>
            </div>
            <MountPushPanel state={mount.state} />
          </GlassCard>

          <GlassCard>
            <h2 className="text-sm font-medium text-white mb-3">Sync</h2>
            <dl className="space-y-2 text-sm">
              <DetailRow label="Last success">
                <span
                  className={mount.state?.last_sync_at ? 'text-zinc-300' : 'text-zinc-500'}
                  title={formatAbsoluteSeconds(mount.state?.last_sync_at)}
                >
                  {mount.state?.last_sync_at ? (
                    <span className="inline-flex items-center gap-1.5">
                      <CheckCircle2 className="w-3.5 h-3.5 text-green-400" />
                      {formatRelativeSeconds(mount.state.last_sync_at)}
                    </span>
                  ) : (
                    'never'
                  )}
                </span>
              </DetailRow>
              <DetailRow label="Last attempt">
                <span
                  className="text-zinc-300"
                  title={formatAbsoluteSeconds(mount.state?.last_attempt_at)}
                >
                  {formatRelativeSeconds(mount.state?.last_attempt_at)}
                </span>
              </DetailRow>
              <DetailRow label="Mode">
                <span className="text-zinc-300">{mount.sync_config?.mode || 'poll'}</span>
              </DetailRow>
              {mount.sync_config?.interval_seconds ? (
                <DetailRow label="Interval">
                  <span className="text-zinc-300">
                    every {mount.sync_config.interval_seconds}s
                  </span>
                </DetailRow>
              ) : null}
              <DetailRow label="Writeback">
                <span className="text-zinc-300">{mount.write_config?.writeback || 'off'}</span>
              </DetailRow>
              {mount.remote_root && (
                <DetailRow label="Remote root">
                  <span className="font-mono text-xs text-zinc-400 break-all">
                    {mount.remote_root}
                  </span>
                </DetailRow>
              )}
            </dl>
          </GlassCard>

          {integration && (
            <GlassCard>
              <h2 className="text-sm font-medium text-white mb-3">Connector</h2>
              <Link
                to={`/${repo}/integrations`}
                className="text-sm text-primary-300 hover:text-primary-200 transition-colors"
              >
                {integration.title || integration.name}
              </Link>
              <p className="text-xs text-zinc-500 mt-0.5 mb-3">{integration.provider_type}</p>
              <CapabilityChips capabilities={integration.capabilities} compact />
            </GlassCard>
          )}
        </div>
      </div>

      {/* ---- Modals ---- */}
      {showEditor && repo && (
        <MountEditor
          repo={repo}
          mount={mount}
          integrations={integrations}
          workspaces={workspaces}
          onClose={() => setShowEditor(false)}
          onSaved={() => void load(true)}
          onError={showError}
          onSuccess={showSuccess}
        />
      )}

      {showTest && repo && (
        <div className="fixed inset-0 bg-black/50 backdrop-blur-sm flex items-center justify-center z-50 p-4 overscroll-none">
          <div className="bg-zinc-900 border border-white/10 rounded-xl shadow-2xl max-w-lg w-full max-h-[90vh] overflow-y-auto overscroll-contain">
            <div className="flex items-center justify-between p-6 border-b border-white/10">
              <div>
                <h2 className="text-xl font-bold text-white">Test connection</h2>
                <p className="text-xs text-zinc-500">{mount.title}</p>
              </div>
              <button
                onClick={() => setShowTest(false)}
                className="p-2 hover:bg-white/10 rounded-lg transition-colors"
              >
                <X className="w-5 h-5 text-white/60" />
              </button>
            </div>
            <div className="p-6">
              <TestConnectionPanel
                repo={repo}
                autoRun
                disabledReason={
                  integration?.path
                    ? undefined
                    : 'The connector backing this mount could not be found.'
                }
                request={{
                  integration_path: integration?.path || mount.integration_ref,
                  account_id: mount.account_ref || undefined,
                  remote_root: mount.remote_root || undefined,
                  sync_config: mount.sync_config,
                }}
                onTested={() => void load(true)}
                onError={showError}
              />
            </div>
          </div>
        </div>
      )}

      <ConfirmDialog
        open={confirmDelete}
        title="Delete mount"
        message={`Delete “${mount.title}”? Materialized virtual nodes will be removed on next sync.`}
        variant="danger"
        confirmText="Delete"
        onConfirm={handleDelete}
        onCancel={() => setConfirmDelete(false)}
      />
      <ConfirmDialog
        open={confirmRemap}
        title="Remap this mount"
        message={
          `Re-import every item in “${mount.title}” through the current mapping function ` +
          `and folder hierarchy.\n\n` +
          `Existing nodes are updated in place — ids, revision history and anything you added ` +
          `locally are kept — and moved if the hierarchy changed. Use this after changing the ` +
          `mapping function or the folder hierarchy; ordinary syncs skip unchanged items and ` +
          `will not pick those changes up.\n\n` +
          `A large mailbox runs in chunks over several minutes or hours. Progress is shown above.`
        }
        confirmText="Remap"
        onConfirm={() => {
          setConfirmRemap(false)
          void enqueue('remap', 'Remap')
        }}
        onCancel={() => setConfirmRemap(false)}
      />
      <ToastContainer toasts={toasts} onClose={closeToast} />
    </div>
  )
}

function DetailRow({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex items-baseline gap-3">
      <dt className="text-zinc-500 text-xs w-24 shrink-0">{label}</dt>
      <dd className="min-w-0 flex-1">{children}</dd>
    </div>
  )
}
