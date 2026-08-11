// SPDX-License-Identifier: BSL-1.1

/**
 * Every version of one secret — metadata only, because that is all the server
 * has a route for.
 *
 * Tombstones are listed, not filtered: a deleted secret's earlier versions stay
 * readable through a pinned `secret://name@N`, so an older node revision still
 * resolves. Hiding them would make the page claim a value is gone when the
 * store can still hand it out.
 */

import { useCallback, useEffect, useState } from 'react'
import { History, RotateCw, Trash2 } from 'lucide-react'
import GlassCard from '../GlassCard'
import { dateFromIso, formatAbsolute, formatRelative } from '../../utils/time'
import { secretsApi, type SecretMetadata } from '../../api/secrets'
import OwnerLabel from './OwnerLabel'
import type { ResolvedOwner } from './useOwnerNodePaths'

interface SecretDetailProps {
  repo: string
  branch: string
  secret: SecretMetadata
  resolvedOwner?: ResolvedOwner | null
  /** Bumped by the page after a write, so the version list refetches. */
  reloadToken: number
  onRotate: () => void
  onDelete: () => void
  onError: (title: string, message?: string) => void
}

export default function SecretDetail({
  repo,
  branch,
  secret,
  resolvedOwner,
  reloadToken,
  onRotate,
  onDelete,
  onError,
}: SecretDetailProps) {
  const [versions, setVersions] = useState<SecretMetadata[]>([])
  const [loading, setLoading] = useState(true)

  const load = useCallback(async () => {
    setLoading(true)
    try {
      const res = await secretsApi.versions(repo, branch, secret.name)
      setVersions(res.versions || [])
    } catch (e: any) {
      onError('Could not load the secret’s versions', e?.message)
      setVersions([])
    } finally {
      setLoading(false)
    }
    // `reloadToken` is not read in the body — it is here precisely so a write
    // on the page refetches this list. `onError` is deliberately absent:
    // useToast recreates it every render, which would refetch in a loop.
  }, [repo, branch, secret.name, reloadToken])

  useEffect(() => {
    load()
  }, [load])

  return (
    <GlassCard className="p-4 space-y-4">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <h2 className="font-medium text-white font-mono truncate" title={secret.name}>
            {secret.name}
          </h2>
          <div className="mt-1">
            <OwnerLabel secret={secret} repo={repo} branch={branch} resolved={resolvedOwner} />
          </div>
          <p className="text-xs text-zinc-500 mt-2 font-mono">secret://{secret.name}</p>
        </div>
        <div className="flex gap-2 shrink-0">
          <button
            type="button"
            onClick={onRotate}
            className="px-3 py-1.5 rounded-md border border-white/10 text-white text-sm hover:bg-white/5"
          >
            <RotateCw className="w-4 h-4 inline mr-1.5" />
            Rotate
          </button>
          <button
            type="button"
            onClick={onDelete}
            className="px-3 py-1.5 rounded-md border border-rose-400/30 text-rose-300 text-sm hover:bg-rose-500/10"
          >
            <Trash2 className="w-4 h-4 inline mr-1.5" />
            Delete
          </button>
        </div>
      </div>

      <div className="space-y-2">
        <h3 className="text-xs uppercase tracking-wider text-zinc-500 flex items-center gap-1.5">
          <History className="w-3.5 h-3.5" />
          Versions
        </h3>
        {loading ? (
          <p className="text-sm text-zinc-400">Loading…</p>
        ) : versions.length === 0 ? (
          <p className="text-sm text-zinc-400">No versions.</p>
        ) : (
          <table className="w-full text-sm">
            <thead>
              <tr className="text-left text-xs text-zinc-500">
                <th className="py-1 pr-3 font-normal">Version</th>
                <th className="py-1 pr-3 font-normal">Written</th>
                <th className="py-1 pr-3 font-normal">By</th>
                <th className="py-1 pr-3 font-normal">Key</th>
                <th className="py-1 pr-3 font-normal">Size</th>
              </tr>
            </thead>
            <tbody>
              {versions.map((v) => {
                const written = dateFromIso(v.rotated_at || v.created_at)
                return (
                  <tr
                    key={`${v.version}-${v.revision?.timestamp_ms}-${v.revision?.counter}`}
                    className="border-t border-white/5"
                  >
                    <td className="py-1.5 pr-3 font-mono text-zinc-300">
                      @{v.version}
                      {v.deleted && (
                        <span className="ml-2 text-xs px-1.5 py-0.5 rounded border border-rose-400/30 text-rose-300">
                          tombstone
                        </span>
                      )}
                      {v.rotated_at && !v.deleted && (
                        <span className="ml-2 text-xs px-1.5 py-0.5 rounded border border-white/10 text-zinc-400">
                          rotated
                        </span>
                      )}
                    </td>
                    <td className="py-1.5 pr-3 text-zinc-400" title={formatAbsolute(written)}>
                      {formatRelative(written)}
                    </td>
                    <td className="py-1.5 pr-3 text-zinc-400 truncate">{v.created_by}</td>
                    <td className="py-1.5 pr-3 text-zinc-500 font-mono">#{v.key_id}</td>
                    {/* The sealed envelope's SIZE — not its content, and the
                        only thing about the ciphertext that is safe to show. */}
                    <td className="py-1.5 pr-3 text-zinc-500">
                      {v.deleted ? '—' : `${v.ciphertext_len} B`}
                    </td>
                  </tr>
                )
              })}
            </tbody>
          </table>
        )}
      </div>
    </GlassCard>
  )
}
