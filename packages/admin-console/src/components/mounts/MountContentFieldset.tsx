// SPDX-License-Identifier: BSL-1.1

const field =
  'w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-white/40 focus:outline-none focus:ring-2 focus:ring-primary-500 disabled:opacity-40'
const labelCls = 'block text-white text-sm font-medium mb-1.5'

interface Props {
  cacheContent: boolean
  /** Seconds, as typed. Blank means "cleared", which is the engine default. */
  contentTtl: string
  /** Whether the CONNECTOR declares `accepts_content` — never a mount setting. */
  acceptsContent: boolean
  onChange: {
    (key: 'cache_content', value: boolean): void
    (key: 'content_ttl_seconds', value: string): void
  }
}

/**
 * Everything about a mount's file BYTES: whether this deployment may hold
 * copies, how long it keeps them, and whether the connector will take bytes
 * back.
 *
 * Split out of `MountSyncSettings` for the 300-line convention, along the seam
 * the settings themselves have — bytes here, node lifetime there. Conflating
 * the two is the mistake the engine's own comments warn about twice: one
 * deletes cached FILES, the other deletes a tenant's CONTENT.
 */
export default function MountContentFieldset({
  cacheContent,
  contentTtl,
  acceptsContent,
  onChange,
}: Props) {
  return (
    <fieldset className="border border-white/10 rounded-lg p-4 space-y-3">
      <legend className="px-2 text-sm font-semibold text-zinc-300">File content</legend>
      <label className="flex items-start gap-2 text-white text-sm">
        <input
          type="checkbox"
          className="w-4 h-4 rounded mt-0.5"
          checked={cacheContent}
          onChange={(e) => onChange('cache_content', e.target.checked)}
        />
        <span>
          Cache file bytes on this deployment
          <span className="block text-xs text-zinc-500 mt-0.5">
            Off, the mount stays metadata-only and its files are findable by name alone. On, bytes
            are fetched when something needs them — text extraction, thumbnails, previews — which
            means holding copies of someone else's storage on this disk. A mount can be far larger
            than this deployment wants to keep.
          </span>
        </span>
      </label>
      <div>
        <label className={labelCls}>Cached bytes expire after (seconds)</label>
        <input
          type="number"
          className={field}
          min={0}
          disabled={!cacheContent}
          placeholder="1800 (engine default)"
          value={contentTtl}
          onChange={(e) => onChange('content_ttl_seconds', e.target.value)}
        />
        <p className="text-xs text-zinc-500 mt-1.5">
          Blank is the engine default of 30 minutes, not "forever" — enabling the cache must not
          be able to silently mirror a whole drive. <span className="font-mono">0</span> drops the
          bytes as soon as processing is done. Deletes only cached FILES; the nodes stay.
        </p>
      </div>
      {/*
        Read-only, and shown even though it cannot be changed here. This is the
        setting whose ABSENCE cost a production drive mount a day of "uploads
        succeed, bytes never leave": the engine sends a file's bytes only when
        the adapter declares `accepts_content`, and that is a fact about the
        provider — whether its objects have bytes at all — not an operator
        preference. There is deliberately no mount switch for it.
      */}
      <div className="flex items-baseline gap-3 pt-1 border-t border-white/5">
        <span className="text-zinc-500 text-xs w-40 shrink-0">Outbound bytes</span>
        <span className="min-w-0 flex-1 text-xs">
          {acceptsContent ? (
            <span className="text-green-400">
              connector accepts content — a local file edit uploads its bytes
            </span>
          ) : (
            <span className="text-zinc-400">
              connector does not declare <span className="font-mono">accepts_content</span> for
              this mount's resource, so a mirror sends metadata only. Not a mount setting: point
              the mount at a resource whose adapter declares it (the connector's{' '}
              <span className="font-mono">resource</span> in the mount editor), then re-run Test
              connection.
            </span>
          )}
        </span>
      </div>
    </fieldset>
  )
}
