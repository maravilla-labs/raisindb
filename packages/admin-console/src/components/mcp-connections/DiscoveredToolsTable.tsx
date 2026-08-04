// SPDX-License-Identifier: BSL-1.1

import { useState } from 'react'
import { RefreshCw, Copy, Check, AlertTriangle, Trash2 } from 'lucide-react'
import { mcpConnectionsApi, type DiscoveredTool, type ToolState } from '../../api/mcp-connections'

interface Props {
  repo: string
  slug: string
  tools: DiscoveredTool[]
  onRefreshed: () => void
  onError: (title: string, detail?: string) => void
  onSuccess: (message: string) => void
}

const STATE_STYLE: Record<ToolState, string> = {
  active: 'text-emerald-300 bg-emerald-500/10 border-emerald-400/30',
  missing: 'text-amber-300 bg-amber-500/10 border-amber-400/30',
  conflict: 'text-rose-300 bg-rose-500/10 border-rose-400/30',
}

const STATE_HELP: Record<ToolState, string> = {
  active: 'Present on the remote server.',
  missing: 'Gone from the remote server. The proxy is disabled, not deleted, so agents referencing it fail loudly rather than silently losing a tool.',
  conflict: 'The generated function name collides with an existing function; this tool was skipped.',
}

/**
 * The tools a connection discovered, and which of them are exposed.
 *
 * Toggling records intent on the connection's tool filter and enqueues a
 * discovery run — the proxy nodes themselves are only ever written by that job,
 * so there is exactly one writer.
 */
export default function DiscoveredToolsTable({
  repo,
  slug,
  tools,
  onRefreshed,
  onError,
  onSuccess,
}: Props) {
  const [busy, setBusy] = useState<string | null>(null)
  const [copied, setCopied] = useState<string | null>(null)

  const toggle = async (tool: DiscoveredTool) => {
    setBusy(tool.remote_name)
    try {
      await mcpConnectionsApi.setToolEnabled(repo, slug, tool.remote_name, !tool.enabled)
      onSuccess(`${tool.remote_name} ${tool.enabled ? 'disabled' : 'enabled'} — refreshing`)
      onRefreshed()
    } catch (e: any) {
      onError('Could not change the tool', e?.message)
    } finally {
      setBusy(null)
    }
  }

  /**
   * Delete the proxies for tools that are gone upstream.
   *
   * Never automatic: discovery disables rather than deletes, because a deleted
   * proxy vanishes from any agent holding its path with no error anywhere. The
   * server refuses with 409 naming those agents, and that message is shown
   * as-is rather than retried with force.
   */
  const prune = async () => {
    setBusy('__prune__')
    try {
      const res = await mcpConnectionsApi.pruneTools(repo, slug)
      onSuccess(`Pruned ${res.pruned} tool(s)`)
      onRefreshed()
    } catch (e: any) {
      onError('Could not prune', e?.message)
    } finally {
      setBusy(null)
    }
  }

  const copy = async (path: string) => {
    await navigator.clipboard.writeText(path)
    setCopied(path)
    setTimeout(() => setCopied(null), 1500)
  }

  if (!tools.length) {
    return (
      <p className="text-sm text-white/50">
        No tools discovered yet. Use <span className="text-white/80">Refresh tools</span> once the
        connection is authorized.
      </p>
    )
  }

  const missing = tools.filter((t) => t.state === 'missing')

  return (
    <div className="overflow-x-auto">
      {missing.length > 0 && (
        <div className="mb-3 flex items-center justify-between gap-3 text-xs">
          <span className="text-amber-200/70">
            {missing.length} tool(s) are gone from the remote server. Their proxies are disabled,
            not deleted, so agents referencing them fail loudly rather than losing a tool silently.
          </span>
          <button
            type="button"
            disabled={busy === '__prune__'}
            onClick={prune}
            className="shrink-0 px-2.5 py-1 rounded-md border border-rose-400/30 text-rose-300 disabled:opacity-40"
          >
            <Trash2 className="w-3 h-3 inline mr-1" />
            Prune missing ({missing.length})
          </button>
        </div>
      )}
      <table className="w-full text-sm">
        <thead className="text-white/50 text-xs uppercase">
          <tr>
            <th className="text-left py-2 pr-4">Remote tool</th>
            <th className="text-left py-2 pr-4">Agent path</th>
            <th className="text-left py-2 pr-4">State</th>
            <th className="text-right py-2">Exposed</th>
          </tr>
        </thead>
        <tbody>
          {tools.map((tool) => (
            <tr key={tool.remote_name} className="border-t border-white/5">
              <td className="py-2 pr-4 font-mono text-white/90">{tool.remote_name}</td>
              <td className="py-2 pr-4">
                {/* This path is what an operator pastes into an agent's tools list. */}
                <button
                  type="button"
                  onClick={() => copy(tool.function_path)}
                  className="font-mono text-xs text-white/60 hover:text-white/90 inline-flex items-center gap-1.5"
                  title="Copy the path to paste into an agent's tools list"
                >
                  {tool.function_path}
                  {copied === tool.function_path ? (
                    <Check className="w-3 h-3 text-emerald-300" />
                  ) : (
                    <Copy className="w-3 h-3" />
                  )}
                </button>
              </td>
              <td className="py-2 pr-4">
                <span
                  title={STATE_HELP[tool.state]}
                  className={`px-2 py-0.5 rounded border text-xs ${STATE_STYLE[tool.state]}`}
                >
                  {tool.state === 'conflict' && <AlertTriangle className="w-3 h-3 inline mr-1" />}
                  {tool.state}
                </span>
              </td>
              <td className="py-2 text-right">
                <button
                  type="button"
                  disabled={busy === tool.remote_name || tool.state === 'conflict'}
                  onClick={() => toggle(tool)}
                  className={`px-2.5 py-1 rounded-md text-xs border disabled:opacity-40 ${
                    tool.enabled
                      ? 'bg-sky-500/15 border-sky-400/40 text-sky-200'
                      : 'border-white/10 text-white/60'
                  }`}
                >
                  {busy === tool.remote_name ? (
                    <RefreshCw className="w-3 h-3 inline animate-spin" />
                  ) : tool.enabled ? (
                    'Exposed'
                  ) : (
                    'Hidden'
                  )}
                </button>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}
