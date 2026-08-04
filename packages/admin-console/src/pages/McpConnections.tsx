// SPDX-License-Identifier: BSL-1.1

import { useCallback, useEffect, useState } from 'react'
import { useParams } from 'react-router-dom'
import { PlugZap, Plus, RefreshCw, Activity, ShieldAlert } from 'lucide-react'
import GlassCard from '../components/GlassCard'
import ConfirmDialog from '../components/ConfirmDialog'
import { useToast, ToastContainer } from '../components/Toast'
import ConnectionAuthPanel from '../components/mcp-connections/ConnectionAuthPanel'
import DiscoveredToolsTable from '../components/mcp-connections/DiscoveredToolsTable'
import {
  mcpConnectionsApi,
  type DiscoveredTool,
  type McpConnection,
  type TestReport,
} from '../api/mcp-connections'

/** Status chip derived from health + credential state. */
function statusOf(c: McpConnection): { label: string; className: string } {
  if (!c.enabled) return { label: 'disabled', className: 'text-white/40 border-white/10' }
  if (c.auth_kind === 'oauth' && !c.oauth_connected)
    return { label: 'needs auth', className: 'text-amber-300 border-amber-400/30' }
  if (c.auth_kind === 'static' && !c.credential_set)
    return { label: 'needs credential', className: 'text-amber-300 border-amber-400/30' }
  return { label: 'ready', className: 'text-emerald-300 border-emerald-400/30' }
}

