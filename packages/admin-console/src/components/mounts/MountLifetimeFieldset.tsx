// SPDX-License-Identifier: BSL-1.1

const field =
  'w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-white/40 focus:outline-none focus:ring-2 focus:ring-primary-500 disabled:opacity-40'
const labelCls = 'block text-white text-sm font-medium mb-1.5'

interface Props {
  ephemeral: boolean
  /** Seconds, as typed. Blank means "cleared". */
  ttlSeconds: string
  reconcileDeletes: boolean
  allowEmptyReconcile: boolean
  onChange: {
    (key: 'ephemeral' | 'reconcile_deletes' | 'allow_empty_reconcile', value: boolean): void
    (key: 'ttl_seconds', value: string): void
  }
}

/**
 * How long synced NODES live, and what a full walk is allowed to delete.
 *
 * The sibling of `MountContentFieldset`, split on the same seam: that one is
 * about cached FILES, this one about a tenant's CONTENT. Both settings here
 * take effect on the next run against items that already exist — the expiry and
 * reconcile passes walk the stored index, so neither needs a remap.
 */
export default function MountLifetimeFieldset({
  ephemeral,
  ttlSeconds,
  reconcileDeletes,
  allowEmptyReconcile,
  onChange,
}: Props) {
  return (
    <fieldset className="border border-white/10 rounded-lg p-4 space-y-3">
      <legend className="px-2 text-sm font-semibold text-zinc-300">Node lifetime</legend>
      <label className="flex items-start gap-2 text-white text-sm">
        <input
          type="checkbox"
          className="w-4 h-4 rounded mt-0.5"
          checked={ephemeral}
          onChange={(e) => onChange('ephemeral', e.target.checked)}
        />
        <span>
          Expire synced nodes
          <span className="block text-xs text-zinc-500 mt-0.5">
            The mailbox pattern: items older than the TTL are DELETED from the workspace. A
            different subject from the byte cache above — this removes content, that removes
            copies.
          </span>
        </span>
      </label>
      <div>
        <label className={labelCls}>Nodes expire after (seconds)</label>
        <input
          type="number"
          className={field}
          min={60}
          disabled={!ephemeral}
          placeholder="604800"
          value={ttlSeconds}
          onChange={(e) => onChange('ttl_seconds', e.target.value)}
        />
        {ephemeral && !ttlSeconds && (
          <p className="text-xs text-amber-400 mt-1.5">
            Required: with no TTL nothing is expired at all, so the mount reads as self-cleaning
            and grows forever. The server refuses this combination.
          </p>
        )}
      </div>
      <label className="flex items-start gap-2 text-white text-sm">
        <input
          type="checkbox"
          className="w-4 h-4 rounded mt-0.5"
          checked={reconcileDeletes}
          onChange={(e) => onChange('reconcile_deletes', e.target.checked)}
        />
        <span>
          A full walk may delete nodes it did not see
          <span className="block text-xs text-zinc-500 mt-0.5">
            On for a provider whose listing is the whole truth. Turn it off only when it is not —
            IMAP lists mailboxes, not messages — but understand the trade: with it off, nothing on
            the walk path ever prunes this mount again.
          </span>
        </span>
      </label>
      <label className="flex items-start gap-2 text-white text-sm">
        <input
          type="checkbox"
          className="w-4 h-4 rounded mt-0.5"
          checked={allowEmptyReconcile}
          onChange={(e) => onChange('allow_empty_reconcile', e.target.checked)}
        />
        <span>
          An empty provider listing may delete the whole subtree
          <span className="block text-xs text-zinc-500 mt-0.5">
            Off by default. An empty listing is as often a permissions change or a provider
            hiccup as a genuinely emptied folder — and stale content is recoverable while deleted
            content is not.
          </span>
        </span>
      </label>
      {allowEmptyReconcile && (
        <p className="text-xs text-amber-400">
          One empty response from the provider will delete every node this mount owns. That is
          not recoverable from here.
        </p>
      )}
    </fieldset>
  )
}
