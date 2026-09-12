// SPDX-License-Identifier: BSL-1.1

import { useEffect, useRef, useState } from 'react'
import {
  LinkIcon,
  Unlink,
  Loader2,
  UserCircle,
  Plus,
  Pencil,
  AlertTriangle,
  RefreshCw,
} from 'lucide-react'
import {
  integrationsApi,
  type Integration,
  type ConnectedAccount,
  type Connection,
} from '../../api/integrations'
import ConnectionEditor from './ConnectionEditor'
import ConfirmDialog from '../ConfirmDialog'

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
  return refreshLabel(account, connection)
}

/** "3 minutes ago" / "2 days ago" for an epoch-seconds instant. */
function ago(seconds: number): string {
  const mins = Math.max(0, Math.round((Date.now() - seconds * 1000) / 60000))
  if (mins < 1) return 'just now'
  if (mins < 60) return `${mins} min ago`
  const hours = Math.round(mins / 60)
  if (hours < 48) return `${hours} h ago`
  return `${Math.round(hours / 24)} days ago`
}

/**
 * What an OAuth connection's secondary line says.
 *
 * It used to show `expires_at`, and that was the wrong number in the most
 * misleading possible way. `expires_at` is the ACCESS token's deadline — an
 * hour or so out, renewed silently in the background — so it renders as a time
 * later today whatever state the connection is in. An operator watching it sees
 * a healthy clock tick right up to the day the grant is dead, and clicking
 * Reconnect appears to "only change the time", because that is all it can
 * change.
 *
 * What decides whether a connection survives is the REFRESH token being
 * exercised. That is what this reports.
 */
function refreshLabel(account: ConnectedAccount, connection?: Connection): string {
  // The connection projection is the better source (it carries the server's own
  // `health` verdict), but it is only fetched for connectors that declare a
  // `connection_config_type`. Fall back to the node's own account entry so a
  // connector without one still reports something true rather than nothing.
  if (isFailing(account, connection)) return 'not refreshing — see below'
  const at = connection?.last_refresh_at ?? account.last_refresh_at
  return at ? `token refreshed ${ago(at)}` : 'connected'
}

/** Whether the last background refresh of this connection failed. */
function isFailing(account: ConnectedAccount, connection?: Connection): boolean {
  if (connection) return connection.health === 'failing'
  return !!account.last_refresh_error
}

