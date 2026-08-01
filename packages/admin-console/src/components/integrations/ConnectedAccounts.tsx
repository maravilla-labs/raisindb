// SPDX-License-Identifier: BSL-1.1

import { useEffect, useRef, useState } from 'react'
import { LinkIcon, Unlink, Loader2, UserCircle, Plus, Pencil, AlertTriangle } from 'lucide-react'
import {
  integrationsApi,
  type Integration,
  type ConnectedAccount,
  type Connection,
} from '../../api/integrations'
import ConnectionEditor from './ConnectionEditor'

interface ConnectedAccountsProps {
  repo: string
  integration: Integration
  /** Called after a connect/disconnect so the parent can reload the node. */
  onChanged: () => void
  onError: (title: string, message?: string) => void
  onSuccess: (title: string, message?: string) => void
  /**
   * Why connecting is currently blocked, or undefined when it is allowed.
   *
   * OAuth starts from the SAVED connector node — the server reads client id,
   * endpoints and client secret from storage, never from this form. Editing a
   * field and pressing Connect without saving therefore authorizes against the
   * previous configuration, and the failure surfaces much later as an opaque
   * provider rejection.
   */
  blockedReason?: string
}

/**
 * Secondary line for a connection.
 *
 * A credential-based connection has no token expiry, so showing one would be
 * nonsense; display its identity field instead (the schema marks one
 * `meta.identity`, conventionally a username or host).
 */
function subtitleFor(account: ConnectedAccount, connection?: Connection): string {
  if (connection?.auth_kind === 'config') {
    const cfg = connection.config || {}
    const identity = ['username', 'host', 'tenant_id']
      .map((k) => cfg[k])
      .find((v) => typeof v === 'string' && v.length > 0)
    return (identity as string) || 'credential connection'
  }
  return expiryLabel(account)
}

function expiryLabel(account: ConnectedAccount): string {
  if (!account.expires_at) return ''
  const when = new Date(account.expires_at * 1000)
  const expired = account.expires_at * 1000 < Date.now()
  return `${expired ? 'expired' : 'expires'} ${when.toLocaleString()}`
}

/**
 * Connected-accounts list plus the "Connect account" OAuth popup flow.
 *
 * The popup navigates to the provider, then the server callback redirects it
 * back to a same-origin URL. We poll the popup: once it is same-origin (or has
 * closed) the flow is done and we ask the parent to reload.
 */
