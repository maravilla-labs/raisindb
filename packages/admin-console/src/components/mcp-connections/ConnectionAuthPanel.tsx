// SPDX-License-Identifier: BSL-1.1

import { useState } from 'react'
import { KeyRound, ShieldCheck, ShieldAlert, Link2, Loader2 } from 'lucide-react'
import {
  mcpConnectionsApi,
  runOAuthPopup,
  type AuthKind,
  type DiscoverResult,
  type McpConnection,
} from '../../api/mcp-connections'

interface Props {
  repo: string
  connection: McpConnection
  onChanged: () => void
  onError: (title: string, detail?: string) => void
  onSuccess: (message: string) => void
}

/**
 * Authentication for one connection: none, a static credential, or the
 * OAuth 2.1 service-account flow.
 *
 * The credential field is write-only — nothing here ever displays a stored
 * secret, because no endpoint returns one. The UI shows only whether one is set.
 */
export default function ConnectionAuthPanel({
  repo,
  connection,
  onChanged,
  onError,
  onSuccess,
}: Props) {
  const [kind, setKind] = useState<AuthKind>(connection.auth_kind)
  const [value, setValue] = useState('')
  const [headerName, setHeaderName] = useState('')
  const [busy, setBusy] = useState(false)
  const [discovery, setDiscovery] = useState<DiscoverResult | null>(null)

  const run = async (label: string, fn: () => Promise<unknown>) => {
    setBusy(true)
    try {
      await fn()
      onChanged()
    } catch (e: any) {
      onError(label, e?.message)
    } finally {
      setBusy(false)
    }
  }

  const saveCredential = () =>
    run('Could not save the credential', async () => {
      const staticAuth = headerName.trim()
        ? { scheme: 'header', header_name: headerName.trim() }
        : { scheme: 'bearer' }
      await mcpConnectionsApi.setCredential(repo, connection.slug, value, staticAuth)
      setValue('')
      onSuccess('Credential saved')
    })

  const discover = () =>
    run('Discovery failed', async () => {
      const result = await mcpConnectionsApi.oauthDiscover(repo, connection.slug)
      setDiscovery(result)
      if (!result.requires_auth) onSuccess('This server does not require authorization')
    })

  const connect = () =>
    run('Could not start authorization', async () => {
      const { auth_url } = await mcpConnectionsApi.oauthStart(repo, connection.slug)
      const outcome = await runOAuthPopup(auth_url)
      if (outcome.error) onError('Authorization failed', outcome.error)
      else onSuccess(`Connected ${outcome.connected ?? connection.title}`)
    })

  return (
    <div className="space-y-4">
      <div className="flex gap-2">
        {(['none', 'static', 'oauth'] as AuthKind[]).map((k) => (
          <button
            key={k}
            type="button"
            onClick={() => setKind(k)}
            className={`px-3 py-1.5 rounded-md text-sm border ${
              kind === k
                ? 'bg-sky-500/15 border-sky-400/40 text-sky-200'
                : 'border-white/10 text-white/60 hover:text-white/90'
            }`}
          >
            {k === 'none' ? 'No auth' : k === 'static' ? 'Token / API key' : 'OAuth 2.1'}
          </button>
        ))}
      </div>

      {/* Every caller of a proxy tool acts as this one identity. Operators need
          to know that before they attach a tool to an agent. */}
      <p className="text-xs text-white/50 flex items-start gap-2">
        <ShieldAlert className="w-4 h-4 mt-0.5 shrink-0 text-amber-300/70" />
        <span>
          A connection has one identity. Every agent and every user calling one of its tools acts
          as this credential — it is not per-user.
        </span>
      </p>

      {kind === 'static' && (
        <div className="space-y-3">
          <div className="flex items-center gap-2 text-sm">
            {connection.credential_set ? (
              <span className="text-emerald-300 flex items-center gap-1.5">
                <ShieldCheck className="w-4 h-4" /> A credential is stored
              </span>
            ) : (
              <span className="text-white/50">No credential stored</span>
            )}
          </div>
          <input
            type="password"
            value={value}
            onChange={(e) => setValue(e.target.value)}
            placeholder={connection.credential_set ? 'Replace the stored token…' : 'Bearer token or API key'}
            className="w-full px-3 py-2 rounded-md bg-black/30 border border-white/10 text-sm"
            autoComplete="off"
          />
          <input
            value={headerName}
            onChange={(e) => setHeaderName(e.target.value)}
            placeholder="Header name (leave blank for Authorization: Bearer)"
            className="w-full px-3 py-2 rounded-md bg-black/30 border border-white/10 text-sm"
          />
          <div className="flex gap-2">
            <button
              type="button"
              disabled={busy || !value}
              onClick={saveCredential}
              className="px-3 py-1.5 rounded-md bg-sky-500/20 border border-sky-400/40 text-sm disabled:opacity-40"
            >
              <KeyRound className="w-4 h-4 inline mr-1.5" />
              Save credential
            </button>
            {connection.credential_set && (
              <button
                type="button"
                disabled={busy}
                onClick={() =>
                  run('Could not clear the credential', async () => {
                    await mcpConnectionsApi.clearCredential(repo, connection.slug)
                    onSuccess('Credential cleared')
                  })
                }
                className="px-3 py-1.5 rounded-md border border-white/10 text-sm text-white/70"
              >
                Clear
              </button>
            )}
          </div>
        </div>
      )}

      {kind === 'oauth' && (
        <div className="space-y-3">
          <div className="flex gap-2">
            <button
              type="button"
              disabled={busy}
              onClick={discover}
              className="px-3 py-1.5 rounded-md border border-white/10 text-sm disabled:opacity-40"
            >
              {busy ? <Loader2 className="w-4 h-4 inline mr-1.5 animate-spin" /> : null}
              Discover
            </button>
            <button
              type="button"
              disabled={busy || !connection.oauth_client}
              onClick={connect}
              className="px-3 py-1.5 rounded-md bg-sky-500/20 border border-sky-400/40 text-sm disabled:opacity-40"
            >
              <Link2 className="w-4 h-4 inline mr-1.5" />
              {connection.oauth_connected ? 'Reconnect' : 'Connect'}
            </button>
            {connection.oauth_connected && (
              <button
                type="button"
                disabled={busy}
                onClick={() =>
                  run('Could not disconnect', async () => {
                    await mcpConnectionsApi.oauthDisconnect(repo, connection.slug)
                    onSuccess('Disconnected')
                  })
                }
                className="px-3 py-1.5 rounded-md border border-white/10 text-sm text-white/70"
              >
                Disconnect
              </button>
            )}
          </div>

          {discovery && (
            <div className="text-xs text-white/60 space-y-1 rounded-md border border-white/10 p-3">
              {discovery.requires_auth === false ? (
                <p>{discovery.message}</p>
              ) : discovery.discovered ? (
                <>
                  <p>
                    Issuer <span className="text-white/80">{discovery.issuer}</span>
                  </p>
                  <p>
                    {discovery.supports_dynamic_registration
                      ? 'Registered automatically (dynamic client registration).'
                      : 'This server does not support dynamic registration — register the redirect URI below by hand.'}
                  </p>
                  {discovery.redirect_uri && (
                    <p className="font-mono break-all text-white/70">{discovery.redirect_uri}</p>
                  )}
                </>
              ) : (
                <p>{discovery.message}</p>
              )}
            </div>
          )}
        </div>
      )}
    </div>
  )
}
