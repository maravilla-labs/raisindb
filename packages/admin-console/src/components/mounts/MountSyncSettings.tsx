// SPDX-License-Identifier: BSL-1.1

import { useMemo, useState } from 'react'
import { Info } from 'lucide-react'
import MountContentFieldset from './MountContentFieldset'
import MountFollowUpNotice from './MountFollowUpNotice'
import MountLifetimeFieldset from './MountLifetimeFieldset'
import {
  integrationsApi,
  type Capabilities,
  type SyncConfig,
  type SyncConfigFollowUp,
  type VirtualMount,
} from '../../api/integrations'

const field =
  'w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-white/40 focus:outline-none focus:ring-2 focus:ring-primary-500 disabled:opacity-40'
const labelCls = 'block text-white text-sm font-medium mb-1.5'

interface Props {
  repo: string
  mount: VirtualMount
  /** The connector's cached capabilities. Absent = never probed. */
  caps?: Capabilities
  onSaved: () => void
  onError: (title: string, message?: string) => void
  onSuccess: (title: string, message?: string) => void
  /**
   * Enqueue a re-materialization the operator has chosen. Owned by the page,
   * which already holds the remap confirmation dialog and the run feed — this
   * panel decides WHETHER to offer one, never triggers it.
   */
  onRequestSync: (mode: 'remap' | 'full') => void
}

/** The editable subset, as strings so a blank input can mean "cleared". */
interface Draft {
  mode: string
  interval_seconds: string
  cache_content: boolean
  content_ttl_seconds: string
  ephemeral: boolean
  ttl_seconds: string
  reconcile_deletes: boolean
  allow_empty_reconcile: boolean
  max_items_per_sync: string
  include_patterns: string
  exclude_patterns: string
}

const num = (v: number | null | undefined) => (v === null || v === undefined ? '' : String(v))

function draftOf(c: SyncConfig | undefined): Draft {
  return {
    mode: c?.mode || 'poll',
    interval_seconds: num(c?.interval_seconds),
    cache_content: c?.cache_content === true,
    content_ttl_seconds: num(c?.content_ttl_seconds),
    ephemeral: c?.ephemeral === true,
    ttl_seconds: num(c?.ttl_seconds),
    // Engine default is ON, so an absent key is `true` — reading it as false
    // would show every ordinary mount as one that never prunes.
    reconcile_deletes: c?.reconcile_deletes !== false,
    allow_empty_reconcile: c?.allow_empty_reconcile === true,
    max_items_per_sync: num(c?.max_items_per_sync),
    include_patterns: (c?.include_patterns || []).join('\n'),
    exclude_patterns: (c?.exclude_patterns || []).join('\n'),
  }
}

/** One textarea line per pattern, blanks dropped — the mount editor's shape. */
const lines = (t: string) =>
  t
    .split(/[\n,]/)
    .map((v) => v.trim())
    .filter(Boolean)

/**
 * Editable `sync_config`, shown against what THIS connector can actually do.
 *
 * Two rules it exists to hold, both learned from a drive mount that reported
 * `ok` on every line while its bytes never left:
 *
 * 1. A control the connector cannot honour is shown DISABLED WITH THE REASON,
 *    never hidden. A missing control is indistinguishable from a broken
 *    feature, which is the failure this panel was built to end.
 * 2. Saving goes through the field-scoped PATCH, never the mount editor's
 *    whole-node save — that one replaces every property from a page-load-old
 *    copy and can drop the engine's `state` (delta cursor, push subscription,
 *    backfill resume point).
 *
 * The server validates the same rules again and refuses; this is the honest
 * surface, not the enforcement point.
 */
