// SPDX-License-Identifier: BSL-1.1

import { useCallback, useEffect, useState } from 'react'
import { ChevronRight, Folder, Loader2, RefreshCw, Search, X } from 'lucide-react'
import {
  integrationsApi,
  type BrowseItem,
  type BrowseKind,
  type SyncConfig,
} from '../../api/integrations'

interface RemotePickerProps {
  repo: string
  integrationPath: string
  accountId?: string
  /** What class of container to list (`folder`, `calendar`, `site`, …). */
  kind: BrowseKind | string
  /** Mount config selecting *what* to browse — principal, drive scope, site. */
  syncConfig?: SyncConfig
  /** Show a search box; only for kinds whose provider supports a query. */
  searchable?: boolean
  /**
   * Container the listing STARTS in, when the kind is scoped by something the
   * operator already chose — document libraries live under a SharePoint site, so
   * that picker opens with `parent_id` set to the site rather than at a root
   * that does not exist for that kind.
   */
  rootParentId?: string
  title: string
  onSelect: (item: BrowseItem) => void
  onClose: () => void
}

interface Crumb {
  id: string | undefined
  name: string
}

/**
 * Modal picker over a connector's remote containers, backed by the adapter's
 * optional `browse` operation (§2.10 of the adapter contract).
 *
 * Purely an input affordance: whatever is picked is written into the mount as an
 * id, and the caller always keeps a manual-entry path. That is deliberate —
 * `browse` results are a convenience, never an authorization statement. A
 * provider may list a container the credential cannot actually read (Graph's
 * directory listing enumerates users regardless of which mailboxes the account
 * may open), so a picker must never become the only way to configure a mount.
 */
