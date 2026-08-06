// SPDX-License-Identifier: BSL-1.1

import { useState } from 'react'
import { AlertTriangle } from 'lucide-react'
import { integrationsApi, type MountState } from '../../api/integrations'

interface Props {
  repo: string
  mountId: string
  state?: MountState
  /** Refetch the mount so the banner disappears once the drain releases it. */
  onReleased: () => void
}

/**
 * A tripped blast-radius rail, and the one control that clears it.
 *
 * Without this the block is invisible AND unclearable from the console: the
 * engine parks every pending delete and waits for `writeback_confirm_token`,
 * which lived only in the mount node's `state` object. The symptom an operator
 * would actually see is "deletes silently stopped happening" — the failure the
 * rails exist to make loud, made quiet again by the UI not showing it.
 *
 * Rendered whenever `writeback_blocked` is present, at the top of the mount, and
 * not folded into the writeback fieldset: this is not configuration, it is an
 * incident waiting for a decision.
 */
export default function MountWritebackBlock({ repo, mountId, state, onReleased }: Props) {
  const block = state?.writeback_blocked
  const [releasing, setReleasing] = useState(false)
  const [error, setError] = useState<string | null>(null)
  if (!block) return null

  const confirmed = state?.writeback_confirm_token === block.token

  async function release() {
    if (!block) return
    setReleasing(true)
    setError(null)
    try {
      await integrationsApi.confirmWriteback(repo, mountId, block.token)
      onReleased()
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    } finally {
      setReleasing(false)
    }
  }

  return (
    <div className="border border-amber-500/40 bg-amber-500/10 rounded-lg p-4 space-y-3">
      <div className="flex items-start gap-2">
        <AlertTriangle className="w-4 h-4 text-amber-400 flex-shrink-0 mt-0.5" />
        <div className="min-w-0">
          <h3 className="text-amber-200 text-sm font-semibold">
            {block.deletes} delete{block.deletes === 1 ? '' : 's'} parked — writeback blocked
          </h3>
          {/*
            The engine's own message, verbatim. It names the rail that tripped
            and the numbers behind it, and rewriting it here would be a second
            explanation free to drift from the one in the logs.
          */}
          <p className="text-xs text-amber-200/80 mt-1 break-words">{block.reason}</p>
        </div>
      </div>

      {/*
        Said plainly, because the instinct on seeing "blocked" is that something
        was lost. Nothing was: the deletes are parked, the provider was not
        touched, and reads never stopped.
      */}
      <p className="text-xs text-zinc-400">
        Nothing was sent to the provider and nothing was lost. Reading is unaffected — only
        outbound writes are held. Review the local deletes before releasing: what is parked is a
        request to delete {block.deletes} object{block.deletes === 1 ? '' : 's'} at the provider.
      </p>

      {confirmed ? (
        <p className="text-xs text-green-400">
          Released. The next drain pushes exactly this batch — a batch that has changed since is
          re-checked and blocked again.
        </p>
      ) : (
        <div className="flex items-center gap-3">
          <button
            className="px-3 py-1.5 rounded-lg bg-amber-500/20 border border-amber-500/40 text-amber-100 text-sm hover:bg-amber-500/30 disabled:opacity-50"
            disabled={releasing}
            onClick={release}
          >
            {releasing ? 'Releasing…' : `Release these ${block.deletes}`}
          </button>
          {/*
            The scope of the button, at the button. The token fingerprints this
            exact batch, so this is not "allow deletes from now on" — which is
            the reading that would make an operator click it once and never think
            about the rail again.
          */}
          <span className="text-xs text-zinc-500">
            Releases this batch only, not future ones.
          </span>
        </div>
      )}
      {error && <p className="text-xs text-red-400">{error}</p>}
    </div>
  )
}
