// SPDX-License-Identifier: BSL-1.1

import { useState } from 'react'
import { Download } from 'lucide-react'
import { integrationsApi } from '../api/integrations'
import type { Node as NodeType } from '../api/nodes'

interface Props {
  repo: string
  branch: string
  workspace: string
  node: NodeType | null
  /** Reload so the fetched `file` appears on the node. */
  onFetched: () => void
}

/**
 * "Fetch content" for a mount-owned `raisin:Asset` that has no bytes yet.
 *
 * A sync writes attachment METADATA only — name, mime type, size — because
 * downloading every attachment of every synced message would multiply a mailbox
 * import by whole documents, and most attachments are never opened. So
 * `file == null` on a mount-owned asset means exactly "not fetched yet", and
 * without this control the console offers no way to say otherwise: the endpoint
 * existed and nothing in the UI called it.
 *
 * Renders only for the case it serves. An asset that already has its bytes, or
 * one that was never mount-owned, shows nothing at all — a permanently disabled
 * button on every ordinary asset would be worse than no button.
 */
export default function AssetContentFetch({ repo, branch, workspace, node, onFetched }: Props) {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const props = (node?.properties ?? {}) as Record<string, unknown>
  // Mount-owned is the load-bearing half of the test: this fetch resolves the
  // mount from the node's own provenance, so an asset without it has nothing to
  // fetch FROM and the request would fail with a validation error.
  const isMountOwned = typeof props.__mount_id === 'string' && !!props.__external_id
  const hasBytes = props.file != null
  if (!node || node.node_type !== 'raisin:Asset' || !isMountOwned || hasBytes) return null

  async function fetchNow() {
    if (!node?.id) return
    setBusy(true)
    setError(null)
    try {
      const res = await integrationsApi.fetchMountContent(repo, branch, workspace, node.id)
      // `already_present` is an ordinary answer — another viewer fetched it
      // first — so it refreshes exactly like a fetch rather than reporting a
      // problem.
      if (res.status === 'stored' || res.status === 'already_present') onFetched()
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="mb-3 flex items-center gap-3 flex-wrap px-3 py-2 rounded-lg border border-white/10 bg-white/5">
      <Download className="w-4 h-4 text-zinc-400 flex-shrink-0" />
      <span className="text-xs text-zinc-400 min-w-0">
        Synced from the provider as metadata only — the file itself has not been downloaded yet.
      </span>
      <button
        className="px-3 py-1.5 rounded-lg bg-primary-500/20 border border-primary-500/40 text-primary-100 text-sm hover:bg-primary-500/30 disabled:opacity-50"
        disabled={busy}
        onClick={fetchNow}
      >
        {busy ? 'Fetching…' : 'Fetch content'}
      </button>
      {error && <span className="text-xs text-red-400 break-words">{error}</span>}
    </div>
  )
}
