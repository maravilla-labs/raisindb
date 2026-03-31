import { useState, useEffect } from 'react'
import {
  Shield,
  Key,
  Eye,
  EyeOff,
  CheckCircle,
  XCircle,
  Loader2,
  Info,
  ChevronDown,
  ChevronRight,
  Users,
  Mail,
  Lock,
  Clock,
  Plus,
  Trash2,
  RefreshCw,
  Globe,
  Fingerprint,
  UserX,
} from 'lucide-react'
import GlassCard from '../components/GlassCard'
import { ToastContainer, useToast } from '../components/Toast'
import {
  identityAuthApi,
  AuthProvider,
  TenantAuthSettings as AuthSettings,
} from '../api/identity-auth'

// Known OIDC providers with icons and defaults
const OIDC_PROVIDERS = [
  {
    id: 'google',
    name: 'Google',
    icon: 'google',
    issuer: 'https://accounts.google.com',
    scopes: ['openid', 'email', 'profile'],
  },
  {
    id: 'microsoft',
    name: 'Microsoft / Azure AD',
    icon: 'microsoft',
    issuer: 'https://login.microsoftonline.com/{tenant}/v2.0',
    scopes: ['openid', 'email', 'profile'],
  },
  {
    id: 'okta',
    name: 'Okta',
    icon: 'okta',
    issuer: 'https://{domain}.okta.com',
    scopes: ['openid', 'email', 'profile', 'groups'],
  },
  {
    id: 'keycloak',
    name: 'Keycloak',
    icon: 'keycloak',
    issuer: 'https://{host}/realms/{realm}',
    scopes: ['openid', 'email', 'profile'],
  },
  {
    id: 'auth0',
    name: 'Auth0',
    icon: 'auth0',
    issuer: 'https://{domain}.auth0.com',
    scopes: ['openid', 'email', 'profile'],
  },
  {
    id: 'custom',
    name: 'Custom OIDC',
    icon: 'shield',
    issuer: '',
    scopes: ['openid', 'email', 'profile'],
  },
]

interface ProviderFormData {
  strategy_type: string
  display_name: string
  client_id: string
  client_secret: string
  issuer_url: string
  scopes: string[]
}