export default function MountSyncSettings({
  repo,
  mount,
  caps,
  onSaved,
  onError,
  onSuccess,
  onRequestSync,
}: Props) {
  const initial = useMemo(() => draftOf(mount.sync_config), [mount.sync_config])
  const [draft, setDraft] = useState<Draft>(initial)
  const [saving, setSaving] = useState(false)
  /**
   * What the LAST save said has to be run for the change to reach items that
   * are already synced. Kept until the operator acts on it or edits again —
   * a toast that scrolls away is exactly how the change silently never lands.
   */
  const [followUp, setFollowUp] = useState<SyncConfigFollowUp | null>(null)

  const set = <K extends keyof Draft>(key: K, value: Draft[K]) =>
    setDraft((d) => ({ ...d, [key]: value }))

  // `supports_push` is what the engine's `subscription::ensure` reads, and a
  // `webhook` mount is never polled — so on a connector without it the mount
  // would have no path to a sync at all. `hybrid` still polls, so it stays
  // offered everywhere. Unknown capabilities count as "cannot", the same
  // conservative default the write fieldset holds.
  const pushKnown = caps !== undefined && caps.supports_push !== undefined
  const canPush = caps?.supports_push === true
  const webhookReason = !pushKnown
    ? 'capabilities not probed yet — run Test connection'
    : !canPush
      ? 'this connector does not declare supports_push'
      : ''

  const dirty = (Object.keys(initial) as (keyof Draft)[]).filter((k) => draft[k] !== initial[k])

  function patchOf(): Partial<SyncConfig> {
    const p: Record<string, unknown> = {}
    for (const key of dirty) {
      const v = draft[key]
      if (typeof v === 'boolean') p[key] = v
      else if (key === 'mode') p[key] = v
      else if (key === 'include_patterns' || key === 'exclude_patterns') p[key] = lines(v)
      // A blank TTL CLEARS it — the one place null is meaningful. A blank
      // interval or item cap is not a state the engine has, so it is skipped
      // rather than sent as a null the server would refuse.
      else if (v === '') {
        if (key === 'content_ttl_seconds' || key === 'ttl_seconds') p[key] = null
      } else p[key] = Number(v)
    }
    return p as Partial<SyncConfig>
  }

  async function save() {
    // Addressed by node id, which the mount route resolves. A mount loaded from
    // the API always carries one; the guard is here because the type allows an
    // unsaved draft, which never reaches this panel.
    if (!mount.id) return
    setSaving(true)
    try {
      const res = await integrationsApi.patchMountSyncConfig(repo, mount.id, patchOf())
      onSuccess('Sync settings saved', res.changed.join(', '))
      setFollowUp(res.follow_up ?? null)
      onSaved()
    } catch (e: any) {
      // The server's refusals name the field AND the reason (a webhook mode a
      // connector cannot serve, a TTL with no cache behind it). Show it whole;
      // summarising it is how an operator ends up back at "it just does
      // nothing".
      onError('Save refused', e?.message)
    } finally {
      setSaving(false)
    }
  }

  return (
    <div className="space-y-4">
      <div>
        <label className={labelCls}>Mode</label>
        <select
          className={field}
          value={draft.mode}
          onChange={(e) => set('mode', e.target.value)}
        >
          <option value="poll">poll — scheduled, every interval</option>
          <option value="hybrid">hybrid — scheduled AND provider push</option>
          <option value="webhook" disabled={!canPush}>
            webhook — push only{webhookReason ? ` — ${webhookReason}` : ''}
          </option>
        </select>
        {!canPush && (
          <p className="flex items-start gap-1.5 text-xs text-zinc-500 mt-1.5">
            <Info className="w-3.5 h-3.5 flex-shrink-0 mt-0.5" />
            Push-only is unavailable: {webhookReason}. A webhook mount is never polled, so on a
            connector that cannot push it would go permanently silent.
          </p>
        )}
      </div>

      <div>
        <label className={labelCls}>Poll interval (seconds)</label>
        <input
          type="number"
          className={field}
          min={30}
          disabled={draft.mode === 'webhook'}
          placeholder="300"
          value={draft.interval_seconds}
          onChange={(e) => set('interval_seconds', e.target.value)}
        />
        {draft.mode === 'webhook' && (
          <p className="text-xs text-zinc-500 mt-1.5">
            Not used: a webhook mount syncs when the provider pings it.
          </p>
        )}
      </div>

      <MountContentFieldset
        cacheContent={draft.cache_content}
        contentTtl={draft.content_ttl_seconds}
        acceptsContent={caps?.accepts_content === true}
        onChange={set}
      />

      <MountLifetimeFieldset
        ephemeral={draft.ephemeral}
        ttlSeconds={draft.ttl_seconds}
        reconcileDeletes={draft.reconcile_deletes}
        allowEmptyReconcile={draft.allow_empty_reconcile}
        onChange={set}
      />

      <div className="grid grid-cols-2 gap-4">
        <div>
          <label className={labelCls}>Include patterns</label>
          <textarea
            className={field}
            rows={3}
            placeholder={'*.pdf\n*.docx'}
            value={draft.include_patterns}
            onChange={(e) => set('include_patterns', e.target.value)}
          />
        </div>
        <div>
          <label className={labelCls}>Exclude patterns</label>
          <textarea
            className={field}
            rows={3}
            placeholder={'archive/*'}
            value={draft.exclude_patterns}
            onChange={(e) => set('exclude_patterns', e.target.value)}
          />
        </div>
      </div>
      <p className="text-xs text-zinc-500 -mt-1">
        One pattern per line. Changing these does not reach the mount on its own — the delta feed
        only carries what changed at the provider, so a newly included item is never re-offered.
        The panel says so after saving and offers the walk that applies it.
      </p>

      <div>
        <label className={labelCls}>Max items per sync</label>
        <input
          type="number"
          className={field}
          min={1}
          placeholder="500"
          value={draft.max_items_per_sync}
          onChange={(e) => set('max_items_per_sync', e.target.value)}
        />
      </div>

      {followUp && (
        <MountFollowUpNotice
          followUp={followUp}
          onRun={() => {
            onRequestSync(followUp.action)
            setFollowUp(null)
          }}
          onDismiss={() => setFollowUp(null)}
        />
      )}

      <div className="flex items-center gap-3">
        <button
          type="button"
          disabled={saving || dirty.length === 0}
          onClick={() => void save()}
          className="px-4 py-2 bg-primary-500 hover:bg-primary-600 text-white text-sm rounded-lg transition-colors disabled:opacity-40"
        >
          {saving ? 'Saving…' : `Save${dirty.length ? ` (${dirty.length})` : ''}`}
        </button>
        {dirty.length > 0 && (
          <button
            type="button"
            onClick={() => setDraft(initial)}
            className="px-4 py-2 bg-white/5 hover:bg-white/10 border border-white/10 text-white text-sm rounded-lg transition-colors"
          >
            Discard
          </button>
        )}
        <span className="text-xs text-zinc-500">
          Merges only the fields you changed; the sync cursor and push subscription are untouched.
        </span>
      </div>
    </div>
  )
}
