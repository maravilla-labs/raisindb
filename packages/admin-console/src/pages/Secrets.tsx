// SPDX-License-Identifier: BSL-1.1

/**
 * The secret store, per repository branch.
 *
 * Two kinds of entry share one namespace and the page's job is to keep them
 * apart — see `OwnerLabel` for what the distinction means:
 *
 * - **auto-vaulted**, named `node/{node_id}/{field}`, minted by writing a node
 *   property whose schema says `encrypted: true`;
 * - **operator**, created here or by the CLI.
 *
 * No value is ever displayed, and none is held in state beyond the submit that
 * carries it — there is no server route that returns one, so there is nothing
 * here that expects one either.
 */

import { useCallback, useEffect, useMemo, useState } from 'react'
import { useParams } from 'react-router-dom'
import { KeyRound, Plus, ShieldAlert } from 'lucide-react'
import GlassCard from '../components/GlassCard'
import ConfirmDialog from '../components/ConfirmDialog'
import { useToast, ToastContainer } from '../components/Toast'
import OwnerLabel from '../components/secrets/OwnerLabel'
import SecretDetail from '../components/secrets/SecretDetail'
import SecretValueDialog from '../components/secrets/SecretValueDialog'
import { useOwnerNodePaths } from '../components/secrets/useOwnerNodePaths'
import { dateFromIso, formatAbsolute, formatRelative } from '../utils/time'
import { isAutoVaulted, ownerOf, secretsApi, type SecretMetadata } from '../api/secrets'

type Filter = 'all' | 'vaulted' | 'operator' | 'deleted'

const FILTERS: Array<{ key: Filter; label: string }> = [
  { key: 'all', label: 'All' },
  { key: 'vaulted', label: 'Auto-vaulted' },
  { key: 'operator', label: 'Operator' },
  { key: 'deleted', label: 'Deleted' },
]

