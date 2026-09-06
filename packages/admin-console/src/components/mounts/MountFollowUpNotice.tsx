// SPDX-License-Identifier: BSL-1.1

import type { SyncConfigFollowUp } from '../../api/integrations'

interface Props {
  followUp: SyncConfigFollowUp
  onRun: () => void
  onDismiss: () => void
}

/**
 * "That saved, and it does NOT reach the items already on this mount."
 *
 * Shown after a config write whose effect the engine will never apply on its
 * own: an ordinary sync skips an item whose etag has not changed without even
 * calling the mapper, and a delta feed carries only what changed at the
 * provider — which for a config change is nothing.
 *
 * Two rules it holds:
 *
 * - It is a PANEL, not a toast. The whole failure being designed out here is a
 *   setting that reads as applied and is not, and a message that scrolls away
 *   reproduces it.
 * - The action is offered, never taken. A remap re-materializes every item on
 *   the mount and writes a node revision each — exactly the cost the etag skip
 *   exists to avoid — so it is the operator's call. (It is also safe now: the
 *   materializer carries engine-derived properties, thumbnails and extraction
 *   artifacts included, across a rebuild.)
 *
 * The reason text is authored server-side beside the rule that decides it, so
 * there is no second copy of the reasoning here to drift out of step.
 */
export default function MountFollowUpNotice({ followUp, onRun, onDismiss }: Props) {
  return (
    <div className="border border-amber-500/30 bg-amber-500/5 rounded-lg p-3 space-y-2">
      <p className="text-xs text-amber-300">
        <span className="font-medium">
          {followUp.fields.join(', ')} changed — existing items are unaffected.
        </span>{' '}
        {followUp.reason}.
      </p>
      <div className="flex items-center gap-2">
        <button
          type="button"
          onClick={onRun}
          className="px-3 py-1.5 bg-amber-500/20 hover:bg-amber-500/30 border border-amber-500/40 text-amber-200 text-xs rounded-lg transition-colors"
        >
          {followUp.action === 'remap' ? 'Remap now' : 'Run a full sync'}
        </button>
        <button
          type="button"
          onClick={onDismiss}
          className="px-3 py-1.5 text-xs text-zinc-400 hover:text-zinc-200 transition-colors"
        >
          Later
        </button>
      </div>
    </div>
  )
}
