// SPDX-License-Identifier: BSL-1.1

import { useEffect, useState, useCallback } from 'react'
import { useParams } from 'react-router-dom'
import { HardDrive, Plus, RefreshCw, CheckCircle, Loader2, ShieldAlert, AlertTriangle, Activity, Webhook, X, Wand2 } from 'lucide-react'
import GlassCard from '../components/GlassCard'
import ConfirmDialog from '../components/ConfirmDialog'
import { ItemTable, type TableColumn } from '../components/ItemTable'
import { useToast, ToastContainer } from '../components/Toast'
import MountEditor from '../components/integrations/MountEditor'
import TestConnectionPanel from '../components/integrations/TestConnectionPanel'
import { integrationsApi, type VirtualMount, type Integration, type MountState } from '../api/integrations'
import { workspacesApi } from '../api/workspaces'

type StatusKind = 'ok' | 'syncing' | 'auth_required' | 'misconfigured' | 'degraded' | 'idle'

function statusKind(state?: MountState): StatusKind {
  const s = (state?.status || '').toLowerCase()
  if (s === 'ok' || s === 'synced') return 'ok'
  if (s === 'syncing') return 'syncing'
  if (s === 'auth_required') return 'auth_required'
  // Checked before 'degraded': the operator action is completely different.
  // Degraded means "flaky, it may recover"; misconfigured means "it will never
  // succeed until you change something", so it must not be diluted into the
  // generic failure bucket.
  if (s === 'misconfigured') return 'misconfigured'
  if (s === 'degraded' || s === 'error') return 'degraded'
  if ((state?.consecutive_failures || 0) > 0) return 'degraded'
  return 'idle'
}

const STATUS_META: Record<StatusKind, { label: string; cls: string; Icon: typeof CheckCircle }> = {
  ok: { label: 'OK', cls: 'bg-green-500/20 text-green-400', Icon: CheckCircle },
  syncing: { label: 'Syncing', cls: 'bg-blue-500/20 text-blue-400', Icon: Loader2 },
  auth_required: { label: 'Auth required', cls: 'bg-yellow-500/20 text-yellow-400', Icon: ShieldAlert },
  misconfigured: { label: 'Misconfigured', cls: 'bg-orange-500/20 text-orange-400', Icon: AlertTriangle },
  degraded: { label: 'Degraded', cls: 'bg-red-500/20 text-red-400', Icon: AlertTriangle },
  idle: { label: 'Idle', cls: 'bg-gray-500/20 text-zinc-400', Icon: HardDrive },
}

/**
 * Progress of an in-flight full walk (first sync, backfill or remap).
 *
 * A large mailbox imports over many chunked runs and can take hours, so the row
 * has to show movement — otherwise a working import is indistinguishable from a
 * stalled one and the natural response is to hit Sync again.
 */
function backfillProgress(state?: MountState): { text: string; done: boolean } | null {
  const inFlight = !!state?.backfill_cursor
  const count = state?.backfill_items_done ?? 0
  if (!inFlight && !count) return null
  const n = count.toLocaleString()
  return inFlight
    ? { text: `Importing… ${n} items so far`, done: false }
    : { text: `Imported ${n} items`, done: true }
}

/**
 * Compact read-only descriptor of a mount's push (webhook) subscription, derived
 * from the engine-managed `state.push_*` fields. Returns null when the mount has
 * no push subscription (poll-only or never subscribed).
 */
function pushIndicator(state?: MountState): { text: string; cls: string; title?: string } | null {
  const status = (state?.push_status || '').toLowerCase()
  if (!status) return null
  const failed = status === 'failed' || status === 'error' || !!state?.push_last_error
  const active = status === 'active'
  const cls = failed
    ? 'text-red-400'
    : active
      ? 'text-green-400'
      : 'text-zinc-400'
  let text = `Push: ${failed ? 'failed' : status}`
  if (!failed && state?.push_expires_at) {
    const d = new Date(state.push_expires_at)
    if (!Number.isNaN(d.getTime())) text += `, renews ${d.toLocaleDateString()}`
  }
  return { text, cls, title: state?.push_last_error || undefined }
}