export default function McpConnections() {
  const { repo } = useParams<{ repo: string }>()
  const [connections, setConnections] = useState<McpConnection[]>([])
  const [selected, setSelected] = useState<string | null>(null)
  const [tools, setTools] = useState<DiscoveredTool[]>([])
  const [report, setReport] = useState<TestReport | null>(null)
  const [loading, setLoading] = useState(true)
  const [creating, setCreating] = useState(false)
  const [form, setForm] = useState({ title: '', slug: '', url: '' })
  const [deleteTarget, setDeleteTarget] = useState<McpConnection | null>(null)
  const { toasts, error: showError, success: showSuccess, closeToast } = useToast()

  const load = useCallback(async () => {
    if (!repo) return
    setLoading(true)
    try {
      setConnections(await mcpConnectionsApi.list(repo))
    } catch (e: any) {
      showError('Failed to load MCP connections', e?.message)
    } finally {
      setLoading(false)
    }
  }, [repo])

  useEffect(() => {
    load()
  }, [load])

  const loadTools = useCallback(
    async (slug: string) => {
      if (!repo) return
      try {
        const res = await mcpConnectionsApi.listTools(repo, slug)
        setTools(res.tools || [])
      } catch (e: any) {
        showError('Failed to load tools', e?.message)
      }
    },
    [repo],
  )

  const select = (slug: string) => {
    setSelected(slug)
    setReport(null)
    setTools([])
    loadTools(slug)
  }

  const current = connections.find((c) => c.slug === selected)

  const create = async () => {
    if (!repo) return
    try {
      // Created DISABLED: a connection is not useful until it has a
      // credential, and an enabled one would start being discovered at once.
      await mcpConnectionsApi.create(repo, { ...form, enabled: false })
      showSuccess('Connection created — add a credential, then enable it')
      setCreating(false)
      setForm({ title: '', slug: '', url: '' })
      await load()
      select(form.slug)
    } catch (e: any) {
      showError('Could not create the connection', e?.message)
    }
  }

  const test = async () => {
    if (!repo || !selected) return
    try {
      setReport(await mcpConnectionsApi.test(repo, selected))
    } catch (e: any) {
      showError('Probe failed', e?.message)
    }
  }

  const refresh = async () => {
    if (!repo || !selected) return
    try {
      await mcpConnectionsApi.refreshTools(repo, selected)
      showSuccess('Discovery queued')
      setTimeout(() => loadTools(selected), 1500)
    } catch (e: any) {
      showError('Could not refresh tools', e?.message)
    }
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-xl font-semibold flex items-center gap-2">
            <PlugZap className="w-5 h-5 text-sky-300" />
            MCP Connections
          </h1>
          <p className="text-sm text-white/50 mt-1">
            External MCP servers whose tools your agents can call.
          </p>
        </div>
        <button
          type="button"
          onClick={() => setCreating(true)}
          className="px-3 py-1.5 rounded-md bg-sky-500/20 border border-sky-400/40 text-sm"
        >
          <Plus className="w-4 h-4 inline mr-1.5" />
          Add connection
        </button>
      </div>

      {/* Attaching a remote tool to an agent means data leaves this server. */}
      <p className="text-xs text-amber-200/70 flex items-start gap-2">
        <ShieldAlert className="w-4 h-4 mt-0.5 shrink-0" />
        <span>
          A remote tool receives whatever arguments the model chooses to send, and the model has
          seen the conversation. Expose only the tools an agent actually needs.
        </span>
      </p>

      {creating && (
        <GlassCard className="p-4 space-y-3">
          <input
            value={form.title}
            onChange={(e) => setForm({ ...form, title: e.target.value })}
            placeholder="Title (e.g. Linear)"
            className="w-full px-3 py-2 rounded-md bg-black/30 border border-white/10 text-sm"
          />
          <input
            value={form.slug}
            onChange={(e) => setForm({ ...form, slug: e.target.value })}
            placeholder="Slug (lowercase, cannot change later)"
            className="w-full px-3 py-2 rounded-md bg-black/30 border border-white/10 text-sm font-mono"
          />
          <p className="text-xs text-white/40">
            The slug is part of every generated tool path, so it is fixed once tools are discovered.
          </p>
          <input
            value={form.url}
            onChange={(e) => setForm({ ...form, url: e.target.value })}
            placeholder="https://mcp.example.com/mcp"
            className="w-full px-3 py-2 rounded-md bg-black/30 border border-white/10 text-sm font-mono"
          />
          <div className="flex gap-2">
            <button
              type="button"
              onClick={create}
              disabled={!form.slug || !form.url}
              className="px-3 py-1.5 rounded-md bg-sky-500/20 border border-sky-400/40 text-sm disabled:opacity-40"
            >
              Create
            </button>
            <button
              type="button"
              onClick={() => setCreating(false)}
              className="px-3 py-1.5 rounded-md border border-white/10 text-sm text-white/70"
            >
              Cancel
            </button>
          </div>
        </GlassCard>
      )}

      {loading ? (
        <p className="text-sm text-white/50">Loading…</p>
      ) : connections.length === 0 ? (
        <GlassCard className="p-6 text-sm text-white/50">
          No MCP connections yet. Add one to let your agents call an external server's tools.
        </GlassCard>
      ) : (
        <div className="grid grid-cols-1 lg:grid-cols-3 gap-4">
          <div className="space-y-2">
            {connections.map((c) => {
              const status = statusOf(c)
              return (
                <button
                  key={c.slug}
                  type="button"
                  onClick={() => select(c.slug)}
                  className={`w-full text-left p-3 rounded-lg border transition ${
                    selected === c.slug
                      ? 'border-sky-400/40 bg-sky-500/10'
                      : 'border-white/10 hover:border-white/20'
                  }`}
                >
                  <div className="flex items-center justify-between gap-2">
                    <span className="font-medium">{c.title || c.slug}</span>
                    <span className={`text-xs px-2 py-0.5 rounded border ${status.className}`}>
                      {status.label}
                    </span>
                  </div>
                  <div className="text-xs text-white/40 mt-1 truncate font-mono">{c.url}</div>
                  <div className="text-xs text-white/40 mt-0.5">{c.tool_count} tools</div>
                </button>
              )
            })}
          </div>

          <div className="lg:col-span-2 space-y-4">
            {!current ? (
              <GlassCard className="p-6 text-sm text-white/50">
                Select a connection to configure it.
              </GlassCard>
            ) : (
              <>
                <GlassCard className="p-4 space-y-4">
                  <div className="flex items-center justify-between">
                    <h2 className="font-medium">Authentication</h2>
                    <label className="text-xs text-white/60 flex items-center gap-2">
                      <input
                        type="checkbox"
                        checked={current.enabled}
                        onChange={async (e) => {
                          if (!repo) return
                          try {
                            await mcpConnectionsApi.update(repo, current.slug, {
                              enabled: e.target.checked,
                            })
                            await load()
                          } catch (err: any) {
                            showError('Could not update the connection', err?.message)
                          }
                        }}
                      />
                      Enabled
                    </label>
                  </div>
                  <ConnectionAuthPanel
                    repo={repo!}
                    connection={current}
                    onChanged={load}
                    onError={showError}
                    onSuccess={showSuccess}
                  />
                </GlassCard>

                <GlassCard className="p-4 space-y-3">
                  <div className="flex items-center justify-between">
                    <h2 className="font-medium">Connection test</h2>
                    <button
                      type="button"
                      onClick={test}
                      className="px-3 py-1.5 rounded-md border border-white/10 text-sm"
                    >
                      <Activity className="w-4 h-4 inline mr-1.5" />
                      Run probe
                    </button>
                  </div>
                  {report && (
                    <div className="text-sm space-y-1">
                      {report.reachable ? (
                        <>
                          <p className="text-emerald-300">
                            Reachable — {report.server_info?.name ?? 'server'} speaks{' '}
                            {report.protocol_version}
                          </p>
                          <p className="text-white/60">
                            {report.tool_count} tools ({report.permitted_tool_count} would be
                            exposed by the current filter)
                          </p>
                          {report.tools.length > 0 && (
                            <p className="text-xs text-white/40 font-mono">
                              {report.tools.join(', ')}
                            </p>
                          )}
                        </>
                      ) : (
                        <p className="text-rose-300">
                          {report.error_code}: {report.error}
                        </p>
                      )}
                    </div>
                  )}
                </GlassCard>

                <GlassCard className="p-4 space-y-3">
                  <div className="flex items-center justify-between">
                    <h2 className="font-medium">Tools</h2>
                    <button
                      type="button"
                      onClick={refresh}
                      className="px-3 py-1.5 rounded-md border border-white/10 text-sm"
                    >
                      <RefreshCw className="w-4 h-4 inline mr-1.5" />
                      Refresh tools
                    </button>
                  </div>
                  <DiscoveredToolsTable
                    repo={repo!}
                    slug={current.slug}
                    tools={tools}
                    onRefreshed={() => loadTools(current.slug)}
                    onError={showError}
                    onSuccess={showSuccess}
                  />
                </GlassCard>

                <div>
                  <button
                    type="button"
                    onClick={() => setDeleteTarget(current)}
                    className="text-xs text-rose-300/70 hover:text-rose-300"
                  >
                    Delete this connection
                  </button>
                </div>
              </>
            )}
          </div>
        </div>
      )}

      <ConfirmDialog
        open={!!deleteTarget}
        title={`Delete ${deleteTarget?.title ?? ''}?`}
        message="Agents referencing this connection's tools will stop working. Discovered proxies are removed."
        confirmText="Delete"
        variant="danger"
        onConfirm={async () => {
          if (!repo || !deleteTarget) return
          try {
            // Force: the server refuses with 409 while proxies exist, and the
            // dialog has already told the operator what that means.
            await mcpConnectionsApi.remove(repo, deleteTarget.slug, true)
            showSuccess('Connection deleted')
            setSelected(null)
            await load()
          } catch (e: any) {
            showError('Could not delete the connection', e?.message)
          } finally {
            setDeleteTarget(null)
          }
        }}
        onCancel={() => setDeleteTarget(null)}
      />
      <ToastContainer toasts={toasts} onClose={closeToast} />
    </div>
  )
}