/** The recorded reason, from whichever source carries it. */
function failureReason(account: ConnectedAccount, connection?: Connection): string {
  return connection?.last_refresh_error || account.last_refresh_error || 'no reason recorded'
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
  // A disconnect the server refused because mounts are pinned to the
  // connection, parked until the operator confirms the cost.
  const [forceTarget, setForceTarget] = useState<
    { account: ConnectedAccount; reason: string } | null
  >(null)

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
      // The one start-failure an operator can do nothing about locally: a
      // managed connector whose OAuth client the control plane has not minted
      // yet (the server names it — see oauth_start.rs). The raw message
      // ("client_id is missing") read as their misconfiguration; it is not.
      if (e?.code === 'MANAGED_CONNECTOR_UNPROVISIONED') {
        onError(
          'Provisioning pending',
          'This connector’s OAuth client has not been provisioned by the Maravilla ' +
            'control plane yet. It usually arrives within a minute — leave this dialog ' +
            'open and try again, or ask your Maravilla admin to run “Repair”.',
        )
      } else {
        onError('Connect failed', e?.message)
      }
    }
  }

  /**
   * Disconnect, refusing to silently orphan mounts.
   *
   * The server answers 409 `CONNECTION_IN_USE` and names the mounts. That
   * confirmation matters more than it looks: removing a connection does not
   * break the mounts now, it breaks them on their next sync, and RE-CONNECTING
   * DOES NOT FIX THEM — consent mints a new account id, so the pinned mounts
   * keep failing against an id that will never come back while this panel shows
   * a healthy connection. Forcing is offered, but only after being told the
   * cost and with the repair (Mounts → Reassign connection) named.
   */
  async function handleDisconnect(account: ConnectedAccount, force = false) {
    if (!integration.path) return
    setDisconnecting(account.id)
    try {
      const res = await integrationsApi.oauthDisconnect(repo, integration.path, account.id, force)
      const orphaned = res.orphaned_mounts?.length ?? 0
      if (orphaned > 0) {
        onSuccess(
          'Disconnected',
          `${orphaned} mount(s) now have no connection. Reassign them on the Mounts page — ` +
            `reconnecting this account will NOT repair them.`,
        )
      } else {
        onSuccess('Disconnected', account.label || account.id)
      }
      onChanged()
    } catch (e: any) {
      if (e?.code === 'CONNECTION_IN_USE' || /CONNECTION_IN_USE/.test(e?.message || '')) {
        // In-DOM dialog, never `window.confirm`: it is forbidden in any
        // Tauri-hosted path (WKWebView returns true without showing anything,
        // which would turn "are you sure?" into an unconditional yes), and the
        // console already ships one everywhere else.
        setForceTarget({ account, reason: e?.message || 'Mounts are pinned to this connection.' })
      } else {
        onError('Disconnect failed', e?.message)
      }
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
                  {/*
                    The whole point of recording granted scopes. Without this the
                    shortfall surfaces as a 403 on the first write — hours or
                    days after the connector's scope list was widened, and
                    nowhere near the account that needs re-consenting.
                  */}
                  {/*
                    A failing refresh is the one thing here that is genuinely
                    urgent, and it used to be visible nowhere at all: the sweep
                    logs a warning and the row keeps rendering a healthy-looking
                    expiry. The grant then dies of inactivity at the provider's
                    own deadline and the operator concludes that reconnecting
                    every couple of weeks is just how this works.
                  */}
                  {isFailing(account, connectionFor(account.id)) && (
                    <div className="mt-1 text-xs text-red-400">
                      <span className="font-medium">Token refresh is failing.</span>{' '}
                      <span className="text-red-300/90">
                        {failureReason(account, connectionFor(account.id))}
                      </span>{' '}
                      Reconnect to re-authorize — left alone, this connection stops working
                      when the provider retires its refresh token.
                    </div>
                  )}
                  {(connectionFor(account.id)?.missing_scopes?.length ?? 0) > 0 && (
                    <div className="mt-1 text-xs text-amber-400">
                      Missing permissions — reconnect to grant:{' '}
                      <span className="font-mono text-[11px] text-amber-300/90">
                        {connectionFor(account.id)!.missing_scopes!.join(', ')}
                      </span>
                    </div>
                  )}
                </div>
              </div>
              <div className="flex items-center gap-1">
              {/*
                Reconnect runs the SAME authorization flow as Connect. It is a
                separate control because the two answer different questions —
                "add an account" versus "this account needs re-consenting" — and
                because a missing scope is otherwise invisible until a write
                comes back 403 hours later.

                Safe to offer per account now that the callback matches on the
                provider's subject and updates in place: re-authorizing keeps
                the account id, so every mount pointing at it keeps working.
                Before that this button would have silently created a SECOND
                account and changed nothing.
              */}
              {connectionFor(account.id)?.auth_kind === 'oauth' && supportsOauth && (
                <button
                  type="button"
                  onClick={handleConnect}
                  disabled={connecting}
                  title="Re-run sign-in for this account, e.g. to grant newly requested permissions"
                  className="flex items-center gap-1 px-2 py-1 text-xs text-zinc-400 hover:text-white hover:bg-white/10 rounded transition-colors disabled:opacity-50"
                >
                  {connecting ? (
                    <Loader2 className="w-3.5 h-3.5 animate-spin" />
                  ) : (
                    <RefreshCw className="w-3.5 h-3.5" />
                  )}
                  Reconnect
                </button>
              )}
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

      <ConfirmDialog
        open={forceTarget !== null}
        title="This connection is in use"
        message={
          `${forceTarget?.reason || ''}\n\n` +
          `Disconnect anyway? Those mounts stop syncing immediately and stay broken until ` +
          `you reassign each one to another connection on the Mounts page.\n\n` +
          `Reconnecting this account will NOT repair them: consent creates a new connection ` +
          `with a new id, and the mounts stay pinned to the one being removed here.`
        }
        variant="danger"
        confirmText="Disconnect anyway"
        onConfirm={() => {
          const target = forceTarget
          setForceTarget(null)
          if (target) void handleDisconnect(target.account, true)
        }}
        onCancel={() => setForceTarget(null)}
      />
    </div>
  )
}
