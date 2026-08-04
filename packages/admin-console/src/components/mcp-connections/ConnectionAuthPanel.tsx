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
  const [clientId, setClientId] = useState('')
  const [clientSecret, setClientSecret] = useState('')

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
                : 'border-white/10 text-zinc-300 hover:text-white'
            }`}
          >
            {k === 'none' ? 'No auth' : k === 'static' ? 'Token / API key' : 'OAuth 2.1'}
          </button>
        ))}
      </div>

      {/* Every caller of a proxy tool acts as this one identity. Operators need
          to know that before they attach a tool to an agent. */}
      <p className="text-xs text-zinc-400 flex items-start gap-2">
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
              <span className="text-zinc-400">No credential stored</span>
            )}
          </div>
          <input
            type="password"
            value={value}
            onChange={(e) => setValue(e.target.value)}
            placeholder={connection.credential_set ? 'Replace the stored token…' : 'Bearer token or API key'}
            className="w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-gray-500 text-sm focus:border-primary-400 focus:ring-2 focus:ring-primary-400/20 transition-all"
            autoComplete="off"
          />
          <input
            value={headerName}
            onChange={(e) => setHeaderName(e.target.value)}
            placeholder="Header name (leave blank for Authorization: Bearer)"
            className="w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-gray-500 text-sm focus:border-primary-400 focus:ring-2 focus:ring-primary-400/20 transition-all"
          />
          <div className="flex gap-2">
            <button
              type="button"
              disabled={busy || !value}
              onClick={saveCredential}
              className="px-3 py-1.5 rounded-md bg-primary-500/20 border border-primary-400/40 text-white text-sm disabled:opacity-40"
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
                className="px-3 py-1.5 rounded-md border border-white/10 text-sm text-zinc-300"
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
              className="px-3 py-1.5 rounded-md border border-white/10 text-white text-sm hover:bg-white/5 disabled:opacity-40"
            >
              {busy ? <Loader2 className="w-4 h-4 inline mr-1.5 animate-spin" /> : null}
              Discover
            </button>
            <button
              type="button"
              disabled={busy || !connection.oauth_client}
              onClick={connect}
              className="px-3 py-1.5 rounded-md bg-primary-500/20 border border-primary-400/40 text-white text-sm disabled:opacity-40"
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
                className="px-3 py-1.5 rounded-md border border-white/10 text-sm text-zinc-300"
              >
                Disconnect
              </button>
            )}
          </div>

          {discovery && (
            <div className="text-xs text-zinc-300 space-y-1 rounded-md border border-white/10 p-3">
              {discovery.requires_auth === false ? (
                <p>{discovery.message}</p>
              ) : discovery.discovered ? (
                <>
                  <p>
                    Issuer <span className="text-zinc-200">{discovery.issuer}</span>
                  </p>
                  <p>
                    {discovery.supports_dynamic_registration
                      ? 'Registered automatically (dynamic client registration).'
                      : 'This server does not support dynamic registration — register the redirect URI below by hand.'}
                  </p>
                  {discovery.redirect_uri && (
                    <p className="font-mono break-all text-zinc-300">{discovery.redirect_uri}</p>
                  )}
                  {discovery.needs_manual_client_secret && (
                    <p className="text-amber-300 pt-1">{discovery.message}</p>
                  )}
                </>
              ) : (
                <p>{discovery.message}</p>
              )}
            </div>
          )}

          {/*
            Opened automatically when discovery says the server demands client
            authentication and we have no secret — that combination fails at the
            token exchange AFTER consent, so it has to be fixed before Connect.
          */}
          <details open={discovery?.needs_manual_client_secret} className="text-xs">
            <summary className="cursor-pointer text-zinc-400 hover:text-white">
              I registered RaisinDB myself (client id / secret)
            </summary>
            <div className="space-y-2 pt-3">
              <input
                value={clientId}
                onChange={(e) => setClientId(e.target.value)}
                placeholder="Client ID"
                className="w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-gray-500 text-sm focus:border-primary-400 focus:ring-2 focus:ring-primary-400/20 transition-all font-mono"
              />
              <input
                type="password"
                value={clientSecret}
                onChange={(e) => setClientSecret(e.target.value)}
                placeholder="Client secret (only if the provider issued one)"
                className="w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-gray-500 text-sm focus:border-primary-400 focus:ring-2 focus:ring-primary-400/20 transition-all"
                autoComplete="off"
              />
              <p className="text-zinc-500">
                Endpoints already found by Discover are kept — leave them alone and supply only
                what is missing. The secret is encrypted immediately and never shown again.
              </p>
              <button
                type="button"
                disabled={busy || !clientId.trim()}
                onClick={() =>
                  run('Could not save the OAuth client', async () => {
                    await mcpConnectionsApi.setOauthClient(repo, connection.slug, {
                      client_id: clientId.trim(),
                      client_secret: clientSecret || undefined,
                    })
                    setClientId('')
                    setClientSecret('')
                    onSuccess('OAuth client saved')
                  })
                }
                className="px-3 py-1.5 rounded-md bg-primary-500/20 border border-primary-400/40 text-white text-sm disabled:opacity-40"
              >
                Save OAuth client
              </button>
            </div>
          </details>
        </div>
      )}
    </div>
  )
}
