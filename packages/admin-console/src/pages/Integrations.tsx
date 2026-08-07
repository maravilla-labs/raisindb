// SPDX-License-Identifier: BSL-1.1

import { useEffect, useState, useCallback } from 'react'
import { useParams, useSearchParams } from 'react-router-dom'
import { Plug, Plus, Package, CheckCircle, XCircle, Users } from 'lucide-react'
import GlassCard from '../components/GlassCard'
import ConfirmDialog from '../components/ConfirmDialog'
import { ItemTable, type TableColumn } from '../components/ItemTable'
import { useToast, ToastContainer } from '../components/Toast'
import IntegrationEditor from '../components/integrations/IntegrationEditor'
import AddConnectorDialog from '../components/integrations/AddConnectorDialog'
import CapabilityChips from '../components/integrations/CapabilityChips'
import { integrationsApi, type Integration, type AdapterPackage } from '../api/integrations'

export default function Integrations() {
  const { repo } = useParams<{ repo: string }>()
  const [searchParams, setSearchParams] = useSearchParams()
  const [integrations, setIntegrations] = useState<Integration[]>([])
  const [adapters, setAdapters] = useState<AdapterPackage[]>([])
  const [templates, setTemplates] = useState<Integration[]>([])
  const [showAdd, setShowAdd] = useState(false)
  const [loading, setLoading] = useState(true)
  const [editing, setEditing] = useState<Integration | undefined>(undefined)
  const [showEditor, setShowEditor] = useState(false)
  const [deleteTarget, setDeleteTarget] = useState<Integration | null>(null)
  const [loadError, setLoadError] = useState<string | null>(null)
  // Mounts are loaded only to warn on delete — a connector's mounts silently
  // stop syncing when it goes away, and the operator deserves the count.
  const [mounts, setMounts] = useState<Array<{ integration_ref: string; title: string }>>([])
  const { toasts, error: showError, success: showSuccess, closeToast } = useToast()

  const load = useCallback(async () => {
    if (!repo) return
    setLoading(true)
    // allSettled, and the CONNECTOR list decides pass/fail on its own. A failed
    // load must never fall through to "No connectors yet": the natural response
    // to that empty state is to recreate a connector that already exists —
    // which means re-entering a client secret and re-consenting every account.
    // (Mounts.tsx documents the same fix for the same bug.)
    const [ints, pkgs, tmpl, mts] = await Promise.allSettled([
      integrationsApi.listIntegrations(repo),
      integrationsApi.listAdapterPackages(repo),
      integrationsApi.listConnectorTemplates(repo),
      integrationsApi.listMounts(repo),
    ])
    if (ints.status === 'fulfilled') {
      setIntegrations(ints.value)
      setLoadError(null)
    } else {
      setLoadError((ints.reason as any)?.message || 'The connector list could not be loaded.')
    }
    // Decorations: their absence degrades the page, never blanks it.
    if (pkgs.status === 'fulfilled') setAdapters(pkgs.value)
    if (tmpl.status === 'fulfilled') setTemplates(tmpl.value)
    if (mts.status === 'fulfilled') {
      setMounts(
        mts.value.map((m: any) => ({ integration_ref: m.integration_ref, title: m.title })),
      )
    }
    setLoading(false)
  }, [repo])

  useEffect(() => {
    load()
  }, [load])

  // The connect popup never loads this page: the OAuth callback returns a
  // self-contained page that postMessages its result to the opener and closes
  // (see oauth_callback.rs). These two effects are the NO-OPENER fallback —
  // consent finished in a normal tab, so the callback navigated here with the
  // outcome in the query string instead.
  useEffect(() => {
    const connected = searchParams.get('connected')
    if (connected) {
      showSuccess('Account connected', connected)
      searchParams.delete('connected')
      setSearchParams(searchParams, { replace: true })
      load()
    }
  }, [searchParams, setSearchParams, load, showSuccess])

  // ...and with ?oauth_error=<code> when the provider refused. The description
  // carries the provider's own diagnostic (e.g. Microsoft's AADSTS50194,
  // "not configured as a multi-tenant application"), which is what actually
  // tells the operator what to change.
  useEffect(() => {
    const code = searchParams.get('oauth_error')
    if (code) {
      const detail = searchParams.get('oauth_error_description') || undefined
      showError(`Connect failed: ${code}`, detail)
      searchParams.delete('oauth_error')
      searchParams.delete('oauth_error_description')
      setSearchParams(searchParams, { replace: true })
    }
  }, [searchParams, setSearchParams, showError])

  /**
   * Adding a connector starts from a template, never from a blank form.
   *
   * The blank form cannot set `config_type` / `connection_config_type` — it only
   * reads them — so a hand-built connector silently lost its schema-driven
   * config form AND its "Add connection" button. Seeding from a template is what
   * makes a second instance of a shipped connector (a second Microsoft tenant,
   * say) actually usable.
   */
  function openNew() {
    setShowAdd(true)
  }

  /** A freshly minted instance goes straight into the editor for credentials. */
  function onConnectorCreated(instance: Integration) {
    setShowAdd(false)
    setEditing(instance)
    setShowEditor(true)
    load()
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
          <Plus className="w-5 h-5" /> Add connector
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
      ) : loadError ? (
        <GlassCard>
          <div className="text-center py-12">
            <XCircle className="w-16 h-16 text-red-400/70 mx-auto mb-4" />
            <h3 className="text-xl font-semibold text-white mb-2">
              The connector list could not be loaded
            </h3>
            <p className="text-zinc-400">{loadError}</p>
            <p className="text-zinc-500 text-sm mt-1">
              Your connectors still exist — do not recreate them.
            </p>
            <button
              onClick={() => void load()}
              className="mt-4 px-4 py-2 bg-white/5 hover:bg-white/10 border border-white/10 text-white text-sm rounded-lg transition-colors"
            >
              Retry
            </button>
          </div>
        </GlassCard>
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

      {showAdd && repo && (
        <AddConnectorDialog
          repo={repo}
          templates={templates}
          existing={integrations}
          onClose={() => setShowAdd(false)}
          onCreated={onConnectorCreated}
          onError={showError}
        />
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
        message={(() => {
          if (!deleteTarget) return ''
          const dependent = mounts.filter((m) => m.integration_ref === deleteTarget.path)
          if (dependent.length === 0) {
            return `Delete “${deleteTarget.title}”? Connected accounts will be lost.`
          }
          const names = dependent
            .slice(0, 3)
            .map((m) => `“${m.title}”`)
            .join(', ')
          const more = dependent.length > 3 ? ` and ${dependent.length - 3} more` : ''
          return (
            `Delete “${deleteTarget.title}”? Connected accounts will be lost, and ` +
            `${dependent.length} mount${dependent.length === 1 ? '' : 's'} ` +
            `(${names}${more}) sync${dependent.length === 1 ? 's' : ''} through this ` +
            `connector and will STOP SYNCING until re-pointed at another one.`
          )
        })()}
        variant="danger"
        confirmText="Delete"
        onConfirm={confirmDelete}
        onCancel={() => setDeleteTarget(null)}
      />
      <ToastContainer toasts={toasts} onClose={closeToast} />
    </div>
  )
}