export default function Secrets() {
  const { repo, branch } = useParams<{ repo: string; branch?: string }>()
  const currentBranch = branch || 'main'
  const [secrets, setSecrets] = useState<SecretMetadata[]>([])
  const [loading, setLoading] = useState(true)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [filter, setFilter] = useState<Filter>('all')
  const [selected, setSelected] = useState<string | null>(null)
  const [dialog, setDialog] = useState<{ mode: 'create' | 'rotate'; name?: string } | null>(null)
  const [busy, setBusy] = useState(false)
  const [deleteTarget, setDeleteTarget] = useState<SecretMetadata | null>(null)
  // Bumped after any write so the open detail panel refetches its versions.
  const [reloadToken, setReloadToken] = useState(0)
  const { toasts, error: showError, success: showSuccess, closeToast } = useToast()

  const load = useCallback(async () => {
    if (!repo) return
    setLoading(true)
    try {
      setSecrets(await secretsApi.list(repo, currentBranch))
      setLoadError(null)
    } catch (e: any) {
      // A failed load must not fall through to the empty state: "no secrets
      // yet" invites creating one that already exists, and a duplicate name is
      // a second version of a live credential.
      setLoadError(e?.message || 'The secret list could not be loaded.')
    } finally {
      setLoading(false)
    }
  }, [repo, currentBranch])

  useEffect(() => {
    load()
  }, [load])

  const ownerIds = useMemo(
    () =>
      Array.from(
        new Set(
          secrets
            .map((s) => ownerOf(s)?.nodeId)
            .filter((id): id is string => typeof id === 'string'),
        ),
      ),
    [secrets],
  )
  const owners = useOwnerNodePaths(repo, currentBranch, ownerIds)

  const counts = useMemo(
    () => ({
      all: secrets.length,
      vaulted: secrets.filter((s) => isAutoVaulted(s) && !s.deleted).length,
      operator: secrets.filter((s) => !isAutoVaulted(s) && !s.deleted).length,
      deleted: secrets.filter((s) => s.deleted).length,
    }),
    [secrets],
  )

  const visible = useMemo(() => {
    const rows = secrets.filter((s) => {
      switch (filter) {
        case 'vaulted':
          return isAutoVaulted(s) && !s.deleted
        case 'operator':
          return !isAutoVaulted(s) && !s.deleted
        case 'deleted':
          return s.deleted
        default:
          return true
      }
    })
    // Live before retired, then by name — a tombstone is kept visible but never
    // pushed above a secret that is actually in use.
    return rows.sort((a, b) =>
      a.deleted === b.deleted ? a.name.localeCompare(b.name) : a.deleted ? 1 : -1,
    )
  }, [secrets, filter])

  const current = secrets.find((s) => s.name === selected) || null

  const submitValue = async (name: string, value: string) => {
    if (!repo || !dialog) return
    setBusy(true)
    try {
      const res =
        dialog.mode === 'create'
          ? await secretsApi.put(repo, currentBranch, name, value)
          : await secretsApi.rotate(repo, currentBranch, name, value)
      showSuccess(
        dialog.mode === 'create' ? 'Secret written' : 'Secret rotated',
        `${res.name} is now at version ${res.version}`,
      )
      setDialog(null)
      setSelected(res.name)
      setReloadToken((n) => n + 1)
      await load()
    } catch (e: any) {
      showError(dialog.mode === 'create' ? 'Could not write the secret' : 'Rotation failed', e?.message)
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold text-white flex items-center gap-2">
            <KeyRound className="w-6 h-6 text-primary-400" />
            Secrets
          </h1>
          <p className="text-sm text-zinc-400 mt-1">
            Credentials sealed in the store for <span className="font-mono">{repo}</span> ·{' '}
            <span className="font-mono">{currentBranch}</span>, referenced from properties as{' '}
            <code className="font-mono">secret://name</code>.
          </p>
        </div>
        <button
          type="button"
          onClick={() => setDialog({ mode: 'create' })}
          className="px-3 py-1.5 rounded-md bg-primary-500/20 border border-primary-400/40 text-white text-sm"
        >
          <Plus className="w-4 h-4 inline mr-1.5" />
          New secret
        </button>
      </div>

      <p className="text-xs text-amber-200/70 flex items-start gap-2">
        <ShieldAlert className="w-4 h-4 mt-0.5 shrink-0" />
        <span>
          Values are write-only. Nothing on this page — or anywhere in the API — can read one back;
          they are only ever resolved server-side when something uses them. Replace a value you have
          lost by rotating it.
        </span>
      </p>

      <div className="flex gap-2">
        {FILTERS.map((f) => (
          <button
            key={f.key}
            type="button"
            onClick={() => setFilter(f.key)}
            className={`px-3 py-1 rounded-md border text-xs transition ${
              filter === f.key
                ? 'border-primary-400/40 bg-primary-500/20 text-white'
                : 'border-white/10 text-zinc-400 hover:text-white'
            }`}
          >
            {f.label} <span className="text-zinc-500">{counts[f.key]}</span>
          </button>
        ))}
      </div>

      {loadError ? (
        <GlassCard className="p-6 text-sm text-rose-300">
          {loadError}{' '}
          <button type="button" onClick={load} className="underline hover:text-rose-200">
            Retry
          </button>
        </GlassCard>
      ) : loading ? (
        <p className="text-sm text-zinc-400">Loading…</p>
      ) : visible.length === 0 ? (
        <GlassCard className="p-6 text-sm text-zinc-400">
          {filter === 'all'
            ? 'No secrets in this branch yet. Create one here, or write a node property whose schema marks the field encrypted — that vaults the value automatically.'
            : 'Nothing matches this filter.'}
        </GlassCard>
      ) : (
        <div className="grid grid-cols-1 lg:grid-cols-3 gap-4">
          <div className="space-y-2">
            {visible.map((s) => {
              const owner = ownerOf(s)
              const written = dateFromIso(s.rotated_at || s.created_at)
              return (
                <button
                  key={s.name}
                  type="button"
                  onClick={() => setSelected(s.name)}
                  className={`w-full text-left p-3 rounded-lg border transition ${
                    selected === s.name
                      ? 'border-sky-400/40 bg-sky-500/10'
                      : 'border-white/10 hover:border-white/20'
                  } ${s.deleted ? 'opacity-60' : ''}`}
                >
                  <div className="flex items-center justify-between gap-2">
                    <span
                      className={`font-mono text-sm truncate ${
                        s.deleted ? 'text-zinc-400 line-through' : 'text-white'
                      }`}
                      title={s.name}
                    >
                      {owner ? owner.field || s.name : s.name}
                    </span>
                    <span className="flex items-center gap-1.5 shrink-0">
                      {s.deleted && (
                        <span className="text-xs px-2 py-0.5 rounded border border-rose-400/30 text-rose-300">
                          deleted
                        </span>
                      )}
                      <span className="text-xs px-2 py-0.5 rounded border border-white/10 text-zinc-400">
                        v{s.version}
                      </span>
                    </span>
                  </div>
                  <div className="mt-1">
                    <OwnerLabel
                      secret={s}
                      repo={repo!}
                      branch={currentBranch}
                      resolved={owners[owner?.nodeId ?? '']}
                    />
                  </div>
                  <div className="text-xs text-zinc-500 mt-1" title={formatAbsolute(written)}>
                    {s.rotated_at ? 'rotated' : 'written'} {formatRelative(written)} by{' '}
                    {s.created_by}
                  </div>
                </button>
              )
            })}
          </div>

          <div className="lg:col-span-2">
            {!current ? (
              <GlassCard className="p-6 text-sm text-zinc-400">
                Select a secret to see its versions.
              </GlassCard>
            ) : (
              <SecretDetail
                repo={repo!}
                branch={currentBranch}
                secret={current}
                resolvedOwner={owners[ownerOf(current)?.nodeId ?? '']}
                reloadToken={reloadToken}
                onRotate={() => setDialog({ mode: 'rotate', name: current.name })}
                onDelete={() => setDeleteTarget(current)}
                onError={showError}
              />
            )}
          </div>
        </div>
      )}

      <SecretValueDialog
        open={!!dialog}
        mode={dialog?.mode ?? 'create'}
        name={dialog?.name}
        busy={busy}
        onSubmit={submitValue}
        onCancel={() => setDialog(null)}
      />

      <ConfirmDialog
        open={!!deleteTarget}
        title={`Delete ${deleteTarget?.name ?? ''}?`}
        message={
          deleteTarget && isAutoVaulted(deleteTarget)
            ? 'This secret belongs to a node — deleting it here leaves the node holding a reference that no longer resolves. Earlier versions stay readable through a pinned reference, so older revisions of the node still work.'
            : 'A tombstone is appended. Anything resolving secret://name will fail from now on; earlier versions stay readable through a pinned secret://name@N, so older node revisions still resolve.'
        }
        confirmText="Delete"
        variant="danger"
        onConfirm={async () => {
          if (!repo || !deleteTarget) return
          try {
            await secretsApi.remove(repo, currentBranch, deleteTarget.name)
            showSuccess('Secret deleted', `${deleteTarget.name} is tombstoned`)
            setReloadToken((n) => n + 1)
            await load()
          } catch (e: any) {
            showError('Could not delete the secret', e?.message)
          } finally {
            setDeleteTarget(null)
          }
        }}
        onCancel={() => setDeleteTarget(null)}
      />
      <ToastContainer toasts={toasts} onClose={closeToast} />
    </div>
  )
}
