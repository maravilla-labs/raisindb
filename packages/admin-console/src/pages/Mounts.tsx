// SPDX-License-Identifier: BSL-1.1

import { useEffect, useState, useCallback } from 'react'
import { Link, useParams } from 'react-router-dom'
import { HardDrive, Plus, RefreshCw, AlertTriangle, Activity, Webhook, X, Wand2, ChevronRight } from 'lucide-react'
import GlassCard from '../components/GlassCard'
import ConfirmDialog from '../components/ConfirmDialog'
import { ItemTable, type TableColumn } from '../components/ItemTable'
import { useToast, ToastContainer } from '../components/Toast'
import MountEditor from '../components/integrations/MountEditor'
import TestConnectionPanel from '../components/integrations/TestConnectionPanel'
import { integrationsApi, type VirtualMount, type Integration } from '../api/integrations'
import { workspacesApi } from '../api/workspaces'
// Shared with the detail view. These used to be private to this file; a row and
// the page it links to disagreeing about a mount's status is worse than either
// being wrong alone, so there is now exactly one status ladder.
import {
  backfillProgress,
  isActive,
  isPaused,
  isSyncing,
  pushIndicator,
  STATUS_META,
  statusKind,
  writeModeLabel,
} from '../utils/mountStatus'
import { formatAbsoluteSeconds, formatRelativeSeconds } from '../utils/time'