export default function Mounts() {
  const { repo } = useParams<{ repo: string }>()
  const [mounts, setMounts] = useState<VirtualMount[]>([])
  const [integrations, setIntegrations] = useState<Integration[]>([])
  const [workspaces, setWorkspaces] = useState<string[]>([])
  const [loading, setLoading] = useState(true)
  const [editing, setEditing] = useState<VirtualMount | undefined>(undefined)
  const [showEditor, setShowEditor] = useState(false)
  const [deleteTarget, setDeleteTarget] = useState<VirtualMount | null>(null)
  const [syncingId, setSyncingId] = useState<string | null>(null)
  const [testTarget, setTestTarget] = useState<VirtualMount | null>(null)
  const [remapTarget, setRemapTarget] = useState<VirtualMount | null>(null)
  /** Set when the mounts list itself failed, so "broken" never renders as "empty". */
  const [loadError, setLoadError] = useState<string | null>(null)
  const { toasts, error: showError, success: showSuccess, closeToast } = useToast()

  /** The connector node backing a mount (matched by integration_ref path). */
  const integrationFor = (m: VirtualMount) => integrations.find((i) => i.path === m.integration_ref)

  const load = useCallback(async () => {
    if (!repo) return
    setLoading(true)
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
    setLoading(false)
  }, [repo])

  useEffect(() => {
    load()
  }, [load])

  // Poll while any mount has a walk in flight. A chunked import runs for
  // minutes to hours, and a progress number that only moves on manual refresh
  // is not progress. Polling stops as soon as nothing is running, so an idle
  // page costs nothing.
  const anyRunning = mounts.some((m) => !!m.state?.backfill_cursor)
  useEffect(() => {
    if (!anyRunning) return
    const t = window.setInterval(load, 5000)
    return () => window.clearInterval(t)
  }, [anyRunning, load])

  async function handleSync(m: VirtualMount) {
    if (!repo || !m.id) return
    setSyncingId(m.id)
    try {
      // Only request a delta sync when the connector advertises a delta API;
      // otherwise the engine can only full-reconcile.
      const mode = integrationFor(m)?.capabilities?.supports_changes === true ? 'delta' : 'full'
      const res = await integrationsApi.syncMount(repo, m.id, mode)
      if (res.status === 'already_running') {
        showSuccess('Already syncing', m.title)
      } else {
        showSuccess('Sync queued', m.title)
      }
      // Give the job a moment, then refresh state.
      setTimeout(load, 1200)
    } catch (e: any) {
      showError('Sync failed', e?.message)
    } finally {
      setSyncingId(null)
    }
  }

  async function confirmRemap() {
    if (!repo || !remapTarget?.id) return
    const target = remapTarget
    setRemapTarget(null)
    setSyncingId(target.id!)
    try {
      await integrationsApi.syncMount(repo, target.id!, 'remap')
      showSuccess(
        'Remap queued',
        'Every item will be re-imported through the current mapping. Progress appears in the row.',
      )
      setTimeout(load, 1200)
    } catch (e: any) {
      showError('Remap failed', e?.message)
    } finally {
      setSyncingId(null)
    }
  }

  async function confirmDelete() {
    if (!repo || !deleteTarget) return
    try {
      await integrationsApi.deleteMount(repo, deleteTarget.name)
      showSuccess('Deleted', deleteTarget.title)
      load()
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
        <div className="flex items-center gap-2">
          <HardDrive className="w-4 h-4 text-teal-400" />
          <div>
            <div className="text-white font-medium">{m.title}</div>
            <div className="text-xs text-zinc-500">
              {m.target_workspace}:{m.mount_path}
            </div>
          </div>
        </div>
      ),
    },
    {
      key: 'writeback',
      header: 'Writeback',
      width: '120px',
      render: (m) => (
        <span className="text-xs text-zinc-300">{m.write_config?.writeback || 'off'}</span>
      ),
    },
    {
      key: 'status',
      header: 'Status',
      width: '200px',
      render: (m) => {
        const kind = statusKind(m.state)
        const meta = STATUS_META[kind]
        const push = pushIndicator(m.state)
        return (
          <div className="flex flex-col gap-0.5">
            <span className={`flex items-center gap-1 px-2 py-0.5 text-xs rounded-full w-fit ${meta.cls}`}>
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
            {m.state?.last_sync_at && (
              <span className="text-[10px] text-zinc-500">
                synced {new Date(m.state.last_sync_at).toLocaleString()}
              </span>
            )}
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
      render: (m) => (
        <div className="flex items-center gap-1">
          <button
            onClick={() => handleSync(m)}
            disabled={syncingId === m.id}
            className="flex items-center gap-1 px-2 py-1 text-xs text-zinc-300 hover:text-primary-300 hover:bg-white/10 rounded transition-colors disabled:opacity-50"
            title="Sync now"
          >
            <RefreshCw className={`w-3.5 h-3.5 ${syncingId === m.id ? 'animate-spin' : ''}`} />
            Sync now
          </button>
          <button
            onClick={() => setRemapTarget(m)}
            disabled={syncingId === m.id}
            className="flex items-center gap-1 px-2 py-1 text-xs text-zinc-400 hover:text-amber-300 hover:bg-white/10 rounded transition-colors disabled:opacity-50"
            title="Re-import every item through the current mapping function and folder hierarchy"
          >
            <Wand2 className="w-3.5 h-3.5" />
            Remap
          </button>
        </div>
      ),
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

      {loading ? (
        <div className="text-center text-zinc-400 py-12">Loading…</div>
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
              onClick={load}
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
            <p className="text-zinc-400">Create a mount to bring external content into a workspace</p>
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
          onSaved={load}
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
                onTested={load}
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