export default function TenantAuthSettings() {
  const { toasts, closeToast, success, error: showError } = useToast()

  // State
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)
  const [providers, setProviders] = useState<AuthProvider[]>([])
  const [settings, setSettings] = useState<AuthSettings | null>(null)

  // UI State
  const [expandedSections, setExpandedSections] = useState<Record<string, boolean>>({
    providers: true,
    anonymous: false,
    cors: false,
    local: false,
    session: false,
    password: false,
    access: false,
  })
  const [newCorsOrigin, setNewCorsOrigin] = useState('')
  const [showAddProvider, setShowAddProvider] = useState(false)
  const [newProvider, setNewProvider] = useState<ProviderFormData>({
    strategy_type: 'oidc:google',
    display_name: 'Sign in with Google',
    client_id: '',
    client_secret: '',
    issuer_url: 'https://accounts.google.com',
    scopes: ['openid', 'email', 'profile'],
  })
  const [showClientSecret, setShowClientSecret] = useState(false)
  const [testingProvider, setTestingProvider] = useState<string | null>(null)
  const [testResults, setTestResults] = useState<Record<string, { success: boolean; error?: string }>>({})

  // Load data
  useEffect(() => {
    loadData()
  }, [])

  // TODO: Get tenant from context or URL params
  const tenantId = 'default'

  const loadData = async () => {
    setLoading(true)
    try {
      const [providersRes, settingsRes] = await Promise.all([
        identityAuthApi.getProviders().catch(() => ({ providers: [], local_enabled: true, magic_link_enabled: true })),
        identityAuthApi.getSettings(tenantId).catch(() => null),
      ])

      setProviders(providersRes.providers)

      if (settingsRes) {
        setSettings(settingsRes)
      } else {
        // Initialize with defaults
        setSettings({
          tenant_id: '',
          local_auth: { enabled: true, allow_registration: false },
          magic_link: { enabled: true, token_ttl_minutes: 15 },
          password_policy: {
            min_length: 8,
            require_uppercase: true,
            require_lowercase: true,
            require_numbers: true,
            require_special: false,
            max_age_days: undefined,
          },
          session_settings: {
            duration_hours: 24,
            refresh_token_duration_days: 30,
            max_sessions_per_user: 10,
            single_session_mode: false,
          },
          access_settings: {
            allow_access_requests: true,
            allow_invitations: true,
            require_approval: true,
            default_roles: ['viewer'],
          },
          anonymous_enabled: false,
          cors_allowed_origins: [],
        })
      }
    } catch (error) {
      showError('Failed to load authentication settings')
    } finally {
      setLoading(false)
    }
  }

  const toggleSection = (section: string) => {
    setExpandedSections(prev => ({ ...prev, [section]: !prev[section] }))
  }

  const handleProviderTypeChange = (type: string) => {
    const provider = OIDC_PROVIDERS.find(p => `oidc:${p.id}` === type) || OIDC_PROVIDERS[0]
    setNewProvider({
      strategy_type: type,
      display_name: `Sign in with ${provider.name}`,
      client_id: '',
      client_secret: '',
      issuer_url: provider.issuer,
      scopes: [...provider.scopes],
    })
  }

  const handleAddProvider = async () => {
    if (!newProvider.client_id || !newProvider.client_secret || !newProvider.issuer_url) {
      showError('Please fill in all required fields')
      return
    }

    setSaving(true)
    try {
      await identityAuthApi.addProvider(newProvider.strategy_type, {
        display_name: newProvider.display_name,
        client_id: newProvider.client_id,
        client_secret: newProvider.client_secret,
        issuer_url: newProvider.issuer_url,
        scopes: newProvider.scopes,
      })

      success('Authentication provider added successfully')
      setShowAddProvider(false)
      setNewProvider({
        strategy_type: 'oidc:google',
        display_name: 'Sign in with Google',
        client_id: '',
        client_secret: '',
        issuer_url: 'https://accounts.google.com',
        scopes: ['openid', 'email', 'profile'],
      })
      await loadData()
    } catch (error) {
      showError(`Failed to add provider: ${error instanceof Error ? error.message : 'Unknown error'}`)
    } finally {
      setSaving(false)
    }
  }

  const handleRemoveProvider = async (providerId: string) => {
    if (!confirm('Are you sure you want to remove this authentication provider? Users will no longer be able to sign in with it.')) {
      return
    }

    try {
      await identityAuthApi.removeProvider(providerId)
      success('Provider removed')
      await loadData()
    } catch (error) {
      showError(`Failed to remove provider: ${error instanceof Error ? error.message : 'Unknown error'}`)
    }
  }

  const handleToggleProvider = async (providerId: string, enabled: boolean) => {
    try {
      await identityAuthApi.updateProvider(providerId, { enabled })
      success(enabled ? 'Provider enabled' : 'Provider disabled')
      await loadData()
    } catch (error) {
      showError(`Failed to update provider: ${error instanceof Error ? error.message : 'Unknown error'}`)
    }
  }

  const handleTestProvider = async (providerId: string) => {
    setTestingProvider(providerId)
    try {
      const result = await identityAuthApi.testProvider(providerId)
      setTestResults(prev => ({ ...prev, [providerId]: result }))
      if (result.success) {
        success('Provider configuration is valid')
      } else {
        showError(result.error || 'Provider test failed')
      }
    } catch (error) {
      setTestResults(prev => ({
        ...prev,
        [providerId]: { success: false, error: error instanceof Error ? error.message : 'Unknown error' },
      }))
      showError('Provider test failed')
    } finally {
      setTestingProvider(null)
    }
  }

  const handleSaveSettings = async () => {
    if (!settings) return

    setSaving(true)
    try {
      await identityAuthApi.updateSettings(tenantId, settings)
      success('Settings saved successfully')
    } catch (error) {
      showError(`Failed to save settings: ${error instanceof Error ? error.message : 'Unknown error'}`)
    } finally {
      setSaving(false)
    }
  }

  if (loading) {
    return (
      <div className="flex items-center justify-center min-h-[400px]">
        <Loader2 className="w-8 h-8 animate-spin text-purple-400" />
      </div>
    )
  }

  return (
    <div className="space-y-6">
      <ToastContainer toasts={toasts} onClose={closeToast} />

      {/* Header */}
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-3">
          <div className="p-3 bg-purple-500/20 rounded-xl border border-purple-500/30">
            <Shield className="w-6 h-6 text-purple-400" />
          </div>
          <div>
            <h1 className="text-2xl font-bold text-white">Authentication Settings</h1>
            <p className="text-gray-400 text-sm mt-0.5">
              Configure identity providers, session policies, and access controls
            </p>
          </div>
        </div>
        <button
          onClick={handleSaveSettings}
          disabled={saving}
          className="flex items-center gap-2 px-4 py-2 bg-purple-500 hover:bg-purple-600 disabled:bg-purple-500/50 text-white rounded-lg transition-colors"
        >
          {saving ? <Loader2 className="w-4 h-4 animate-spin" /> : <CheckCircle className="w-4 h-4" />}
          Save Changes
        </button>
      </div>

      {/* Authentication Providers */}
      <GlassCard>
        <button
          onClick={() => toggleSection('providers')}
          className="w-full flex items-center justify-between"
        >
          <div className="flex items-center gap-3">
            <div className="p-2 rounded-lg bg-blue-500/20 border border-blue-500/30">
              <Fingerprint className="w-5 h-5 text-blue-400" />
            </div>
            <div className="text-left">
              <h2 className="text-lg font-semibold text-white">Authentication Providers</h2>
              <p className="text-sm text-gray-400">Configure OIDC, local, and magic link authentication</p>
            </div>
          </div>
          {expandedSections.providers ? (
            <ChevronDown className="w-5 h-5 text-gray-400" />
          ) : (
            <ChevronRight className="w-5 h-5 text-gray-400" />
          )}
        </button>

        {expandedSections.providers && (
          <div className="mt-6 space-y-4">
            {/* Built-in Methods */}
            <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
              {/* Local Auth */}
              <div className={`p-4 rounded-lg border ${settings?.local_auth.enabled ? 'bg-green-500/10 border-green-500/30' : 'bg-white/5 border-white/10'}`}>
                <div className="flex items-center justify-between">
                  <div className="flex items-center gap-3">
                    <Lock className="w-5 h-5 text-gray-400" />
                    <div>
                      <p className="font-medium text-white">Password Authentication</p>
                      <p className="text-xs text-gray-400">Email and password login</p>
                    </div>
                  </div>
                  <label className="flex items-center gap-2 cursor-pointer">
                    <div className="relative">
                      <input
                        type="checkbox"
                        checked={settings?.local_auth.enabled ?? true}
                        onChange={(e) => setSettings(prev => prev ? {
                          ...prev,
                          local_auth: { ...prev.local_auth, enabled: e.target.checked }
                        } : null)}
                        className="sr-only peer"
                      />
                      <div className="w-9 h-5 bg-white/10 border border-white/20 rounded-full peer peer-checked:bg-green-500 peer-checked:border-green-400 transition-all"></div>
                      <div className="absolute left-0.5 top-0.5 w-4 h-4 bg-white rounded-full transition-transform peer-checked:translate-x-4"></div>
                    </div>
                  </label>
                </div>
                {/* Allow Registration Option */}
                {settings?.local_auth.enabled && (
                  <div className="mt-3 pt-3 border-t border-white/10">
                    <label className="flex items-center gap-2 cursor-pointer text-sm">
                      <input
                        type="checkbox"
                        checked={settings?.local_auth.allow_registration ?? false}
                        onChange={(e) => setSettings(prev => prev ? {
                          ...prev,
                          local_auth: { ...prev.local_auth, allow_registration: e.target.checked }
                        } : null)}
                        className="w-4 h-4 rounded border-white/20 bg-white/5 text-purple-500 focus:ring-purple-400"
                      />
                      <span className="text-gray-300">Allow self-registration</span>
                    </label>
                  </div>
                )}
              </div>

              {/* Magic Link */}
              <div className={`p-4 rounded-lg border ${settings?.magic_link.enabled ? 'bg-green-500/10 border-green-500/30' : 'bg-white/5 border-white/10'}`}>
                <div className="flex items-center justify-between">
                  <div className="flex items-center gap-3">
                    <Mail className="w-5 h-5 text-gray-400" />
                    <div>
                      <p className="font-medium text-white">Magic Link</p>
                      <p className="text-xs text-gray-400">Passwordless email login</p>
                    </div>
                  </div>
                  <label className="flex items-center gap-2 cursor-pointer">
                    <div className="relative">
                      <input
                        type="checkbox"
                        checked={settings?.magic_link.enabled ?? true}
                        onChange={(e) => setSettings(prev => prev ? {
                          ...prev,
                          magic_link: { ...prev.magic_link, enabled: e.target.checked }
                        } : null)}
                        className="sr-only peer"
                      />
                      <div className="w-9 h-5 bg-white/10 border border-white/20 rounded-full peer peer-checked:bg-green-500 peer-checked:border-green-400 transition-all"></div>
                      <div className="absolute left-0.5 top-0.5 w-4 h-4 bg-white rounded-full transition-transform peer-checked:translate-x-4"></div>
                    </div>
                  </label>
                </div>
                {/* Token TTL Option */}
                {settings?.magic_link.enabled && (
                  <div className="mt-3 pt-3 border-t border-white/10">
                    <label className="flex items-center gap-2 text-sm">
                      <span className="text-gray-300">Link expires in</span>
                      <input
                        type="number"
                        value={settings?.magic_link.token_ttl_minutes ?? 15}
                        onChange={(e) => setSettings(prev => prev ? {
                          ...prev,
                          magic_link: { ...prev.magic_link, token_ttl_minutes: parseInt(e.target.value) || 15 }
                        } : null)}
                        min={5}
                        max={60}
                        className="w-16 px-2 py-1 bg-white/5 border border-white/10 rounded text-white text-center focus:border-purple-400 focus:ring-1 focus:ring-purple-400/20"
                      />
                      <span className="text-gray-300">minutes</span>
                    </label>
                  </div>
                )}
              </div>
            </div>

            {/* OIDC Providers */}
            <div className="border-t border-white/10 pt-4">
              <div className="flex items-center justify-between mb-4">
                <h3 className="text-sm font-medium text-gray-300">OIDC Providers</h3>
                <button
                  onClick={() => setShowAddProvider(true)}
                  className="flex items-center gap-2 px-3 py-1.5 bg-purple-500/20 hover:bg-purple-500/30 border border-purple-500/30 text-purple-300 rounded-lg text-sm transition-colors"
                >
                  <Plus className="w-4 h-4" />
                  Add Provider
                </button>
              </div>

              {providers.length === 0 ? (
                <div className="text-center py-8 text-gray-500">
                  <Globe className="w-8 h-8 mx-auto mb-2 opacity-50" />
                  <p>No OIDC providers configured</p>
                  <p className="text-xs mt-1">Add Google, Okta, or other OIDC providers</p>
                </div>
              ) : (
                <div className="space-y-3">
                  {providers.map(provider => (
                    <div
                      key={provider.id}
                      className={`p-4 rounded-lg border transition-colors ${
                        provider.enabled
                          ? 'bg-green-500/10 border-green-500/30'
                          : 'bg-white/5 border-white/10'
                      }`}
                    >
                      <div className="flex items-center justify-between">
                        <div className="flex items-center gap-3">
                          <Globe className="w-5 h-5 text-gray-400" />
                          <div>
                            <p className="font-medium text-white">{provider.display_name}</p>
                            <p className="text-xs text-gray-400">{provider.strategy_type}</p>
                          </div>
                        </div>
                        <div className="flex items-center gap-2">
                          {testResults[provider.id] && (
                            testResults[provider.id].success ? (
                              <CheckCircle className="w-4 h-4 text-green-400" />
                            ) : (
                              <XCircle className="w-4 h-4 text-red-400" />
                            )
                          )}
                          <button
                            onClick={() => handleTestProvider(provider.id)}
                            disabled={testingProvider === provider.id}
                            className="p-1.5 hover:bg-white/10 rounded transition-colors"
                            title="Test connection"
                          >
                            {testingProvider === provider.id ? (
                              <Loader2 className="w-4 h-4 animate-spin text-gray-400" />
                            ) : (
                              <RefreshCw className="w-4 h-4 text-gray-400" />
                            )}
                          </button>
                          <label className="flex items-center gap-2 cursor-pointer">
                            <div className="relative">
                              <input
                                type="checkbox"
                                checked={provider.enabled}
                                onChange={(e) => handleToggleProvider(provider.id, e.target.checked)}
                                className="sr-only peer"
                              />
                              <div className="w-9 h-5 bg-white/10 border border-white/20 rounded-full peer peer-checked:bg-green-500 peer-checked:border-green-400 transition-all"></div>
                              <div className="absolute left-0.5 top-0.5 w-4 h-4 bg-white rounded-full transition-transform peer-checked:translate-x-4"></div>
                            </div>
                          </label>
                          <button
                            onClick={() => handleRemoveProvider(provider.id)}
                            className="p-1.5 hover:bg-red-500/20 rounded transition-colors text-gray-400 hover:text-red-400"
                            title="Remove provider"
                          >
                            <Trash2 className="w-4 h-4" />
                          </button>
                        </div>
                      </div>
                    </div>
                  ))}
                </div>
              )}
            </div>

            {/* Add Provider Form */}
            {showAddProvider && (
              <div className="border-t border-white/10 pt-4 mt-4">
                <div className="flex items-center justify-between mb-4">
                  <h3 className="text-sm font-medium text-gray-300">Add OIDC Provider</h3>
                  <button
                    onClick={() => setShowAddProvider(false)}
                    className="text-gray-400 hover:text-white transition-colors"
                  >
                    <XCircle className="w-5 h-5" />
                  </button>
                </div>

                <div className="space-y-4">
                  {/* Provider Type */}
                  <div>
                    <label className="block text-sm font-medium text-gray-300 mb-2">Provider Type</label>
                    <select
                      value={newProvider.strategy_type}
                      onChange={(e) => handleProviderTypeChange(e.target.value)}
                      className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20 transition-all"
                    >
                      {OIDC_PROVIDERS.map(p => (
                        <option key={p.id} value={`oidc:${p.id}`}>{p.name}</option>
                      ))}
                    </select>
                  </div>

                  {/* Display Name */}
                  <div>
                    <label className="block text-sm font-medium text-gray-300 mb-2">Display Name</label>
                    <input
                      type="text"
                      value={newProvider.display_name}
                      onChange={(e) => setNewProvider(prev => ({ ...prev, display_name: e.target.value }))}
                      placeholder="Sign in with Google"
                      className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder:text-gray-500 focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20 transition-all"
                    />
                  </div>

                  {/* Issuer URL */}
                  <div>
                    <label className="block text-sm font-medium text-gray-300 mb-2">
                      Issuer URL
                      <span className="text-red-400 ml-1">*</span>
                    </label>
                    <input
                      type="text"
                      value={newProvider.issuer_url}
                      onChange={(e) => setNewProvider(prev => ({ ...prev, issuer_url: e.target.value }))}
                      placeholder="https://accounts.google.com"
                      className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder:text-gray-500 focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20 transition-all"
                    />
                  </div>

                  {/* Client ID */}
                  <div>
                    <label className="block text-sm font-medium text-gray-300 mb-2">
                      Client ID
                      <span className="text-red-400 ml-1">*</span>
                    </label>
                    <input
                      type="text"
                      value={newProvider.client_id}
                      onChange={(e) => setNewProvider(prev => ({ ...prev, client_id: e.target.value }))}
                      placeholder="your-client-id"
                      className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder:text-gray-500 focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20 transition-all"
                    />
                  </div>

                  {/* Client Secret */}
                  <div>
                    <label className="block text-sm font-medium text-gray-300 mb-2">
                      Client Secret
                      <span className="text-red-400 ml-1">*</span>
                    </label>
                    <div className="relative">
                      <input
                        type={showClientSecret ? 'text' : 'password'}
                        value={newProvider.client_secret}
                        onChange={(e) => setNewProvider(prev => ({ ...prev, client_secret: e.target.value }))}
                        placeholder="your-client-secret"
                        className="w-full px-4 py-2 pr-12 bg-white/5 border border-white/10 rounded-lg text-white placeholder:text-gray-500 focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20 transition-all"
                      />
                      <button
                        type="button"
                        onClick={() => setShowClientSecret(!showClientSecret)}
                        className="absolute right-3 top-1/2 -translate-y-1/2 p-1 hover:bg-white/10 rounded transition-colors"
                      >
                        {showClientSecret ? (
                          <EyeOff className="w-5 h-5 text-gray-400" />
                        ) : (
                          <Eye className="w-5 h-5 text-gray-400" />
                        )}
                      </button>
                    </div>
                  </div>

                  {/* Scopes */}
                  <div>
                    <label className="block text-sm font-medium text-gray-300 mb-2">Scopes</label>
                    <input
                      type="text"
                      value={newProvider.scopes.join(' ')}
                      onChange={(e) => setNewProvider(prev => ({ ...prev, scopes: e.target.value.split(' ').filter(Boolean) }))}
                      placeholder="openid email profile"
                      className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder:text-gray-500 focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20 transition-all"
                    />
                    <p className="text-xs text-gray-500 mt-1">Space-separated list of OAuth scopes</p>
                  </div>

                  {/* Actions */}
                  <div className="flex justify-end gap-3 pt-2">
                    <button
                      onClick={() => setShowAddProvider(false)}
                      className="px-4 py-2 bg-white/5 hover:bg-white/10 border border-white/10 text-white rounded-lg transition-colors"
                    >
                      Cancel
                    </button>
                    <button
                      onClick={handleAddProvider}
                      disabled={saving}
                      className="flex items-center gap-2 px-4 py-2 bg-purple-500 hover:bg-purple-600 disabled:bg-purple-500/50 text-white rounded-lg transition-colors"
                    >
                      {saving ? <Loader2 className="w-4 h-4 animate-spin" /> : <Plus className="w-4 h-4" />}
                      Add Provider
                    </button>
                  </div>
                </div>
              </div>
            )}
          </div>
        )}
      </GlassCard>

      {/* Anonymous Access */}
      <GlassCard>
        <button
          onClick={() => toggleSection('anonymous')}
          className="w-full flex items-center justify-between"
        >
          <div className="flex items-center gap-3">
            <div className="p-2 rounded-lg bg-orange-500/20 border border-orange-500/30">
              <UserX className="w-5 h-5 text-orange-400" />
            </div>
            <div className="text-left">
              <h2 className="text-lg font-semibold text-white">Anonymous Access</h2>
              <p className="text-sm text-gray-400">Allow unauthenticated users to access public content</p>
            </div>
          </div>
          {expandedSections.anonymous ? (
            <ChevronDown className="w-5 h-5 text-gray-400" />
          ) : (
            <ChevronRight className="w-5 h-5 text-gray-400" />
          )}
        </button>

        {expandedSections.anonymous && settings && (
          <div className="mt-6 space-y-4">
            <div className={`p-4 rounded-lg border ${settings.anonymous_enabled ? 'bg-orange-500/10 border-orange-500/30' : 'bg-white/5 border-white/10'}`}>
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-3">
                  <UserX className="w-5 h-5 text-gray-400" />
                  <div>
                    <p className="font-medium text-white">Enable Anonymous Access</p>
                    <p className="text-xs text-gray-400">Unauthenticated requests use the "anonymous" role</p>
                  </div>
                </div>
                <label className="flex items-center gap-2 cursor-pointer">
                  <div className="relative">
                    <input
                      type="checkbox"
                      checked={settings.anonymous_enabled ?? false}
                      onChange={(e) => setSettings(prev => prev ? {
                        ...prev,
                        anonymous_enabled: e.target.checked
                      } : null)}
                      className="sr-only peer"
                    />
                    <div className="w-9 h-5 bg-white/10 border border-white/20 rounded-full peer peer-checked:bg-orange-500 peer-checked:border-orange-400 transition-all"></div>
                    <div className="absolute left-0.5 top-0.5 w-4 h-4 bg-white rounded-full transition-transform peer-checked:translate-x-4"></div>
                  </div>
                </label>
              </div>
              {settings.anonymous_enabled && (
                <div className="mt-4 pt-4 border-t border-white/10">
                  <div className="flex items-start gap-2 text-sm text-orange-200/80">
                    <Info className="w-4 h-4 flex-shrink-0 mt-0.5" />
                    <div>
                      <p>When enabled, unauthenticated users can access content with permissions assigned to the "anonymous" role.</p>
                      <p className="mt-1 text-gray-400">Configure anonymous role permissions in Access Control → Roles.</p>
                    </div>
                  </div>
                </div>
              )}
            </div>
          </div>
        )}
      </GlassCard>

      {/* CORS Configuration */}
      <GlassCard>
        <button
          onClick={() => toggleSection('cors')}
          className="w-full flex items-center justify-between"
        >
          <div className="flex items-center gap-3">
            <div className="p-2 rounded-lg bg-cyan-500/20 border border-cyan-500/30">
              <Globe className="w-5 h-5 text-cyan-400" />
            </div>
            <div className="text-left">
              <h2 className="text-lg font-semibold text-white">CORS Configuration</h2>
              <p className="text-sm text-gray-400">Configure allowed origins for cross-origin requests</p>
            </div>
          </div>
          {expandedSections.cors ? (
            <ChevronDown className="w-5 h-5 text-gray-400" />
          ) : (
            <ChevronRight className="w-5 h-5 text-gray-400" />
          )}
        </button>

        {expandedSections.cors && settings && (
          <div className="mt-6 space-y-4">
            {/* Info about hierarchy */}
            <div className="flex items-start gap-2 p-3 bg-cyan-500/10 border border-cyan-500/20 rounded-lg text-sm text-cyan-200/80">
              <Info className="w-4 h-4 flex-shrink-0 mt-0.5" />
              <div>
                <p>CORS configuration follows a hierarchy:</p>
                <ol className="list-decimal list-inside mt-1 text-cyan-300/70 space-y-0.5">
                  <li><strong>Repository-level</strong> (highest priority) - Configured in repository settings</li>
                  <li><strong>Tenant-level</strong> (this page) - Fallback when repo has no CORS config</li>
                  <li><strong>Global</strong> (server config) - Fallback when tenant has no CORS config</li>
                </ol>
              </div>
            </div>

            {/* Add new origin */}
            <div className="flex gap-2">
              <input
                type="text"
                value={newCorsOrigin}
                onChange={(e) => setNewCorsOrigin(e.target.value)}
                placeholder="https://app.example.com"
                className="flex-1 px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder:text-gray-500 focus:border-cyan-400 focus:ring-2 focus:ring-cyan-400/20 transition-all"
                onKeyDown={(e) => {
                  if (e.key === 'Enter' && newCorsOrigin.trim()) {
                    const origin = newCorsOrigin.trim()
                    if (!settings.cors_allowed_origins?.includes(origin)) {
                      setSettings(prev => prev ? {
                        ...prev,
                        cors_allowed_origins: [...(prev.cors_allowed_origins || []), origin]
                      } : null)
                    }
                    setNewCorsOrigin('')
                  }
                }}
              />
              <button
                onClick={() => {
                  const origin = newCorsOrigin.trim()
                  if (origin && !settings.cors_allowed_origins?.includes(origin)) {
                    setSettings(prev => prev ? {
                      ...prev,
                      cors_allowed_origins: [...(prev.cors_allowed_origins || []), origin]
                    } : null)
                  }
                  setNewCorsOrigin('')
                }}
                disabled={!newCorsOrigin.trim()}
                className="flex items-center gap-2 px-4 py-2 bg-cyan-500/20 hover:bg-cyan-500/30 disabled:bg-white/5 disabled:text-gray-500 border border-cyan-500/30 disabled:border-white/10 text-cyan-300 rounded-lg transition-colors"
              >
                <Plus className="w-4 h-4" />
                Add
              </button>
            </div>

            {/* Origins list */}
            {(settings.cors_allowed_origins?.length ?? 0) > 0 ? (
              <div className="space-y-2">
                {settings.cors_allowed_origins?.map((origin, index) => (
                  <div
                    key={index}
                    className="flex items-center justify-between px-4 py-2 bg-white/5 border border-white/10 rounded-lg"
                  >
                    <span className="text-white font-mono text-sm">{origin}</span>
                    <button
                      onClick={() => {
                        setSettings(prev => prev ? {
                          ...prev,
                          cors_allowed_origins: prev.cors_allowed_origins?.filter((_, i) => i !== index) || []
                        } : null)
                      }}
                      className="p-1.5 hover:bg-red-500/20 rounded transition-colors text-gray-400 hover:text-red-400"
                      title="Remove origin"
                    >
                      <Trash2 className="w-4 h-4" />
                    </button>
                  </div>
                ))}
              </div>
            ) : (
              <div className="text-center py-6 text-gray-500">
                <Globe className="w-8 h-8 mx-auto mb-2 opacity-50" />
                <p>No CORS origins configured</p>
                <p className="text-xs mt-1">Add origins to allow cross-origin requests from specific domains</p>
              </div>
            )}
          </div>
        )}
      </GlassCard>

      {/* Session Settings */}
      <GlassCard>
        <button
          onClick={() => toggleSection('session')}
          className="w-full flex items-center justify-between"
        >
          <div className="flex items-center gap-3">
            <div className="p-2 rounded-lg bg-yellow-500/20 border border-yellow-500/30">
              <Clock className="w-5 h-5 text-yellow-400" />
            </div>
            <div className="text-left">
              <h2 className="text-lg font-semibold text-white">Session Settings</h2>
              <p className="text-sm text-gray-400">Configure session duration and limits</p>
            </div>
          </div>
          {expandedSections.session ? (
            <ChevronDown className="w-5 h-5 text-gray-400" />
          ) : (
            <ChevronRight className="w-5 h-5 text-gray-400" />
          )}
        </button>

        {expandedSections.session && settings && (
          <div className="mt-6 grid grid-cols-1 md:grid-cols-2 gap-4">
            <div>
              <label className="block text-sm font-medium text-gray-300 mb-2">Session Duration (hours)</label>
              <input
                type="number"
                value={settings.session_settings.duration_hours}
                onChange={(e) => setSettings(prev => prev ? {
                  ...prev,
                  session_settings: { ...prev.session_settings, duration_hours: parseInt(e.target.value) || 24 }
                } : null)}
                min={1}
                max={720}
                className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20 transition-all"
              />
            </div>
            <div>
              <label className="block text-sm font-medium text-gray-300 mb-2">Refresh Token Duration (days)</label>
              <input
                type="number"
                value={settings.session_settings.refresh_token_duration_days}
                onChange={(e) => setSettings(prev => prev ? {
                  ...prev,
                  session_settings: { ...prev.session_settings, refresh_token_duration_days: parseInt(e.target.value) || 30 }
                } : null)}
                min={1}
                max={365}
                className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20 transition-all"
              />
            </div>
            <div>
              <label className="block text-sm font-medium text-gray-300 mb-2">Max Sessions Per User</label>
              <input
                type="number"
                value={settings.session_settings.max_sessions_per_user}
                onChange={(e) => setSettings(prev => prev ? {
                  ...prev,
                  session_settings: { ...prev.session_settings, max_sessions_per_user: parseInt(e.target.value) || 10 }
                } : null)}
                min={1}
                max={100}
                className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20 transition-all"
              />
            </div>
            <div>
              <label className="flex items-center gap-3 cursor-pointer mt-6">
                <div className="relative">
                  <input
                    type="checkbox"
                    checked={settings.session_settings.single_session_mode}
                    onChange={(e) => setSettings(prev => prev ? {
                      ...prev,
                      session_settings: { ...prev.session_settings, single_session_mode: e.target.checked }
                    } : null)}
                    className="sr-only peer"
                  />
                  <div className="w-11 h-6 bg-white/10 border border-white/20 rounded-full peer peer-checked:bg-purple-500 peer-checked:border-purple-400 transition-all"></div>
                  <div className="absolute left-1 top-1 w-4 h-4 bg-white rounded-full transition-transform peer-checked:translate-x-5"></div>
                </div>
                <span className="text-white font-medium">Single Session Mode</span>
              </label>
              <p className="text-xs text-gray-500 mt-1 ml-14">Logout previous sessions on new login</p>
            </div>
          </div>
        )}
      </GlassCard>

      {/* Password Policy */}
      <GlassCard>
        <button
          onClick={() => toggleSection('password')}
          className="w-full flex items-center justify-between"
        >
          <div className="flex items-center gap-3">
            <div className="p-2 rounded-lg bg-red-500/20 border border-red-500/30">
              <Key className="w-5 h-5 text-red-400" />
            </div>
            <div className="text-left">
              <h2 className="text-lg font-semibold text-white">Password Policy</h2>
              <p className="text-sm text-gray-400">Set password requirements for local authentication</p>
            </div>
          </div>
          {expandedSections.password ? (
            <ChevronDown className="w-5 h-5 text-gray-400" />
          ) : (
            <ChevronRight className="w-5 h-5 text-gray-400" />
          )}
        </button>

        {expandedSections.password && settings && (
          <div className="mt-6 space-y-4">
            <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
              <div>
                <label className="block text-sm font-medium text-gray-300 mb-2">Minimum Length</label>
                <input
                  type="number"
                  value={settings.password_policy.min_length}
                  onChange={(e) => setSettings(prev => prev ? {
                    ...prev,
                    password_policy: { ...prev.password_policy, min_length: parseInt(e.target.value) || 8 }
                  } : null)}
                  min={6}
                  max={128}
                  className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20 transition-all"
                />
              </div>
              <div>
                <label className="block text-sm font-medium text-gray-300 mb-2">Max Age (days)</label>
                <input
                  type="number"
                  value={settings.password_policy.max_age_days || ''}
                  onChange={(e) => setSettings(prev => prev ? {
                    ...prev,
                    password_policy: { ...prev.password_policy, max_age_days: e.target.value ? parseInt(e.target.value) : undefined }
                  } : null)}
                  min={0}
                  placeholder="No expiration"
                  className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder:text-gray-500 focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20 transition-all"
                />
              </div>
            </div>

            <div className="grid grid-cols-2 md:grid-cols-4 gap-4">
              {[
                { key: 'require_uppercase', label: 'Uppercase' },
                { key: 'require_lowercase', label: 'Lowercase' },
                { key: 'require_numbers', label: 'Numbers' },
                { key: 'require_special', label: 'Special chars' },
              ].map(({ key, label }) => (
                <label key={key} className="flex items-center gap-2 cursor-pointer p-3 bg-white/5 rounded-lg border border-white/10 hover:border-white/20 transition-colors">
                  <input
                    type="checkbox"
                    checked={settings.password_policy[key as keyof typeof settings.password_policy] as boolean}
                    onChange={(e) => setSettings(prev => prev ? {
                      ...prev,
                      password_policy: { ...prev.password_policy, [key]: e.target.checked }
                    } : null)}
                    className="w-4 h-4 rounded border-white/20 bg-white/5 text-purple-500 focus:ring-purple-400"
                  />
                  <span className="text-white text-sm">{label}</span>
                </label>
              ))}
            </div>
          </div>
        )}
      </GlassCard>

      {/* Access Control */}
      <GlassCard>
        <button
          onClick={() => toggleSection('access')}
          className="w-full flex items-center justify-between"
        >
          <div className="flex items-center gap-3">
            <div className="p-2 rounded-lg bg-green-500/20 border border-green-500/30">
              <Users className="w-5 h-5 text-green-400" />
            </div>
            <div className="text-left">
              <h2 className="text-lg font-semibold text-white">Access Control</h2>
              <p className="text-sm text-gray-400">Configure workspace access and invitation settings</p>
            </div>
          </div>
          {expandedSections.access ? (
            <ChevronDown className="w-5 h-5 text-gray-400" />
          ) : (
            <ChevronRight className="w-5 h-5 text-gray-400" />
          )}
        </button>

        {expandedSections.access && settings && (
          <div className="mt-6 space-y-4">
            <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
              {[
                { key: 'allow_access_requests', label: 'Allow Access Requests', desc: 'Users can request workspace access' },
                { key: 'allow_invitations', label: 'Allow Invitations', desc: 'Admins can invite users to workspaces' },
                { key: 'require_approval', label: 'Require Approval', desc: 'Access requests require admin approval' },
              ].map(({ key, label, desc }) => (
                <label key={key} className="flex items-start gap-3 cursor-pointer p-4 bg-white/5 rounded-lg border border-white/10 hover:border-white/20 transition-colors">
                  <div className="relative mt-0.5">
                    <input
                      type="checkbox"
                      checked={settings.access_settings[key as keyof typeof settings.access_settings] as boolean}
                      onChange={(e) => setSettings(prev => prev ? {
                        ...prev,
                        access_settings: { ...prev.access_settings, [key]: e.target.checked }
                      } : null)}
                      className="sr-only peer"
                    />
                    <div className="w-9 h-5 bg-white/10 border border-white/20 rounded-full peer peer-checked:bg-green-500 peer-checked:border-green-400 transition-all"></div>
                    <div className="absolute left-0.5 top-0.5 w-4 h-4 bg-white rounded-full transition-transform peer-checked:translate-x-4"></div>
                  </div>
                  <div>
                    <span className="text-white font-medium text-sm">{label}</span>
                    <p className="text-xs text-gray-500 mt-0.5">{desc}</p>
                  </div>
                </label>
              ))}
            </div>

            <div>
              <label className="block text-sm font-medium text-gray-300 mb-2">Default Roles for New Users</label>
              <input
                type="text"
                value={settings.access_settings.default_roles.join(', ')}
                onChange={(e) => setSettings(prev => prev ? {
                  ...prev,
                  access_settings: { ...prev.access_settings, default_roles: e.target.value.split(',').map(r => r.trim()).filter(Boolean) }
                } : null)}
                placeholder="viewer, editor"
                className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder:text-gray-500 focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20 transition-all"
              />
              <p className="text-xs text-gray-500 mt-1">Comma-separated list of roles assigned to new users</p>
            </div>
          </div>
        )}
      </GlassCard>

      {/* Info Card */}
      <div className="flex items-start gap-3 p-4 bg-blue-500/10 border border-blue-500/30 rounded-xl">
        <Info className="w-5 h-5 text-blue-400 flex-shrink-0 mt-0.5" />
        <div className="text-sm text-blue-200">
          <p className="font-medium mb-1">SQL Configuration</p>
          <p className="text-blue-300/80">
            You can also manage authentication settings via SQL using functions like{' '}
            <code className="px-1.5 py-0.5 bg-blue-500/20 rounded text-xs">RAISIN_AUTH_ADD_PROVIDER()</code>,{' '}
            <code className="px-1.5 py-0.5 bg-blue-500/20 rounded text-xs">RAISIN_AUTH_UPDATE_SETTINGS()</code>, and{' '}
            <code className="px-1.5 py-0.5 bg-blue-500/20 rounded text-xs">RAISIN_AUTH_GET_SETTINGS()</code>.
            See the SQL Help panel for examples.
          </p>
        </div>
      </div>
    </div>
  )
}
