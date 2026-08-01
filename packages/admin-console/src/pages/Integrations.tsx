// SPDX-License-Identifier: BSL-1.1

import { useEffect, useState, useCallback } from 'react'
import { useParams, useSearchParams } from 'react-router-dom'
import { Plug, Plus, Package, CheckCircle, XCircle, Users } from 'lucide-react'
import GlassCard from '../components/GlassCard'
import ConfirmDialog from '../components/ConfirmDialog'
import { ItemTable, type TableColumn } from '../components/ItemTable'
import { useToast, ToastContainer } from '../components/Toast'
import IntegrationEditor from '../components/integrations/IntegrationEditor'
import CapabilityChips from '../components/integrations/CapabilityChips'
import { integrationsApi, type Integration, type AdapterPackage } from '../api/integrations'

export default function Integrations() {
  const { repo } = useParams<{ repo: string }>()
  const [searchParams, setSearchParams] = useSearchParams()
  const [integrations, setIntegrations] = useState<Integration[]>([])
  const [adapters, setAdapters] = useState<AdapterPackage[]>([])
  const [loading, setLoading] = useState(true)
  const [editing, setEditing] = useState<Integration | undefined>(undefined)
  const [showEditor, setShowEditor] = useState(false)
  const [deleteTarget, setDeleteTarget] = useState<Integration | null>(null)
  const { toasts, error: showError, success: showSuccess, closeToast } = useToast()

  const load = useCallback(async () => {
    if (!repo) return
    setLoading(true)
    try {
      const [ints, pkgs] = await Promise.all([
        integrationsApi.listIntegrations(repo),
        integrationsApi.listAdapterPackages(repo),
      ])
      setIntegrations(ints)
      setAdapters(pkgs)
    } catch (e: any) {
      showError('Failed to load connectors', e?.message)
    } finally {
      setLoading(false)
    }
  }, [repo])

  useEffect(() => {
    load()
  }, [load])

  // The OAuth callback lands HERE, and usually inside the connect popup opened
  // by ConnectedAccounts. In that case this page must not render at all: it
  // hands the outcome to the opener and closes.
  //
  // Relaying beats having the opener poll `popup.location.search`, because this
  // very component strips those params from the URL a moment after mount — the
  // opener would race a cleanup it cannot see, and read an empty query string
  // as "no result". `window.name` is the name given to window.open(), and the
  // target origin is pinned so the message cannot be read cross-origin.
  const [relaying] = useState(
    () => typeof window !== 'undefined' && !!window.opener && window.name === 'raisin-oauth',
  )
  useEffect(() => {
    if (!relaying) return
    const connected = searchParams.get('connected')
    const error = searchParams.get('oauth_error')
    if (!connected && !error) return
    window.opener.postMessage(
      {
        type: 'raisin-oauth-result',
        connected,
        error,
        description: searchParams.get('oauth_error_description'),
      },
      window.location.origin,
    )
    window.close()
  }, [relaying, searchParams])

  // Non-popup fallback (the operator navigated here directly, or the opener is
  // gone): the callback redirects back with ?connected=<name>; surface a toast
  // and refresh once so the newly-connected account shows up.
  useEffect(() => {
    if (relaying) return
    const connected = searchParams.get('connected')
    if (connected) {
      showSuccess('Account connected', connected)
      searchParams.delete('connected')
      setSearchParams(searchParams, { replace: true })
      load()
    }
  }, [relaying, searchParams, setSearchParams, load, showSuccess])

  // ...and with ?oauth_error=<code> when the provider refused. The description
  // carries the provider's own diagnostic (e.g. Microsoft's AADSTS50194,
  // "not configured as a multi-tenant application"), which is what actually
  // tells the operator what to change.
  useEffect(() => {
    if (relaying) return
    const code = searchParams.get('oauth_error')
    if (code) {
      const detail = searchParams.get('oauth_error_description') || undefined
      showError(`Connect failed: ${code}`, detail)
      searchParams.delete('oauth_error')
      searchParams.delete('oauth_error_description')
      setSearchParams(searchParams, { replace: true })
    }
  }, [relaying, searchParams, setSearchParams, showError])

  function openNew() {
    setEditing(undefined)
    setShowEditor(true)
  }

  function openEdit(i: Integration) {
    setEditing(i)
    setShowEditor(true)
  }

  async function confirmDelete() {
    if (!repo || !deleteTarget) return
    try {
      await integrationsApi.deleteIntegration(repo, deleteTarget.name)
      showSuccess('Deleted', deleteTarget.title)
      load()
    } catch (e: any) {
      showError('Delete failed', e?.message)
    } finally {
      setDeleteTarget(null)
    }
  }

  const columns: TableColumn<Integration>[] = [
    {
      key: 'title',
      header: 'Connector',
      render: (i) => (
        <div className="flex items-center gap-2">
          <Plug className="w-4 h-4 text-sky-400" />
          <div>
            <div className="text-white font-medium">{i.title}</div>
            <div className="text-xs text-zinc-500">{i.provider_type}</div>
          </div>
        </div>
      ),
    },
    {
      key: 'capabilities',
      header: 'Capabilities',
      width: '260px',
      render: (i) => <CapabilityChips capabilities={i.capabilities} compact />,
    },
    {
      key: 'accounts',
      header: 'Accounts',
      width: '120px',
      render: (i) => (
        <span className="flex items-center gap-1 text-zinc-300 text-sm">
          <Users className="w-3.5 h-3.5" />
          {i.connected_accounts?.length || 0}
        </span>
      ),
    },
    {
      key: 'secret',
      header: 'Secret',
      width: '110px',
      render: (i) =>
        i.client_secret_set ? (
          <span className="text-green-400 text-xs">set</span>
        ) : (
          <span className="text-zinc-500 text-xs">—</span>
        ),
    },
    {
      key: 'enabled',
      header: 'Status',
      width: '120px',
      render: (i) =>
        i.enabled ? (
          <span className="flex items-center gap-1 px-2 py-0.5 bg-green-500/20 text-green-400 text-xs rounded-full w-fit">
            <CheckCircle className="w-3 h-3" /> Enabled
          </span>
        ) : (
          <span className="flex items-center gap-1 px-2 py-0.5 bg-gray-500/20 text-zinc-400 text-xs rounded-full w-fit">
            <XCircle className="w-3 h-3" /> Disabled
          </span>
        ),
    },
  ]

  return (
    <div className="animate-fade-in">
      <div className="mb-6 flex justify-between items-start">
        <div>
          <h1 className="text-4xl font-bold text-white mb-2">Connectors</h1>
          <p className="text-zinc-400">Connect external systems and manage OAuth accounts</p>
        </div>
        <button
          onClick={openNew}
          className="flex items-center gap-2 px-4 py-2 bg-primary-500 hover:bg-primary-600 text-white rounded-lg transition-colors"
        >
          <Plus className="w-5 h-5" /> New Connector
        </button>
      </div>

      {/* Installed adapter packages */}
      <div className="mb-6">
        <h2 className="text-sm font-semibold text-zinc-300 mb-3">Installed adapters</h2>
        {adapters.length === 0 ? (
          <p className="text-zinc-500 text-sm">
            No adapter packages installed (category “integrations”).
          </p>
        ) : (
          <div className="grid grid-cols-1 md:grid-cols-3 gap-3">
            {adapters.map((a) => (
              <div
                key={a.name}
                className="flex items-center gap-3 px-4 py-3 bg-white/5 border border-white/10 rounded-lg"
              >
                <Package className="w-5 h-5 text-sky-400 flex-shrink-0" />
                <div className="min-w-0">
                  <div className="text-white text-sm truncate">{a.title || a.name}</div>
                  <div className="text-zinc-500 text-xs truncate">
                    {a.name}
                    {a.version ? ` · v${a.version}` : ''}
                  </div>
                </div>
              </div>
            ))}
          </div>
        )}
      </div>

      {loading ? (
        <div className="text-center text-zinc-400 py-12">Loading…</div>
      ) : integrations.length === 0 ? (
        <GlassCard>
          <div className="text-center py-12">
            <Plug className="w-16 h-16 text-zinc-500 mx-auto mb-4" />
            <h3 className="text-xl font-semibold text-white mb-2">No connectors yet</h3>
            <p className="text-zinc-400">Create one to connect an external system</p>
          </div>
        </GlassCard>
      ) : (
        <GlassCard className="flex-1 overflow-hidden flex flex-col">
          <ItemTable
            items={integrations}
            columns={columns}
            getItemId={(i) => i.name}
            getItemPath={(i) => i.path || i.name}
            getItemName={(i) => i.title}
            itemType="integration"
            onEdit={openEdit}
            onDelete={(i) => setDeleteTarget(i)}
          />
        </GlassCard>
      )}

      {showEditor && repo && (
        <IntegrationEditor
          repo={repo}
          integration={editing}
          onClose={() => setShowEditor(false)}
          onSaved={load}
          onError={showError}
          onSuccess={showSuccess}
        />
      )}

      <ConfirmDialog
        open={deleteTarget !== null}
        title="Delete connector"
        message={`Delete “${deleteTarget?.title}”? Connected accounts will be lost.`}
        variant="danger"
        confirmText="Delete"
        onConfirm={confirmDelete}
        onCancel={() => setDeleteTarget(null)}
      />
      <ToastContainer toasts={toasts} onClose={closeToast} />
    </div>
  )
}