export default function RemotePicker({
  repo,
  integrationPath,
  accountId,
  kind,
  syncConfig,
  searchable,
  rootParentId,
  title,
  onSelect,
  onClose,
}: RemotePickerProps) {
  const [items, setItems] = useState<BrowseItem[]>([])
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [unsupported, setUnsupported] = useState(false)
  const [trail, setTrail] = useState<Crumb[]>([{ id: rootParentId, name: title }])
  const [nextCursor, setNextCursor] = useState<string | null>(null)
  const [query, setQuery] = useState('')
  const [submittedQuery, setSubmittedQuery] = useState('')

  const parentId = trail[trail.length - 1]?.id

  const load = useCallback(
    async (cursor?: string) => {
      setLoading(true)
      setError(null)
      try {
        const res = await integrationsApi.browse(repo, {
          integration_path: integrationPath,
          account_id: accountId,
          kind,
          parent_id: parentId,
          query: submittedQuery || undefined,
          cursor,
          sync_config: syncConfig,
        })
        if (!res.supported) {
          setUnsupported(true)
          setItems([])
          return
        }
        // Append on paging, replace on a fresh listing.
        setItems((prev) => (cursor ? [...prev, ...res.items] : res.items))
        setNextCursor(res.next_cursor || null)
        if (!res.ok && res.error) setError(res.error.message || res.error.code)
      } catch (e: any) {
        setError(e?.message || 'Could not reach the connector')
      } finally {
        setLoading(false)
      }
    },
    [repo, integrationPath, accountId, kind, parentId, submittedQuery, syncConfig],
  )

  useEffect(() => {
    void load()
    // Paging is driven explicitly by the "Load more" button, so the cursor is
    // deliberately not a dependency here.
  }, [load])

  function openChild(item: BrowseItem) {
    setTrail((t) => [...t, { id: item.id, name: item.name }])
    setNextCursor(null)
  }

  function jumpTo(index: number) {
    setTrail((t) => t.slice(0, index + 1))
    setNextCursor(null)
  }

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4">
      <div className="w-full max-w-2xl max-h-[80vh] flex flex-col bg-zinc-900 border border-white/10 rounded-xl shadow-xl">
        <div className="flex items-center justify-between px-5 py-4 border-b border-white/10">
          <h2 className="text-lg font-semibold text-white">{title}</h2>
          <button
            type="button"
            onClick={onClose}
            className="p-1 text-zinc-400 hover:text-white"
            aria-label="Close"
          >
            <X className="w-5 h-5" />
          </button>
        </div>

        {/* Breadcrumbs — the only way back up a hierarchical listing. */}
        <div className="flex items-center flex-wrap gap-1 px-5 py-2 text-xs text-zinc-400 border-b border-white/10">
          {trail.map((c, i) => (
            <span key={`${c.id ?? 'root'}-${i}`} className="flex items-center gap-1">
              {i > 0 && <ChevronRight className="w-3 h-3" />}
              <button
                type="button"
                className={i === trail.length - 1 ? 'text-zinc-200' : 'hover:text-zinc-200 underline'}
                onClick={() => jumpTo(i)}
                disabled={i === trail.length - 1}
              >
                {c.name}
              </button>
            </span>
          ))}
        </div>

        {searchable && (
          <form
            className="flex gap-2 px-5 py-3 border-b border-white/10"
            onSubmit={(e) => {
              e.preventDefault()
              setSubmittedQuery(query.trim())
            }}
          >
            <div className="relative flex-1">
              <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-zinc-500" />
              <input
                className="w-full pl-9 pr-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white text-sm"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="Search…"
              />
            </div>
            <button
              type="submit"
              className="px-3 py-2 bg-white/5 hover:bg-white/10 border border-white/10 text-white text-sm rounded-lg"
            >
              Search
            </button>
          </form>
        )}

        <div className="flex-1 overflow-y-auto px-5 py-3">
          {unsupported && (
            <p className="text-sm text-zinc-400">
              This connector cannot list its remote containers. Close this and enter the id
              manually.
            </p>
          )}

          {error && !unsupported && (
            <div className="mb-3 px-3 py-2 rounded-lg bg-red-500/10 border border-red-500/20 text-sm text-red-300">
              {error}
            </div>
          )}

          {!unsupported && !loading && items.length === 0 && !error && (
            <p className="text-sm text-zinc-500">Nothing here.</p>
          )}

          <ul className="divide-y divide-white/5">
            {items.map((item) => (
              <li key={item.id} className="flex items-center gap-2 py-2">
                <Folder className="w-4 h-4 text-zinc-500 flex-shrink-0" />
                <button
                  type="button"
                  className="flex-1 text-left min-w-0"
                  onClick={() => onSelect(item)}
                >
                  <span className="block text-sm text-white truncate">{item.name}</span>
                  {item.hint && (
                    <span className="block text-xs text-zinc-500 truncate">{item.hint}</span>
                  )}
                </button>
                {item.has_children && (
                  <button
                    type="button"
                    onClick={() => openChild(item)}
                    className="p-1 text-zinc-400 hover:text-white"
                    title="Open"
                    aria-label={`Open ${item.name}`}
                  >
                    <ChevronRight className="w-4 h-4" />
                  </button>
                )}
              </li>
            ))}
          </ul>

          {loading && (
            <div className="flex items-center gap-2 py-3 text-sm text-zinc-400">
              <Loader2 className="w-4 h-4 animate-spin" /> Loading…
            </div>
          )}

          {nextCursor && !loading && (
            <button
              type="button"
              onClick={() => void load(nextCursor)}
              className="mt-3 px-3 py-2 bg-white/5 hover:bg-white/10 border border-white/10 text-white text-sm rounded-lg"
            >
              Load more
            </button>
          )}
        </div>

        <div className="flex items-center justify-between px-5 py-3 border-t border-white/10">
          <button
            type="button"
            onClick={() => void load()}
            className="flex items-center gap-1 text-xs text-zinc-400 hover:text-white"
          >
            <RefreshCw className="w-3 h-3" /> Refresh
          </button>
          <button
            type="button"
            onClick={onClose}
            className="px-3 py-2 bg-white/5 hover:bg-white/10 border border-white/10 text-white text-sm rounded-lg"
          >
            Cancel
          </button>
        </div>
      </div>
    </div>
  )
}
