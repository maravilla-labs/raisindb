// SPDX-License-Identifier: BSL-1.1

/**
 * How a secret's origin is rendered — the distinction the page exists to make.
 *
 * **Auto-vaulted**: the operator did not create it. Writing a node property
 * whose schema says `encrypted: true` moved the value into the store and left a
 * `secret://…` reference behind, so the secret's life is the node's: it is
 * copied on branch fork, tombstoned when the node is deleted, and rotating it
 * by hand desynchronises nothing but is unusual — the ordinary way to change it
 * is to edit the field on the node.
 *
 * **Operator**: created here or by the CLI, referenced by whatever chooses to
 * reference it. Nothing else manages its lifecycle.
 */

import { Link } from 'react-router-dom'
import { FileKey2, User } from 'lucide-react'
import type { SecretMetadata } from '../../api/secrets'
import { ownerOf } from '../../api/secrets'
import type { ResolvedOwner } from './useOwnerNodePaths'

interface OwnerLabelProps {
  secret: SecretMetadata
  repo: string
  branch: string
  /** `undefined` while the probe is in flight, `null` once it found nothing. */
  resolved?: ResolvedOwner | null
}

export default function OwnerLabel({ secret, repo, branch, resolved }: OwnerLabelProps) {
  const owner = ownerOf(secret)

  if (!owner) {
    return (
      <span className="inline-flex items-center gap-1.5 text-xs text-zinc-400">
        <User className="w-3.5 h-3.5" />
        Operator secret
      </span>
    )
  }

  return (
    <span className="inline-flex items-center gap-1.5 text-xs text-zinc-300 min-w-0">
      <FileKey2 className="w-3.5 h-3.5 shrink-0 text-sky-300" />
      {resolved ? (
        <Link
          to={`/${repo}/content/${branch}/${resolved.workspace}${resolved.path}`}
          className="truncate font-mono text-sky-300 hover:underline"
          title={`${resolved.workspace}:${resolved.path}`}
        >
          {resolved.workspace}:{resolved.path}
        </Link>
      ) : (
        // Either still probing or genuinely unresolvable — the node may have
        // been deleted while older revisions still reference this secret. The
        // id is the honest fallback, never an error.
        <span className="truncate font-mono text-zinc-500" title={owner.nodeId}>
          {resolved === null ? 'node ' : ''}
          {owner.nodeId}
        </span>
      )}
      {owner.field && (
        <span className="text-zinc-500 shrink-0">
          · <span className="font-mono text-zinc-400">{owner.field}</span>
        </span>
      )}
    </span>
  )
}