export default function ConnectedAccounts({
  repo,
  integration,
  onChanged,
  onError,
  onSuccess,
  blockedReason,
}: ConnectedAccountsProps) {
  const [connecting, setConnecting] = useState(false)
  const [disconnecting, setDisconnecting] = useState<string | null>(null)
  const pollRef = useRef<number | null>(null)
  // Editing state for credential-based connections.
  const [editorOpen, setEditorOpen] = useState(false)
  const [editing, setEditing] = useState<Connection | undefined>()
  // Read connections from the dedicated endpoint rather than off the node: the
  // node carries token/secret ciphertext, this projection does not.
  const [connections, setConnections] = useState<Connection[]>([])

  const accounts = integration.connected_accounts || []
  // "Add connection" only makes sense once the connector declares what a
  // connection looks like.
  const supportsConnections = !!integration.connection_config_type
  // OAuth is only offered when the connector actually has an authorize endpoint.
  const supportsOauth = !!integration.oauth_config?.auth_url

  useEffect(() => {
    if (!integration.path || !supportsConnections) return
    let cancelled = false
    integrationsApi
      .listConnections(repo, integration.path)
      .then((c) => !cancelled && setConnections(c))
      .catch(() => {
        /* Non-fatal: the account list below still renders from the node. */
      })
    return () => {
      cancelled = true
    }
  }, [repo, integration.path, supportsConnections, accounts.length])

  // The dialog can be closed mid-connect; without this the interval keeps
  // firing against an unmounted component.
  useEffect(() => stopPolling, [])

  function connectionFor(id: string): Connection | undefined {
    return connections.find((c) => c.id === id)
  }

  function stopPolling() {
    if (pollRef.current !== null) {
      window.clearInterval(pollRef.current)
      pollRef.current = null
    }
  }

  async function handleConnect() {
    if (!integration.path) {
      onError('Save first', 'Save the integration before connecting an account.')
      return
    }
    setConnecting(true)
    try {
      const { auth_url } = await integrationsApi.oauthStart(repo, integration.path)
      const popup = window.open(auth_url, 'raisin-oauth', 'width=520,height=680')
      if (!popup) {
        onError('Popup blocked', 'Allow popups for this site, then try again.')
        setConnecting(false)
        return
      }

      // The outcome arrives as a postMessage from the console page the callback
      // redirects the popup to (see Integrations.tsx). It is authoritative:
      // reaching a same-origin URL means only that the flow ENDED, and this used
      // to be read as success — so a refused grant reported "Account connected"
      // and the operator was left with a connector that silently had no account.
      let settled = false
      const finish = (report: () => void) => {
        if (settled) return
        settled = true
        stopPolling()
        window.removeEventListener('message', onMessage)
        if (!popup.closed) popup.close()
        setConnecting(false)
        report()
        onChanged()
      }

      function onMessage(ev: MessageEvent) {
        if (ev.origin !== window.location.origin) return
        if (ev.data?.type !== 'raisin-oauth-result') return
        const { connected, error, description } = ev.data
        finish(() =>
          error
            ? onError(`Connect failed: ${error}`, description || undefined)
            : onSuccess('Account connected', connected || undefined),
        )
      }
      window.addEventListener('message', onMessage)

      // Fallback for a popup that never reports: the operator closed it, or it
      // ended somewhere that cannot relay (an old server rendering a raw error
      // body). Never claim success here — we genuinely do not know.
      pollRef.current = window.setInterval(() => {
        if (!popup.closed) return
        finish(() =>
          onError(
            'Connect not completed',
            'The connect window closed before reporting a result. If you did authorize, ' +
              'check the connections list below — otherwise check the server log for the ' +
              'provider’s error.',
          ),
        )
      }, 600)
    } catch (e: any) {
      setConnecting(false)
      onError('Connect failed', e?.message)
    }
  }

  async function handleDisconnect(account: ConnectedAccount) {
    if (!integration.path) return
    setDisconnecting(account.id)
    try {
      await integrationsApi.oauthDisconnect(repo, integration.path, account.id)
      onSuccess('Disconnected', account.label || account.id)
      onChanged()
    } catch (e: any) {
      onError('Disconnect failed', e?.message)
    } finally {
      setDisconnecting(null)
    }
  }

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between">
        <h3 className="text-sm font-semibold text-white">Connections</h3>
        <div className="flex items-center gap-2">
          {supportsConnections && (
            <button
              type="button"
              onClick={() => {
                setEditing(undefined)
                setEditorOpen(true)
              }}
              disabled={!integration.path}
              title="Add a connection using a username and password"
              className="flex items-center gap-2 px-3 py-1.5 bg-white/5 hover:bg-white/10 border border-white/10 text-white text-sm rounded-lg transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
            >
              <Plus className="w-4 h-4" />
              Add connection
            </button>
          )}
          {supportsOauth && (
            <button
              type="button"
              onClick={handleConnect}
              disabled={connecting || !integration.path || !!blockedReason}
              title={blockedReason}
              className="flex items-center gap-2 px-3 py-1.5 bg-primary-500 hover:bg-primary-600 text-white text-sm rounded-lg transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
            >
              {connecting ? <Loader2 className="w-4 h-4 animate-spin" /> : <LinkIcon className="w-4 h-4" />}
              Connect account
            </button>
          )}
        </div>
      </div>

      {supportsOauth && blockedReason && (
        <p className="flex items-start gap-1.5 text-xs text-amber-400">
          <AlertTriangle className="mt-0.5 h-3.5 w-3.5 flex-shrink-0" />
          {blockedReason}
        </p>
      )}

      {accounts.length === 0 ? (
        <p className="text-zinc-500 text-sm">
          No connections yet.
          {supportsConnections && ' Use "Add connection" to enter a username and password.'}
        </p>
      ) : (
        <ul className="space-y-2">
          {accounts.map((account) => (
            <li
              key={account.id}
              className="flex items-center justify-between gap-3 px-3 py-2 bg-white/5 border border-white/10 rounded-lg"
            >
              <div className="flex items-center gap-2 min-w-0">
                <UserCircle className="w-5 h-5 text-primary-300 flex-shrink-0" />
                <div className="min-w-0">
                  <div className="text-white text-sm truncate">
                    {account.label || account.subject || account.id}
                  </div>
                  <div className="text-zinc-500 text-xs truncate">
                    {subtitleFor(account, connectionFor(account.id))}
                  </div>
                </div>
              </div>
              <div className="flex items-center gap-1">
              {connectionFor(account.id)?.auth_kind === 'config' && (
                <button
                  type="button"
                  onClick={() => {
                    setEditing(connectionFor(account.id))
                    setEditorOpen(true)
                  }}
                  title="Edit this connection"
                  className="flex items-center gap-1 px-2 py-1 text-xs text-zinc-400 hover:text-white hover:bg-white/10 rounded transition-colors"
                >
                  <Pencil className="w-3.5 h-3.5" />
                  Edit
                </button>
              )}
              <button
                type="button"
                onClick={() => handleDisconnect(account)}
                disabled={disconnecting === account.id}
                className="flex items-center gap-1 px-2 py-1 text-xs text-zinc-400 hover:text-red-400 hover:bg-red-500/10 rounded transition-colors disabled:opacity-50"
              >
                {disconnecting === account.id ? (
                  <Loader2 className="w-3.5 h-3.5 animate-spin" />
                ) : (
                  <Unlink className="w-3.5 h-3.5" />
                )}
                Disconnect
              </button>
              </div>
            </li>
          ))}
        </ul>
      )}

      {editorOpen && (
        <ConnectionEditor
          repo={repo}
          integration={integration}
          connection={editing}
          onClose={() => setEditorOpen(false)}
          onSaved={onChanged}
          onError={onError}
          onSuccess={onSuccess}
        />
      )}
    </div>
  )
}
