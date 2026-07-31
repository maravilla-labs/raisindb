// SPDX-License-Identifier: BSL-1.1

import { useEffect, useState } from 'react'
import { X, KeyRound, CheckCircle2, ExternalLink } from 'lucide-react'
import { integrationsApi, type Integration, type OAuthConfig } from '../../api/integrations'
import ConnectedAccounts from './ConnectedAccounts'
import SchemaForm from '../PropertyFields/SchemaForm'
import { nodeTypesApi, type ResolvedNodeType } from '../../api/nodetypes'
import CapabilityChips from './CapabilityChips'
import TestConnectionPanel from './TestConnectionPanel'
import CopyableUrlField from './CopyableUrlField'
import MarkdownRenderer from '../MarkdownRenderer'

interface IntegrationEditorProps {
  repo: string
  /** Existing integration to edit; omit to create a new one. */
  integration?: Integration
  onClose: () => void
  /** Called after a successful save so the parent can reload the list. */
  onSaved: () => void
  onError: (title: string, message?: string) => void
  onSuccess: (title: string, message?: string) => void
}

const field =
  'w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-white/40 focus:outline-none focus:ring-2 focus:ring-primary-500'
const labelCls = 'block text-white text-sm font-medium mb-1.5'

export default function IntegrationEditor({
  repo,
  integration,
  onClose,
  onSaved,
  onError,
  onSuccess,
}: IntegrationEditorProps) {
  const isEdit = !!integration
  const [name, setName] = useState(integration?.name || '')
  const [title, setTitle] = useState(integration?.title || '')
  const [providerType, setProviderType] = useState(integration?.provider_type || 'custom')
  const [adapterFn, setAdapterFn] = useState(integration?.adapter_function || '')
  const [enabled, setEnabled] = useState(integration?.enabled ?? true)
  const [oauth, setOauth] = useState<OAuthConfig>(integration?.oauth_config || {})
  const [scopesText, setScopesText] = useState((integration?.oauth_config?.scopes || []).join('\n'))
  const [clientSecret, setClientSecret] = useState('')
  const [secretSet, setSecretSet] = useState(integration?.client_secret_set || false)
  const [setupInstructions, setSetupInstructions] = useState(integration?.setup_instructions || '')
  const [docsUrl, setDocsUrl] = useState(integration?.docs_url || '')
  const [saving, setSaving] = useState(false)
  // Local copy so the connected-accounts panel can refresh after connect.
  const [current, setCurrent] = useState<Integration | undefined>(integration)
  // Canonical OAuth redirect URI to register with the provider. Server-resolved
  // from RAISINDB_BASE_URL; falls back to the current origin when unset so the
  // user still sees a copyable, usually-correct value.
  const clientRedirectUri = `${window.location.origin}/api/integrations/${repo}/oauth/callback`
  const [redirectUri, setRedirectUri] = useState(clientRedirectUri)
  const [baseConfigured, setBaseConfigured] = useState(true)
  // Which connected account the connection test runs as. The server invokes the
  // adapter with `credential: null` when no account_id is sent, which every
  // OAuth adapter dereferences — so always send one when we have one.
  const [testAccountId, setTestAccountId] = useState('')

  const testAccounts = current?.connected_accounts || []
  // An OAuth connector with no connected account cannot be tested at all.
  const needsAccount = testAccounts.length === 0 && !!oauth.auth_url

  // Microsoft identity platform URLs are `.../{authority}/oauth2/v2.0/{authorize,token}`,
  // where authority is `common`, `organizations`, `consumers` or a tenant GUID.
  const MS_LOGIN_HOST = 'login.microsoftonline.com'
  const isMicrosoftAuthority = [oauth.auth_url, oauth.token_url].some((u) =>
    (u || '').includes(MS_LOGIN_HOST),
  )
  const [msTenant, setMsTenant] = useState('')

  // Schema-driven connector-level configuration. When the connector declares a
  // `config_type`, its fields are rendered from that NodeType instead of being
  // hardcoded here — which is what lets a new connector ship its own settings
  // without a console change.
  const [configSchema, setConfigSchema] = useState<ResolvedNodeType | null>(null)
  const [configValues, setConfigValues] = useState<Record<string, any>>(
    (integration?.config as Record<string, any>) || {},
  )
  const [configSecretDrafts, setConfigSecretDrafts] = useState<Record<string, string | null>>({})
  const configSecretsSet = current?.config_secret_fields || []

  useEffect(() => {
    const typeName = current?.config_type
    if (!typeName) {
      setConfigSchema(null)
      return
    }
    let cancelled = false
    nodeTypesApi
      .getResolved(repo, 'main', typeName, 'raisin:system')
      .then((r) => !cancelled && setConfigSchema(r))
      .catch(() => {
        // The declaring package may not be installed here; the hardcoded OAuth
        // block below still renders, so this is a degradation, not a failure.
        if (!cancelled) setConfigSchema(null)
      })
    return () => {
      cancelled = true
    }
  }, [repo, current?.config_type])

  /** Swap the authority segment in both Microsoft endpoint URLs. */
  function applyMsTenant() {
    const tenant = msTenant.trim()
    if (!tenant) return
    const swap = (u?: string) => {
      if (!u || !u.includes(MS_LOGIN_HOST)) return u
      // Replace only the first path segment after the host, leaving the rest
      // (oauth2/v2.0/authorize|token) untouched.
      const host = MS_LOGIN_HOST.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
      return u.replace(new RegExp(`(${host}/)[^/]+`), (_m, prefix: string) => `${prefix}${tenant}`)
    }
    patchOauth({ auth_url: swap(oauth.auth_url), token_url: swap(oauth.token_url) })
  }

  // The server reads `oauth_config.redirect_uri` verbatim and never validates
  // it, so a value missing the `{repo}` segment or the trailing `callback`
  // sends the provider to a URL that 404s. Compare on path only: the host may
  // legitimately differ from this browser's origin (proxy, custom domain).
  const canonicalPath = `/api/integrations/${repo}/oauth/callback`
  const redirectMismatch = (() => {
    const v = (oauth.redirect_uri || '').trim()
    if (!v) return false
    try {
      return new URL(v).pathname.replace(/\/+$/, '') !== canonicalPath
    } catch {
      return true // not a URL at all
    }
  })()

  useEffect(() => {
    // Default to the first account, and follow along when accounts change
    // (e.g. right after a connect) without clobbering an explicit choice.
    if (!testAccounts.some((a) => a.id === testAccountId)) {
      setTestAccountId(testAccounts[0]?.id || '')
    }
  }, [testAccounts, testAccountId])

  useEffect(() => {
    let cancelled = false
    integrationsApi
      .getSetupUrls(repo)
      .then((s) => {
        if (cancelled) return
        setBaseConfigured(s.base_url_configured)
        // Prefer the server value when a real base is configured; otherwise keep
        // the origin-based fallback (the server value carries a {base} placeholder).
        const resolved =
          s.base_url_configured && s.oauth_redirect_uri ? s.oauth_redirect_uri : clientRedirectUri
        setRedirectUri(resolved)
        // The shipped connector templates ship `redirect_uri: ""`, which makes
        // oauth/start 400. Prefill it so the common case is correct by default;
        // never overwrite a value the operator already set.
        setOauth((prev) => (prev.redirect_uri ? prev : { ...prev, redirect_uri: resolved }))
      })
      .catch(() => {
        /* Non-fatal: the origin-based fallback is already shown. */
      })
    return () => {
      cancelled = true
    }
  }, [repo])

  function patchOauth(patch: Partial<OAuthConfig>) {
    setOauth((prev) => ({ ...prev, ...patch }))
  }

  async function reloadAccounts() {
    if (!current) return
    try {
      const fresh = await integrationsApi.getIntegration(repo, current.name)
      setCurrent(fresh)
      setSecretSet(fresh.client_secret_set || false)
    } catch (e: any) {
      onError('Reload failed', e?.message)
    }
  }

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault()
    if (!name.trim() || !title.trim() || !adapterFn.trim()) {
      onError('Missing fields', 'Name, title and adapter function are required.')
      return
    }
    setSaving(true)
    try {
      const scopes = scopesText
        .split(/[\n,]/)
        .map((s) => s.trim())
        .filter(Boolean)
      const model: Integration = {
        // Carry the server's own property bag so the save merges onto it rather
        // than replacing it — without this, Update deletes connected_accounts,
        // client_secret_encrypted, capabilities and api_config. `current` is
        // refreshed after a connect, so this is the post-connect bag.
        _raw: current?._raw,
        name: name.trim(),
        title: title.trim(),
        provider_type: providerType.trim(),
        adapter_function: adapterFn.trim(),
        enabled,
        oauth_config: { ...oauth, scopes },
        setup_instructions: setupInstructions.trim() || undefined,
        docs_url: docsUrl.trim() || undefined,
        ...(configSchema ? { config: configValues } : {}),
      }

      const saved = isEdit
        ? await integrationsApi.updateIntegration(repo, integration!.name, model)
        : await integrationsApi.createIntegration(repo, model)

      // Schema-declared secrets go through the dedicated encrypting endpoint —
      // never onto the node in cleartext. Only touched fields are sent.
      const changedSecrets = Object.fromEntries(
        Object.entries(configSecretDrafts).filter(([, v]) => v === null || (v && v.length > 0)),
      )
      if (Object.keys(changedSecrets).length > 0 && (saved.path || current?.path)) {
        try {
          await integrationsApi.setConfigSecrets(
            repo,
            saved.path || current!.path!,
            changedSecrets,
          )
          setConfigSecretDrafts({})
        } catch (e: any) {
          onError('Secrets not saved', e?.message || 'The connector secrets could not be stored.')
        }
      }

      // Write-only secret goes through the dedicated encrypting endpoint.
      if (clientSecret.trim()) {
        try {
          await integrationsApi.setClientSecret(repo, saved.path || '', clientSecret.trim())
          setSecretSet(true)
          setClientSecret('')
        } catch (e: any) {
          onError('Secret not saved', e?.message || 'The client secret could not be stored.')
        }
      }

      setCurrent(saved)
      onSuccess(isEdit ? 'Integration updated' : 'Integration created', saved.title)
      onSaved()
    } catch (e: any) {
      onError('Save failed', e?.message)
    } finally {
      setSaving(false)
    }
  }

  return (
    <div className="fixed inset-0 bg-black/50 backdrop-blur-sm flex items-center justify-center z-50 p-4 overscroll-none">
      <div className="bg-zinc-900 border border-white/10 rounded-xl shadow-2xl max-w-2xl w-full max-h-[90vh] overflow-y-auto overscroll-contain">
        <div className="flex items-center justify-between p-6 border-b border-white/10">
          <h2 className="text-2xl font-bold text-white">
            {isEdit ? 'Edit Connector' : 'New Connector'}
          </h2>
          <button onClick={onClose} className="p-2 hover:bg-white/10 rounded-lg transition-colors">
            <X className="w-5 h-5 text-white/60" />
          </button>
        </div>

        <form onSubmit={handleSubmit} className="p-6 space-y-5">
          <div className="grid grid-cols-2 gap-4">
            <div>
              <label className={labelCls}>Name *</label>
              <input
                className={field}
                value={name}
                disabled={isEdit}
                onChange={(e) => setName(e.target.value)}
                placeholder="google-drive"
              />
            </div>
            <div>
              <label className={labelCls}>Title *</label>
              <input
                className={field}
                value={title}
                onChange={(e) => setTitle(e.target.value)}
                placeholder="Google Drive"
              />
            </div>
            <div>
              <label className={labelCls}>Provider type</label>
              <input
                className={field}
                value={providerType}
                onChange={(e) => setProviderType(e.target.value)}
                placeholder="google-drive"
              />
            </div>
            <div>
              <label className={labelCls}>Adapter function *</label>
              <input
                className={field}
                value={adapterFn}
                onChange={(e) => setAdapterFn(e.target.value)}
                placeholder="/adapters/google-drive"
              />
            </div>
          </div>

          <label className="flex items-center gap-2 text-white text-sm">
            <input
              type="checkbox"
              checked={enabled}
              onChange={(e) => setEnabled(e.target.checked)}
              className="w-4 h-4 rounded"
            />
            Enabled
          </label>

          <fieldset className="border border-white/10 rounded-lg p-4 space-y-4">
            <legend className="px-2 text-sm font-semibold text-zinc-300">Provider setup</legend>

            <CopyableUrlField
              label="OAuth Redirect URI"
              value={redirectUri}
              warn={!baseConfigured}
              helper={
                baseConfigured
                  ? 'Register this exact URI in your provider’s OAuth client (redirect / reply URL).'
                  : 'RAISINDB_BASE_URL is not set on the server — this URI is guessed from your browser’s address. Set RAISINDB_BASE_URL, then re-open this dialog for the canonical value.'
              }
            />

            <div>
              <label className={labelCls}>Provider docs URL</label>
              <input
                className={field}
                value={docsUrl}
                onChange={(e) => setDocsUrl(e.target.value)}
                placeholder="https://…"
              />
              {docsUrl.trim() && (
                <a
                  href={docsUrl.trim()}
                  target="_blank"
                  rel="noopener noreferrer"
                  className="mt-1 inline-flex items-center gap-1 text-xs text-primary-400 hover:text-primary-300"
                >
                  <ExternalLink className="w-3 h-3" /> Open provider docs
                </a>
              )}
            </div>

            <div>
              <label className={labelCls}>Setup instructions (markdown)</label>
              <textarea
                className={`${field} resize-y font-mono text-xs`}
                rows={5}
                value={setupInstructions}
                onChange={(e) => setSetupInstructions(e.target.value)}
                placeholder="Steps the operator must complete on the provider side…"
              />
              {setupInstructions.trim() && (
                <div className="mt-2 rounded-lg border border-white/10 bg-black/20 p-3">
                  <p className="mb-2 text-[11px] uppercase tracking-wide text-zinc-500">Preview</p>
                  <MarkdownRenderer content={setupInstructions} />
                </div>
              )}
            </div>
          </fieldset>

          <fieldset className="border border-white/10 rounded-lg p-4 space-y-4">
            <legend className="px-2 text-sm font-semibold text-zinc-300">OAuth configuration</legend>
            <div className="grid grid-cols-2 gap-4">
              <div>
                <label className={labelCls}>Client ID</label>
                <input
                  className={field}
                  value={oauth.client_id || ''}
                  onChange={(e) => patchOauth({ client_id: e.target.value })}
                />
              </div>
              <div>
                <label className={labelCls}>Redirect URI</label>
                <input
                  className={field}
                  value={oauth.redirect_uri || ''}
                  onChange={(e) => patchOauth({ redirect_uri: e.target.value })}
                  placeholder={redirectUri}
                />
                {redirectMismatch && (
                  <p className="mt-1 text-xs text-amber-400">
                    This does not match the callback this server actually serves. The provider
                    will redirect to a URL that 404s. Expected{' '}
                    <code className="font-mono">{redirectUri}</code>.{' '}
                    <button
                      type="button"
                      className="underline hover:text-amber-300"
                      onClick={() => patchOauth({ redirect_uri: redirectUri })}
                    >
                      Use it
                    </button>
                  </p>
                )}
              </div>
              <div>
                <label className={labelCls}>Auth URL</label>
                <input
                  className={field}
                  value={oauth.auth_url || ''}
                  onChange={(e) => patchOauth({ auth_url: e.target.value })}
                />
              </div>
              <div>
                <label className={labelCls}>Token URL</label>
                <input
                  className={field}
                  value={oauth.token_url || ''}
                  onChange={(e) => patchOauth({ token_url: e.target.value })}
                />
              </div>
              <div>
                <label className={labelCls}>Revoke URL</label>
                <input
                  className={field}
                  value={oauth.revoke_url || ''}
                  onChange={(e) => patchOauth({ revoke_url: e.target.value })}
                />
              </div>
            </div>

            {isMicrosoftAuthority && (
              <div className="rounded-lg border border-white/10 bg-white/5 p-3 space-y-2">
                <label className={labelCls}>Microsoft directory (tenant) ID</label>
                <div className="flex gap-2">
                  <input
                    className={field}
                    value={msTenant}
                    onChange={(e) => setMsTenant(e.target.value)}
                    placeholder="common, organizations, or a tenant GUID"
                  />
                  <button
                    type="button"
                    onClick={applyMsTenant}
                    disabled={!msTenant.trim()}
                    className="px-3 py-2 bg-white/5 hover:bg-white/10 border border-white/10 text-white text-sm rounded-lg whitespace-nowrap disabled:opacity-50 disabled:cursor-not-allowed"
                  >
                    Apply
                  </button>
                </div>
                <p className="text-xs text-zinc-500">
                  Rewrites the authority segment of both the Auth URL and the Token URL.
                  Microsoft rejects <code className="font-mono">common</code> for
                  single-tenant apps registered after 2018-10-15 with{' '}
                  <code className="font-mono">AADSTS50194</code> — use your directory GUID
                  unless the app registration is explicitly multi-tenant.
                </p>
              </div>
            )}

            <div>
              <label className={labelCls}>Scopes (one per line)</label>
              <textarea
                className={`${field} resize-none font-mono text-xs`}
                rows={3}
                value={scopesText}
                onChange={(e) => setScopesText(e.target.value)}
              />
            </div>

            <div>
              <label className={labelCls}>
                <span className="inline-flex items-center gap-1">
                  <KeyRound className="w-3.5 h-3.5" /> Client secret (write-only)
                </span>
              </label>
              <input
                type="password"
                className={field}
                value={clientSecret}
                autoComplete="new-password"
                onChange={(e) => setClientSecret(e.target.value)}
                placeholder={secretSet ? '•••••••• (secret set — leave blank to keep)' : 'Enter to set'}
              />
              {secretSet && (
                <p className="mt-1 text-xs text-green-400 flex items-center gap-1">
                  <CheckCircle2 className="w-3.5 h-3.5" /> A client secret is stored (never displayed).
                </p>
              )}
            </div>
          </fieldset>

          {configSchema && (configSchema.resolved_properties?.length ?? 0) > 0 && (
            <fieldset className="border border-white/10 rounded-lg p-4 space-y-4">
              <legend className="px-2 text-sm font-semibold text-zinc-300">
                Connector configuration
              </legend>
              <p className="text-xs text-zinc-500">
                Settings shared by every connection of this connector, declared by{' '}
                <code className="font-mono">{current?.config_type}</code>.
              </p>
              <SchemaForm
                properties={configSchema.resolved_properties as any[]}
                values={configValues}
                secretsSet={configSecretsSet}
                secretDrafts={configSecretDrafts}
                onChange={(name, value) =>
                  setConfigValues((prev) => ({ ...prev, [name]: value }))
                }
                onSecretChange={(name, value) =>
                  setConfigSecretDrafts((prev) => ({ ...prev, [name]: value }))
                }
                disabled={saving}
              />
            </fieldset>
          )}

          {current?.path && (
            <div className="border border-white/10 rounded-lg p-4 space-y-3">
              <div className="flex items-center justify-between gap-3">
                <h3 className="text-sm font-semibold text-white">Capabilities</h3>
                {current.capabilities_checked_at && (
                  <span className="text-[10px] text-zinc-500">
                    checked {new Date(current.capabilities_checked_at).toLocaleString()}
                  </span>
                )}
              </div>
              <CapabilityChips capabilities={current.capabilities} />
              {testAccounts.length > 1 && (
                <div>
                  <label className={labelCls}>Test as account</label>
                  <select
                    value={testAccountId}
                    onChange={(e) => setTestAccountId(e.target.value)}
                    className={field}
                  >
                    {testAccounts.map((a) => (
                      <option key={a.id} value={a.id}>
                        {a.label || a.subject || a.id}
                      </option>
                    ))}
                  </select>
                </div>
              )}
              <TestConnectionPanel
                repo={repo}
                request={{
                  integration_path: current.path,
                  // Without an account the adapter is invoked with a null
                  // credential and every OAuth connector fails on
                  // `credential.access_token`. Default to the first account.
                  account_id: testAccountId || undefined,
                }}
                disabledReason={
                  needsAccount
                    ? 'Connect an account below first — this connector uses OAuth, so the test needs a credential.'
                    : undefined
                }
                onTested={reloadAccounts}
                onError={onError}
              />
            </div>
          )}

          {current?.path && (
            <div className="border border-white/10 rounded-lg p-4">
              <ConnectedAccounts
                repo={repo}
                integration={current}
                onChanged={reloadAccounts}
                onError={onError}
                onSuccess={onSuccess}
              />
            </div>
          )}

          <div className="flex flex-col-reverse md:flex-row gap-3 pt-2">
            <button
              type="button"
              onClick={onClose}
              className="flex-1 px-6 py-3 bg-white/5 hover:bg-white/10 border border-white/10 text-white rounded-lg transition-colors"
            >
              Close
            </button>
            <button
              type="submit"
              disabled={saving}
              className="flex-1 px-6 py-3 bg-primary-500 hover:bg-primary-600 text-white rounded-lg transition-colors disabled:opacity-50"
            >
              {saving ? 'Saving…' : isEdit ? 'Update' : 'Create'}
            </button>
          </div>
        </form>
      </div>
    </div>
  )
}