export default function Mounts() {
  const { repo } = useParams<{ repo: string }>()
  const [mounts, setMounts] = useState<VirtualMount[]>([])
  const [integrations, setIntegrations] = useState<Integration[]>([])
  const [workspaces, setWorkspaces] = useState<string[]>([])
  const [loading, setLoading] = useState(true)
  const [editing, setEditing] = useState<VirtualMount | undefined>(undefined)
  const [showEditor, setShowEditor] = useState(false)
  const [deleteTarget, setDeleteTarget] = useState<VirtualMount | null>(null)
  const [testTarget, setTestTarget] = useState<VirtualMount | null>(null)
  const [remapTarget, setRemapTarget] = useState<VirtualMount | null>(null)
  /** Set when the mounts list itself failed, so "broken" never renders as "empty". */
  const [loadError, setLoadError] = useState<string | null>(null)
  const { toasts, error: showError, success: showSuccess, info: showInfo, closeToast } = useToast()

  /** The connector node backing a mount (matched by integration_ref path). */
  const integrationFor = (m: VirtualMount) => integrations.find((i) => i.path === m.integration_ref)

  /**
   * @param silent Refresh in place, leaving the current table rendered.
   *
   * Only the FIRST load may show the skeleton. `load()` used to call
   * `setLoading(true)` unconditionally, including from the 5s poll, and the
   * render swapped the whole table for a centered "Loading…" — so an importing
   * mount made the page blink back to a spinner every five seconds, which is
   * the "the page keeps reloading itself" complaint. A refresh must be
   * invisible unless it is the first one.
   */
  const load = useCallback(async (silent = false) => {
    if (!repo) return
    if (!silent) setLoading(true)
    // allSettled, NOT all. The mounts list is the entire point of this page, and
    // the other two calls only decorate it (connector names, the workspace
    // dropdown in the editor). Under Promise.all a failure in either one
    // rejected the whole batch, so `setMounts` never ran and the table rendered
    // empty — indistinguishable from "no mounts configured", with only a
    // transient toast to say otherwise. A mount you cannot see is a mount you
    // cannot sync, disable or delete.
    const [msR, intsR, wsR] = await Promise.allSettled([
      integrationsApi.listMounts(repo),
      integrationsApi.listIntegrations(repo),
      workspacesApi.list(repo),
    ])

    if (msR.status === 'fulfilled') {
      setMounts(msR.value)
      setLoadError(null)
    } else {
      // Persistent, not a toast: this must never be mistaken for an empty list.
      setLoadError(msR.reason?.message || 'The mounts list could not be loaded.')
    }
    if (intsR.status === 'fulfilled') setIntegrations(intsR.value)
    if (wsR.status === 'fulfilled') setWorkspaces(wsR.value.map((w) => w.name))

    // Degraded, not fatal: say so, but keep the mounts on screen.
    if (msR.status === 'fulfilled' && (intsR.status === 'rejected' || wsR.status === 'rejected')) {
      showError(
        'Some details could not be loaded',
        'Connector names and the workspace list are unavailable; mounts are still shown and can be synced.',
      )
    }
    if (!silent) setLoading(false)
  }, [repo])

  useEffect(() => {
    load()
  }, [load])

  // Poll while any mount has a walk in flight. A chunked import runs for
  // minutes to hours, and a progress number that only moves on manual refresh
  // is not progress. Polling stops as soon as nothing is running, so an idle
  // page costs nothing.
  //
  // The trigger is `isActive`, NOT `backfill_cursor` alone: the cursor only
  // exists at a chunk boundary, so keying on it stopped the refresh during the
  // very chunk whose progress the user is watching.
  const anyRunning = mounts.some((m) => isActive(m.state))
  useEffect(() => {
    if (!anyRunning) return
    const t = window.setInterval(() => void load(true), 5000)
    return () => window.clearInterval(t)
  }, [anyRunning, load])

  /**
   * Mounts whose enqueue POST is still in flight.
   *
   * This is only the request window. Whether the button stays disabled after
   * that is decided by the mount's own `status === 'syncing'` — see
   * `syncBusy`. `syncingId` alone cleared in `finally` roughly 100ms after the
   * POST returned, so the button re-enabled while the job was just starting and
   * could be clicked over and over against a running sync.
   */
  const [enqueuing, setEnqueuing] = useState<Set<string>>(new Set())
  const markEnqueuing = (id: string, on: boolean) =>
    setEnqueuing((prev) => {
      const next = new Set(prev)
      if (on) next.add(id)
      else next.delete(id)
      return next
    })

  /** True while a mount cannot accept another sync request. */
  const syncBusy = (m: VirtualMount) =>
    (!!m.id && enqueuing.has(m.id)) || isSyncing(m.state)

  /**
   * Why Sync/Remap are unavailable, or null when they are not. A disabled or
   * paused mount used to accept the click and toast "Sync queued" for work the
   * scheduler would never run.
   */
  const syncUnavailable = (m: VirtualMount): string | null => {
    if (!m.enabled) return 'This mount is disabled — enable it first'
    if (isPaused(m.state)) return 'Paused — resume on the mount page to sync'
    return null
  }

  async function handleSync(m: VirtualMount) {
    if (!repo || !m.id) return
    markEnqueuing(m.id, true)
    try {
      // Only request a delta sync when the connector advertises a delta API;
      // otherwise the engine can only full-reconcile.
      const mode = integrationFor(m)?.capabilities?.supports_changes === true ? 'delta' : 'full'
      const res = await integrationsApi.syncMount(repo, m.id, mode)
      if (res.status === 'already_running') {
        // Informational, not a green success toast: NOTHING was queued. Saying
        // "success" to a request the server declined is how a user ends up
        // clicking Sync repeatedly, believing each click did something.
        showInfo('Already syncing', 'A run is in progress; this request was not queued.')
      } else {
        showSuccess('Sync queued', m.title)
      }
      // Give the job a moment, then refresh state.
      window.setTimeout(() => void load(true), 1200)
    } catch (e: any) {
      showError('Sync failed', e?.message)
    } finally {
      markEnqueuing(m.id, false)
    }
  }

  async function confirmRemap() {
    if (!repo || !remapTarget?.id) return
    const target = remapTarget
    setRemapTarget(null)
    markEnqueuing(target.id!, true)
    try {
      const res = await integrationsApi.syncMount(repo, target.id!, 'remap')
      if (res.status === 'already_running') {
        showInfo('Already syncing', 'A run is in progress; the remap was not queued.')
      } else {
        showSuccess(
          'Remap queued',
          'Every item will be re-imported through the current mapping. Progress appears in the row.',
        )
      }
      window.setTimeout(() => void load(true), 1200)
    } catch (e: any) {
      showError('Remap failed', e?.message)
    } finally {
      markEnqueuing(target.id!, false)
    }
  }

  async function confirmDelete() {
    if (!repo || !deleteTarget?.id) return
    try {
      const res = await integrationsApi.deleteMount(repo, deleteTarget.id)
      // A failed unsubscribe still deletes the mount, but it leaves a live
      // provider subscription pointing at a URL that no longer resolves. Say so
      // — silently succeeding is what let the leak accumulate unnoticed.
      //
      // Gated on the mount having HAD a subscription: the server also reports
      // `unsubscribed: false` when there was simply nothing to unregister, and
      // warning about a leak on every poll-mode delete would train people to
      // ignore the message that matters.
      if (res.unsubscribed === false && !!deleteTarget.state?.push_subscription_id) {
        showInfo(
          'Deleted, but the provider subscription may remain',
          'The webhook could not be unregistered. If notifications keep arriving, remove the subscription at the provider.',
        )
      } else {
        showSuccess('Deleted', deleteTarget.title)
      }
      void load(true)
    } catch (e: any) {
      showError('Delete failed', e?.message)
    } finally {
      setDeleteTarget(null)
    }
  }

  const columns: TableColumn<VirtualMount>[] = [
    {
      key: 'title',
      header: 'Mount',
      render: (m) => (
        <div className="flex items-center gap-2 group">
          <HardDrive className="w-4 h-4 text-teal-400 flex-shrink-0" />
          <div className="min-w-0">
            <div className="text-white font-medium group-hover:text-primary-300 transition-colors">
              {m.title}
            </div>
            <div className="text-xs text-zinc-500 truncate">
              {m.target_workspace}:{m.mount_path}
            </div>
          </div>
          {/* Affordance for the row link — the row is otherwise indistinguishable
              from the non-clickable tables elsewhere in the console. */}
          <ChevronRight className="w-4 h-4 text-zinc-600 opacity-0 group-hover:opacity-100 transition-opacity flex-shrink-0" />
        </div>
      ),
    },
    {
      key: 'writeback',
      header: 'Writeback',
      width: '120px',
      render: (m) => (
        <span className="text-xs text-zinc-300">{writeModeLabel(m.write_config)}</span>
      ),
    },
    {
      key: 'status',
      header: 'Status',
      width: '200px',
      render: (m) => {
        const kind = statusKind(m.state, m.enabled)
        const meta = STATUS_META[kind]
        const push = pushIndicator(m.state)
        return (
          <div className="flex flex-col gap-0.5">
            <span
              className={`flex items-center gap-1 px-2 py-0.5 text-xs rounded-full w-fit ${meta.cls}`}
              title={meta.hint}
            >
              <meta.Icon className={`w-3 h-3 ${kind === 'syncing' ? 'animate-spin' : ''}`} />
              {meta.label}
            </span>
            {push && (
              <span className={`flex items-center gap-1 text-[10px] ${push.cls}`} title={push.title}>
                <Webhook className="w-2.5 h-2.5 flex-shrink-0" />
                {push.text}
              </span>
            )}
            {(() => {
              const p = backfillProgress(m.state)
              if (!p) return null
              return (
                <span className={`text-[10px] ${p.done ? 'text-zinc-500' : 'text-blue-400'}`}>
                  {p.text}
                </span>
              )
            })()}
            {/* Epoch SECONDS, not milliseconds. `new Date(seconds)` is what
                rendered every live mount as "synced 1/21/1970, 5:02:27 PM" —
                the timestamp was fine, the unit was not. */}
            {m.state?.last_sync_at ? (
              <span
                className="text-[10px] text-zinc-500"
                title={formatAbsoluteSeconds(m.state.last_sync_at)}
              >
                synced {formatRelativeSeconds(m.state.last_sync_at)}
              </span>
            ) : null}
            {/* The attempt is what the scheduler backs off from, so on a failing
                mount it is the field that says when it will next be tried.
                Redundant while the two agree, so it is shown only when they
                differ — i.e. exactly when the last attempt did not succeed. */}
            {m.state?.last_attempt_at && m.state.last_attempt_at !== m.state.last_sync_at ? (
              <span
                className="text-[10px] text-zinc-500"
                title={formatAbsoluteSeconds(m.state.last_attempt_at)}
              >
                attempted {formatRelativeSeconds(m.state.last_attempt_at)}
              </span>
            ) : null}
            {m.state?.last_error && (
              <span className="text-[10px] text-red-400 truncate max-w-[180px]" title={m.state.last_error}>
                {m.state.last_error}
              </span>
            )}
          </div>
        )
      },
    },
    {
      key: 'test',
      header: 'Test',
      width: '90px',
      render: (m) => (
        <button
          onClick={() => setTestTarget(m)}
          className="flex items-center gap-1 px-2 py-1 text-xs text-zinc-300 hover:text-primary-300 hover:bg-white/10 rounded transition-colors"
          title="Test connection"
        >
          <Activity className="w-3.5 h-3.5" />
          Test
        </button>
      ),
    },
    {
      key: 'sync',
      header: 'Sync',
      width: '170px',
      render: (m) => {
        const busy = syncBusy(m)
        const unavailable = syncUnavailable(m)
        return (
          <div className="flex items-center gap-1">
            <button
              onClick={() => handleSync(m)}
              disabled={busy || !!unavailable}
              className="flex items-center gap-1 px-2 py-1 text-xs text-zinc-300 hover:text-primary-300 hover:bg-white/10 rounded transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
              title={unavailable ?? (isSyncing(m.state) ? 'A sync is already running' : 'Sync now')}
            >
              <RefreshCw className={`w-3.5 h-3.5 ${busy ? 'animate-spin' : ''}`} />
              {isSyncing(m.state) ? 'Syncing…' : 'Sync now'}
            </button>
            <button
              onClick={() => setRemapTarget(m)}
              disabled={busy || !!unavailable}
              className="flex items-center gap-1 px-2 py-1 text-xs text-zinc-400 hover:text-amber-300 hover:bg-white/10 rounded transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
              title={unavailable ?? 'Re-import every item through the current mapping function and folder hierarchy'}
            >
              <Wand2 className="w-3.5 h-3.5" />
              Remap
            </button>
          </div>
        )
      },
    },
  ]

  return (
    <div className="animate-fade-in">
      <div className="mb-6 flex justify-between items-start">
        <div>
          <h1 className="text-4xl font-bold text-white mb-2">Mounts</h1>
          <p className="text-zinc-400">Mount external subtrees into workspace paths</p>
        </div>
        <button
          onClick={() => {
            setEditing(undefined)
            setShowEditor(true)
          }}
          className="flex items-center gap-2 px-4 py-2 bg-primary-500 hover:bg-primary-600 text-white rounded-lg transition-colors"
        >
          <Plus className="w-5 h-5" /> New Mount
        </button>
      </div>

      {/* First load only. Once the table exists it stays on screen through
          every refresh — see `load(silent)`. */}
      {loading ? (
        <div className="animate-pulse space-y-2">
          <div className="h-10 bg-white/5 rounded-lg" />
          <div className="h-16 bg-white/5 rounded-lg" />
          <div className="h-16 bg-white/5 rounded-lg" />
        </div>
      ) : loadError ? (
        // Never fall through to the "No mounts yet" card on a failure: a mount
        // that exists but cannot be listed still syncs, and telling the operator
        // it does not exist invites them to recreate it.
        <GlassCard>
          <div className="text-center py-12">
            <AlertTriangle className="w-16 h-16 text-amber-400 mx-auto mb-4" />
            <h3 className="text-xl font-semibold text-white mb-2">Could not load mounts</h3>
            <p className="text-zinc-400 max-w-lg mx-auto">{loadError}</p>
            <p className="text-zinc-500 text-sm mt-2">
              Existing mounts keep syncing on the server regardless of this page.
            </p>
            <button
              onClick={() => void load()}
              className="mt-4 px-4 py-2 bg-white/5 hover:bg-white/10 border border-white/10 text-white text-sm rounded-lg transition-colors"
            >
              Retry
            </button>
          </div>
        </GlassCard>
      ) : mounts.length === 0 ? (
        <GlassCard>
          <div className="text-center py-12">
            <HardDrive className="w-16 h-16 text-zinc-500 mx-auto mb-4" />
            <h3 className="text-xl font-semibold text-white mb-2">No mounts yet</h3>
            {integrations.length === 0 ? (
              <>
                <p className="text-zinc-400">
                  A mount syncs through a connector, and no connector is configured yet.
                </p>
                <Link
                  to={`/${repo}/integrations`}
                  className="mt-4 inline-flex items-center gap-2 px-4 py-2 bg-primary-500 hover:bg-primary-600 text-white text-sm rounded-lg transition-colors"
                >
                  Set up a connector first
                </Link>
              </>
            ) : (
              <p className="text-zinc-400">Create a mount to bring external content into a workspace</p>
            )}
          </div>
        </GlassCard>
      ) : (
        <GlassCard className="flex-1 overflow-hidden flex flex-col">
          <ItemTable
            items={mounts}
            columns={columns}
            getItemId={(m) => m.name}
            getItemPath={(m) => m.path || m.name}
            getItemName={(m) => m.title}
            itemType="mount"
            // The row stays a one-liner; the detail view is where you zoom in.
            detailPath={(m) => `/${repo}/mounts/${encodeURIComponent(m.name)}`}
            onEdit={(m) => {
              setEditing(m)
              setShowEditor(true)
            }}
            onDelete={(m) => setDeleteTarget(m)}
          />
        </GlassCard>
      )}

      {showEditor && repo && (
        <MountEditor
          repo={repo}
          mount={editing}
          integrations={integrations}
          workspaces={workspaces}
          onClose={() => setShowEditor(false)}
          onSaved={() => void load(true)}
          onError={showError}
          onSuccess={showSuccess}
        />
      )}

      {testTarget && repo && (
        <div className="fixed inset-0 bg-black/50 backdrop-blur-sm flex items-center justify-center z-50 p-4 overscroll-none">
          <div className="bg-zinc-900 border border-white/10 rounded-xl shadow-2xl max-w-lg w-full max-h-[90vh] overflow-y-auto overscroll-contain">
            <div className="flex items-center justify-between p-6 border-b border-white/10">
              <div>
                <h2 className="text-xl font-bold text-white">Test connection</h2>
                <p className="text-xs text-zinc-500">{testTarget.title}</p>
              </div>
              <button
                onClick={() => setTestTarget(null)}
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
                  integrationFor(testTarget)?.path
                    ? undefined
                    : 'The connector backing this mount could not be found.'
                }
                request={{
                  integration_path: integrationFor(testTarget)?.path || testTarget.integration_ref,
                  account_id: testTarget.account_ref || undefined,
                  remote_root: testTarget.remote_root || undefined,
                  // Probe the surface this mount actually syncs (mail/calendar/
                  // files), not the adapter's default.
                  sync_config: testTarget.sync_config,
                }}
                onTested={() => void load(true)}
                onError={showError}
              />
            </div>
          </div>
        </div>
      )}

      <ConfirmDialog
        open={deleteTarget !== null}
        title="Delete mount"
        message={`Delete “${deleteTarget?.title}”? Materialized virtual nodes will be removed on next sync.`}
        variant="danger"
        confirmText="Delete"
        onConfirm={confirmDelete}
        onCancel={() => setDeleteTarget(null)}
      />
      <ConfirmDialog
        open={remapTarget !== null}
        title="Remap this mount"
        message={
          `Re-import every item in “${remapTarget?.title}” through the current mapping function ` +
          `and folder hierarchy.\n\n` +
          `Existing nodes are updated in place — ids, revision history and anything you added ` +
          `locally are kept — and moved if the hierarchy changed. Use this after changing the ` +
          `mapping function or the folder hierarchy; ordinary syncs skip unchanged items and ` +
          `will not pick those changes up.\n\n` +
          `A large mailbox runs in chunks over several minutes or hours. New items keep arriving ` +
          `throughout, and progress is shown in the row.`
        }
        confirmText="Remap"
        onConfirm={confirmRemap}
        onCancel={() => setRemapTarget(null)}
      />
      <ToastContainer toasts={toasts} onClose={closeToast} />
    </div>
  )
}
