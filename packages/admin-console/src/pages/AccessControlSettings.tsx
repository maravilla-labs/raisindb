import { useState, useEffect } from 'react'
import { useParams } from 'react-router-dom'
import {
  Shield,
  Users,
  Loader2,
  Save,
  AlertCircle,
  Baby,
  Lock,
  UserPlus,
  Calendar,
  CheckCircle,
  XCircle,
  ChevronDown,
  ChevronRight,
  UserX,
  Info,
  Globe,
  Plus,
  Trash2,
  MessageCircle,
  ListTodo,
  Settings,
} from 'lucide-react'
import GlassCard from '../components/GlassCard'
import { ToastContainer, useToast } from '../components/Toast'
import { nodesApi } from '../api/nodes'
import Tabs, { Tab } from '../components/Tabs'
import PermissionRuleEditor, { PermissionRuleSet } from '../components/PermissionRuleEditor'

// CORS Configuration
interface CorsConfig {
  cors_allowed_origins: string[]
}

const DEFAULT_CORS_CONFIG: CorsConfig = {
  cors_allowed_origins: [],
}

// Access Control / Stewardship Configuration
interface AccessControlConfig {
  anonymous_enabled: boolean | null
  stewardship_enabled: boolean
  stewardship_relation_types: string[]
  require_minor_for_parent: boolean
  allowed_workflows: string[]
  steward_creates_ward_enabled: boolean
  max_stewards_per_ward: number
  max_wards_per_steward: number
  invitation_expiry_days: number
  require_ward_consent: boolean
  minor_age_threshold: number
  allow_minor_login: boolean
}

const DEFAULT_ACCESS_CONFIG: AccessControlConfig = {
  anonymous_enabled: null,
  stewardship_enabled: false,
  stewardship_relation_types: ['PARENT_OF', 'GUARDIAN_OF'],
  require_minor_for_parent: true,
  allowed_workflows: ['invitation', 'admin_assignment', 'steward_creates_ward'],
  steward_creates_ward_enabled: false,
  max_stewards_per_ward: 5,
  max_wards_per_steward: 10,
  invitation_expiry_days: 7,
  require_ward_consent: true,
  minor_age_threshold: 18,
  allow_minor_login: false,
}

// Messaging Configuration
interface MessagingConfig {
  enabled: boolean
  chat_permissions: PermissionRuleSet
  task_permissions: PermissionRuleSet
  blocked_users_prevent_messaging: boolean
  require_email_verification: boolean
  max_message_length: number
  rate_limit_messages_per_minute: number
}

const DEFAULT_MESSAGING_CONFIG: MessagingConfig = {
  enabled: true,
  chat_permissions: {
    mode: 'any_of',
    rules: [{ type: 'always' }],
  },
  task_permissions: {
    mode: 'any_of',
    rules: [{ type: 'always' }],
  },
  blocked_users_prevent_messaging: true,
  require_email_verification: false,
  max_message_length: 10000,
  rate_limit_messages_per_minute: 60,
}

type TabId = 'general' | 'stewardship' | 'messaging'

const TABS: Tab[] = [
  { id: 'general', label: 'General', icon: Settings },
  { id: 'stewardship', label: 'Stewardship', icon: Users },
  { id: 'messaging', label: 'Messaging', icon: MessageCircle },
]

export default function AccessControlSettings() {
  const { repo } = useParams<{ repo: string }>()
  const { toasts, closeToast, success, error: showError } = useToast()

  // Tab state
  const [activeTab, setActiveTab] = useState<TabId>('general')

  // Loading state
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)

  // Access control config state
  const [config, setConfig] = useState<AccessControlConfig>(DEFAULT_ACCESS_CONFIG)
  const [originalConfig, setOriginalConfig] = useState<AccessControlConfig>(DEFAULT_ACCESS_CONFIG)

  // CORS config state
  const [corsConfig, setCorsConfig] = useState<CorsConfig>(DEFAULT_CORS_CONFIG)
  const [originalCorsConfig, setOriginalCorsConfig] = useState<CorsConfig>(DEFAULT_CORS_CONFIG)
  const [newCorsOrigin, setNewCorsOrigin] = useState('')

  // Messaging config state
  const [messagingConfig, setMessagingConfig] = useState<MessagingConfig>(DEFAULT_MESSAGING_CONFIG)
  const [originalMessagingConfig, setOriginalMessagingConfig] = useState<MessagingConfig>(DEFAULT_MESSAGING_CONFIG)

  // UI State for collapsible sections
  const [expandedSections, setExpandedSections] = useState<Record<string, boolean>>({
    cors: true,
    anonymous: false,
    stewardship: false,
    workflows: false,
    limits: false,
    minors: false,
    messagingGeneral: true,
    chatPermissions: false,
    taskPermissions: false,
  })

  const branch = 'main'
  const accessWorkspace = 'raisin:access_control'
  const accessConfigPath = '/config/stewardship'
  const systemWorkspace = 'raisin:system'
  const corsConfigPath = `/config/repos/${repo}`
  const messagingConfigPath = '/config/messaging'

  // Load data
  useEffect(() => {
    loadConfig()
  }, [repo])

  const loadConfig = async () => {
    if (!repo) return

    setLoading(true)
    try {
      // Load access control config
      const node = await nodesApi.getAtHead(repo, branch, accessWorkspace, accessConfigPath)

      const anonymousValue = node.properties?.anonymous_enabled
      const anonymousEnabled =
        anonymousValue === undefined || anonymousValue === null ? null : (anonymousValue as boolean)

      const nodeConfig: AccessControlConfig = {
        anonymous_enabled: anonymousEnabled,
        stewardship_enabled:
          ((node.properties?.stewardship_enabled ?? node.properties?.enabled) as boolean) ??
          DEFAULT_ACCESS_CONFIG.stewardship_enabled,
        stewardship_relation_types:
          (node.properties?.stewardship_relation_types as string[]) ??
          DEFAULT_ACCESS_CONFIG.stewardship_relation_types,
        require_minor_for_parent:
          (node.properties?.require_minor_for_parent as boolean) ?? DEFAULT_ACCESS_CONFIG.require_minor_for_parent,
        allowed_workflows:
          (node.properties?.allowed_workflows as string[]) ?? DEFAULT_ACCESS_CONFIG.allowed_workflows,
        steward_creates_ward_enabled:
          (node.properties?.steward_creates_ward_enabled as boolean) ??
          DEFAULT_ACCESS_CONFIG.steward_creates_ward_enabled,
        max_stewards_per_ward:
          (node.properties?.max_stewards_per_ward as number) ?? DEFAULT_ACCESS_CONFIG.max_stewards_per_ward,
        max_wards_per_steward:
          (node.properties?.max_wards_per_steward as number) ?? DEFAULT_ACCESS_CONFIG.max_wards_per_steward,
        invitation_expiry_days:
          (node.properties?.invitation_expiry_days as number) ?? DEFAULT_ACCESS_CONFIG.invitation_expiry_days,
        require_ward_consent:
          (node.properties?.require_ward_consent as boolean) ?? DEFAULT_ACCESS_CONFIG.require_ward_consent,
        minor_age_threshold:
          (node.properties?.minor_age_threshold as number) ?? DEFAULT_ACCESS_CONFIG.minor_age_threshold,
        allow_minor_login: (node.properties?.allow_minor_login as boolean) ?? DEFAULT_ACCESS_CONFIG.allow_minor_login,
      }

      setConfig(nodeConfig)
      setOriginalConfig(nodeConfig)

      // Load CORS config from system workspace
      try {
        const corsNode = await nodesApi.getAtHead(repo, branch, systemWorkspace, corsConfigPath)
        const loadedCorsConfig: CorsConfig = {
          cors_allowed_origins: (corsNode.properties?.cors_allowed_origins as string[]) ?? [],
        }
        setCorsConfig(loadedCorsConfig)
        setOriginalCorsConfig(loadedCorsConfig)
      } catch {
        setCorsConfig(DEFAULT_CORS_CONFIG)
        setOriginalCorsConfig(DEFAULT_CORS_CONFIG)
      }

      // Load messaging config
      try {
        const msgNode = await nodesApi.getAtHead(repo, branch, accessWorkspace, messagingConfigPath)
        const loadedMsgConfig: MessagingConfig = {
          enabled: (msgNode.properties?.enabled as boolean) ?? DEFAULT_MESSAGING_CONFIG.enabled,
          chat_permissions:
            (msgNode.properties?.chat_permissions as PermissionRuleSet) ?? DEFAULT_MESSAGING_CONFIG.chat_permissions,
          task_permissions:
            (msgNode.properties?.task_permissions as PermissionRuleSet) ?? DEFAULT_MESSAGING_CONFIG.task_permissions,
          blocked_users_prevent_messaging:
            (msgNode.properties?.blocked_users_prevent_messaging as boolean) ??
            DEFAULT_MESSAGING_CONFIG.blocked_users_prevent_messaging,
          require_email_verification:
            (msgNode.properties?.require_email_verification as boolean) ??
            DEFAULT_MESSAGING_CONFIG.require_email_verification,
          max_message_length:
            (msgNode.properties?.max_message_length as number) ?? DEFAULT_MESSAGING_CONFIG.max_message_length,
          rate_limit_messages_per_minute:
            (msgNode.properties?.rate_limit_messages_per_minute as number) ??
            DEFAULT_MESSAGING_CONFIG.rate_limit_messages_per_minute,
        }
        setMessagingConfig(loadedMsgConfig)
        setOriginalMessagingConfig(loadedMsgConfig)
      } catch {
        setMessagingConfig(DEFAULT_MESSAGING_CONFIG)
        setOriginalMessagingConfig(DEFAULT_MESSAGING_CONFIG)
      }
    } catch (error) {
      showError('Failed to load access control configuration')
      console.error('Error loading config:', error)
    } finally {
      setLoading(false)
    }
  }

  const toggleSection = (section: string) => {
    setExpandedSections((prev) => ({ ...prev, [section]: !prev[section] }))
  }

  const handleSave = async () => {
    if (!repo) return

    setSaving(true)
    try {
      // Save access control config
      const properties: Record<string, unknown> = {
        stewardship_enabled: config.stewardship_enabled,
        stewardship_relation_types: config.stewardship_relation_types,
        require_minor_for_parent: config.require_minor_for_parent,
        allowed_workflows: config.allowed_workflows,
        steward_creates_ward_enabled: config.steward_creates_ward_enabled,
        max_stewards_per_ward: config.max_stewards_per_ward,
        max_wards_per_steward: config.max_wards_per_steward,
        invitation_expiry_days: config.invitation_expiry_days,
        require_ward_consent: config.require_ward_consent,
        minor_age_threshold: config.minor_age_threshold,
        allow_minor_login: config.allow_minor_login,
      }

      if (config.anonymous_enabled !== null) {
        properties.anonymous_enabled = config.anonymous_enabled
      }

      await nodesApi.update(repo, branch, accessWorkspace, accessConfigPath, {
        properties,
        commit: {
          message: 'Update access control configuration',
          actor: 'admin-console',
        },
      })

      // Save CORS config if changed
      if (JSON.stringify(corsConfig) !== JSON.stringify(originalCorsConfig)) {
        try {
          await nodesApi.update(repo, branch, systemWorkspace, corsConfigPath, {
            properties: {
              cors_allowed_origins: corsConfig.cors_allowed_origins,
            },
            commit: {
              message: 'Update CORS configuration for auth endpoints',
              actor: 'admin-console',
            },
          })
        } catch {
          const repoConfigParentPath = '/config/repos'
          await nodesApi.create(repo, branch, systemWorkspace, repoConfigParentPath, {
            name: repo,
            node_type: 'raisin:RepoAuthConfig',
            properties: {
              cors_allowed_origins: corsConfig.cors_allowed_origins,
            },
            commit: {
              message: 'Create CORS configuration for auth endpoints',
              actor: 'admin-console',
            },
          })
        }
        setOriginalCorsConfig(corsConfig)
      }

      // Save messaging config if changed
      if (JSON.stringify(messagingConfig) !== JSON.stringify(originalMessagingConfig)) {
        try {
          await nodesApi.update(repo, branch, accessWorkspace, messagingConfigPath, {
            properties: messagingConfig as unknown as Record<string, unknown>,
            commit: {
              message: 'Update messaging configuration',
              actor: 'admin-console',
            },
          })
        } catch {
          const configParentPath = '/config'
          await nodesApi.create(repo, branch, accessWorkspace, configParentPath, {
            name: 'messaging',
            node_type: 'raisin:MessagingConfig',
            properties: messagingConfig as unknown as Record<string, unknown>,
            commit: {
              message: 'Create messaging configuration',
              actor: 'admin-console',
            },
          })
        }
        setOriginalMessagingConfig(messagingConfig)
      }

      setOriginalConfig(config)
      success('Settings saved successfully')
    } catch (error) {
      showError(`Failed to save settings: ${error instanceof Error ? error.message : 'Unknown error'}`)
    } finally {
      setSaving(false)
    }
  }

  // CORS origin management
  const addCorsOrigin = () => {
    const origin = newCorsOrigin.trim()
    if (!origin) return

    try {
      new URL(origin)
    } catch {
      showError('Invalid URL format. Please enter a valid origin (e.g., http://localhost:5173)')
      return
    }

    if (corsConfig.cors_allowed_origins.includes(origin)) {
      showError('This origin is already in the list')
      return
    }

    setCorsConfig((prev) => ({
      ...prev,
      cors_allowed_origins: [...prev.cors_allowed_origins, origin],
    }))
    setNewCorsOrigin('')
  }

  const removeCorsOrigin = (origin: string) => {
    setCorsConfig((prev) => ({
      ...prev,
      cors_allowed_origins: prev.cors_allowed_origins.filter((o) => o !== origin),
    }))
  }

  const hasChanges =
    JSON.stringify(config) !== JSON.stringify(originalConfig) ||
    JSON.stringify(corsConfig) !== JSON.stringify(originalCorsConfig) ||
    JSON.stringify(messagingConfig) !== JSON.stringify(originalMessagingConfig)

  if (loading) {
    return (
      <div className="flex items-center justify-center min-h-[400px]">
        <Loader2 className="w-8 h-8 animate-spin text-purple-400" />
      </div>
    )
  }

  return (
    <div className="space-y-6 max-w-5xl">
      <ToastContainer toasts={toasts} onClose={closeToast} />

      {/* Header */}
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-3">
          <div className="p-3 bg-purple-500/20 rounded-xl border border-purple-500/30">
            <Shield className="w-6 h-6 text-purple-400" />
          </div>
          <div>
            <h1 className="text-2xl font-bold text-white">Access Control Settings</h1>
            <p className="text-gray-400 text-sm mt-0.5">Configure stewardship, messaging, and access policies</p>
          </div>
        </div>
        <button
          onClick={handleSave}
          disabled={!hasChanges || saving}
          className="flex items-center gap-2 px-4 py-2 bg-purple-500 hover:bg-purple-600 disabled:bg-purple-500/50 disabled:cursor-not-allowed text-white rounded-lg transition-colors"
        >
          {saving ? <Loader2 className="w-4 h-4 animate-spin" /> : <Save className="w-4 h-4" />}
          Save Changes
        </button>
      </div>

      {/* Tabs */}
      <Tabs tabs={TABS} activeTab={activeTab} onChange={(id) => setActiveTab(id as TabId)}>
        {/* General Tab */}
        {activeTab === 'general' && (
          <div className="space-y-6">
            {/* CORS Configuration */}
            <GlassCard>
              <button onClick={() => toggleSection('cors')} className="w-full flex items-center justify-between">
                <div className="flex items-center gap-3">
                  <div className="p-2 rounded-lg bg-cyan-500/20 border border-cyan-500/30">
                    <Globe className="w-5 h-5 text-cyan-400" />
                  </div>
                  <div className="text-left">
                    <h2 className="text-lg font-semibold text-white">CORS Configuration</h2>
                    <p className="text-sm text-gray-400">Configure allowed origins for authentication endpoints</p>
                  </div>
                </div>
                {expandedSections.cors ? (
                  <ChevronDown className="w-5 h-5 text-gray-400" />
                ) : (
                  <ChevronRight className="w-5 h-5 text-gray-400" />
                )}
              </button>

              {expandedSections.cors && (
                <div className="mt-6 space-y-4">
                  <div className="flex items-start gap-2 p-3 bg-cyan-500/10 border border-cyan-500/30 rounded-lg text-sm">
                    <Info className="w-4 h-4 text-cyan-400 flex-shrink-0 mt-0.5" />
                    <p className="text-cyan-200/80">
                      These origins are allowed to make cross-origin requests to authentication endpoints (
                      <code className="px-1 bg-black/20 rounded">/auth/{repo}/register</code>,{' '}
                      <code className="px-1 bg-black/20 rounded">/auth/{repo}/login</code>, etc.). Required for frontend
                      apps hosted on different domains.
                    </p>
                  </div>

                  <div className="flex gap-2">
                    <input
                      type="text"
                      value={newCorsOrigin}
                      onChange={(e) => setNewCorsOrigin(e.target.value)}
                      onKeyDown={(e) => e.key === 'Enter' && addCorsOrigin()}
                      placeholder="https://example.com or http://localhost:5173"
                      className="flex-1 px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-gray-500 focus:border-cyan-400 focus:ring-2 focus:ring-cyan-400/20 transition-all"
                    />
                    <button
                      onClick={addCorsOrigin}
                      className="flex items-center gap-2 px-4 py-2 bg-cyan-500 hover:bg-cyan-600 text-white rounded-lg transition-colors"
                    >
                      <Plus className="w-4 h-4" />
                      Add
                    </button>
                  </div>

                  {corsConfig.cors_allowed_origins.length > 0 ? (
                    <div className="space-y-2">
                      {corsConfig.cors_allowed_origins.map((origin) => (
                        <div
                          key={origin}
                          className="flex items-center justify-between p-3 bg-white/5 border border-white/10 rounded-lg"
                        >
                          <code className="text-cyan-300">{origin}</code>
                          <button
                            onClick={() => removeCorsOrigin(origin)}
                            className="p-1.5 hover:bg-red-500/20 rounded-lg text-gray-400 hover:text-red-400 transition-colors"
                            title="Remove origin"
                          >
                            <Trash2 className="w-4 h-4" />
                          </button>
                        </div>
                      ))}
                    </div>
                  ) : (
                    <div className="p-4 bg-amber-500/10 border border-amber-500/30 rounded-lg">
                      <div className="flex items-start gap-3">
                        <AlertCircle className="w-5 h-5 text-amber-400 mt-0.5 flex-shrink-0" />
                        <div>
                          <h3 className="text-amber-200 font-semibold mb-1">No CORS Origins Configured</h3>
                          <p className="text-amber-100/80 text-sm">
                            Cross-origin requests to authentication endpoints will be blocked. Add your frontend
                            application origin to enable authentication from your app.
                          </p>
                        </div>
                      </div>
                    </div>
                  )}
                </div>
              )}
            </GlassCard>

            {/* Anonymous Access */}
            <GlassCard>
              <button onClick={() => toggleSection('anonymous')} className="w-full flex items-center justify-between">
                <div className="flex items-center gap-3">
                  <div className="p-2 rounded-lg bg-orange-500/20 border border-orange-500/30">
                    <UserX className="w-5 h-5 text-orange-400" />
                  </div>
                  <div className="text-left">
                    <h2 className="text-lg font-semibold text-white">Anonymous Access</h2>
                    <p className="text-sm text-gray-400">Configure unauthenticated access for this repository</p>
                  </div>
                </div>
                {expandedSections.anonymous ? (
                  <ChevronDown className="w-5 h-5 text-gray-400" />
                ) : (
                  <ChevronRight className="w-5 h-5 text-gray-400" />
                )}
              </button>

              {expandedSections.anonymous && (
                <div className="mt-6 space-y-4">
                  <div className="space-y-3">
                    <label
                      className={`flex items-start gap-3 cursor-pointer p-4 rounded-lg border transition-colors ${
                        config.anonymous_enabled === null
                          ? 'bg-purple-500/10 border-purple-500/30'
                          : 'bg-white/5 border-white/10 hover:border-white/20'
                      }`}
                    >
                      <input
                        type="radio"
                        name="anonymous_mode"
                        checked={config.anonymous_enabled === null}
                        onChange={() => setConfig((prev) => ({ ...prev, anonymous_enabled: null }))}
                        className="mt-1 w-4 h-4 text-purple-500 border-white/20 bg-white/5 focus:ring-purple-400"
                      />
                      <div>
                        <span className="text-white font-medium">Inherit from Global Settings</span>
                        <p className="text-xs text-gray-400 mt-0.5">Use the tenant-level anonymous access setting</p>
                      </div>
                    </label>

                    <label
                      className={`flex items-start gap-3 cursor-pointer p-4 rounded-lg border transition-colors ${
                        config.anonymous_enabled === true
                          ? 'bg-orange-500/10 border-orange-500/30'
                          : 'bg-white/5 border-white/10 hover:border-white/20'
                      }`}
                    >
                      <input
                        type="radio"
                        name="anonymous_mode"
                        checked={config.anonymous_enabled === true}
                        onChange={() => setConfig((prev) => ({ ...prev, anonymous_enabled: true }))}
                        className="mt-1 w-4 h-4 text-orange-500 border-white/20 bg-white/5 focus:ring-orange-400"
                      />
                      <div>
                        <span className="text-white font-medium">Enable Anonymous Access</span>
                        <p className="text-xs text-gray-400 mt-0.5">
                          Allow unauthenticated users to access this repository with the "anonymous" role
                        </p>
                      </div>
                    </label>

                    <label
                      className={`flex items-start gap-3 cursor-pointer p-4 rounded-lg border transition-colors ${
                        config.anonymous_enabled === false
                          ? 'bg-red-500/10 border-red-500/30'
                          : 'bg-white/5 border-white/10 hover:border-white/20'
                      }`}
                    >
                      <input
                        type="radio"
                        name="anonymous_mode"
                        checked={config.anonymous_enabled === false}
                        onChange={() => setConfig((prev) => ({ ...prev, anonymous_enabled: false }))}
                        className="mt-1 w-4 h-4 text-red-500 border-white/20 bg-white/5 focus:ring-red-400"
                      />
                      <div>
                        <span className="text-white font-medium">Disable Anonymous Access</span>
                        <p className="text-xs text-gray-400 mt-0.5">
                          Require authentication for all access to this repository
                        </p>
                      </div>
                    </label>
                  </div>

                  {config.anonymous_enabled !== null && (
                    <div className="flex items-start gap-2 p-3 bg-blue-500/10 border border-blue-500/30 rounded-lg text-sm">
                      <Info className="w-4 h-4 text-blue-400 flex-shrink-0 mt-0.5" />
                      <p className="text-blue-200/80">
                        This setting overrides the global anonymous access configuration for this repository only.
                      </p>
                    </div>
                  )}
                </div>
              )}
            </GlassCard>
          </div>
        )}

        {/* Stewardship Tab */}
        {activeTab === 'stewardship' && (
          <div className="space-y-6">
            {/* Stewardship Configuration */}
            <GlassCard>
              <button
                onClick={() => toggleSection('stewardship')}
                className="w-full flex items-center justify-between"
              >
                <div className="flex items-center gap-3">
                  <div className="p-2 rounded-lg bg-blue-500/20 border border-blue-500/30">
                    <Users className="w-5 h-5 text-blue-400" />
                  </div>
                  <div className="text-left">
                    <h2 className="text-lg font-semibold text-white">Stewardship Configuration</h2>
                    <p className="text-sm text-gray-400">Enable and configure stewardship relationships</p>
                  </div>
                </div>
                {expandedSections.stewardship ? (
                  <ChevronDown className="w-5 h-5 text-gray-400" />
                ) : (
                  <ChevronRight className="w-5 h-5 text-gray-400" />
                )}
              </button>

              {expandedSections.stewardship && (
                <div className="mt-6 space-y-6">
                  <div
                    className={`p-4 rounded-lg border ${config.stewardship_enabled ? 'bg-green-500/10 border-green-500/30' : 'bg-white/5 border-white/10'}`}
                  >
                    <div className="flex items-center justify-between">
                      <div className="flex items-center gap-3">
                        <Shield className="w-5 h-5 text-gray-400" />
                        <div>
                          <p className="font-medium text-white">Enable Stewardship System</p>
                          <p className="text-xs text-gray-400">
                            Allow users to act as stewards for other users (wards)
                          </p>
                        </div>
                      </div>
                      <label className="flex items-center gap-2 cursor-pointer">
                        <div className="relative">
                          <input
                            type="checkbox"
                            checked={config.stewardship_enabled}
                            onChange={(e) => setConfig((prev) => ({ ...prev, stewardship_enabled: e.target.checked }))}
                            className="sr-only peer"
                          />
                          <div className="w-11 h-6 bg-white/10 border border-white/20 rounded-full peer peer-checked:bg-green-500 peer-checked:border-green-400 transition-all"></div>
                          <div className="absolute left-1 top-1 w-4 h-4 bg-white rounded-full transition-transform peer-checked:translate-x-5"></div>
                        </div>
                      </label>
                    </div>
                  </div>

                  <div>
                    <label className="block text-sm font-medium text-gray-300 mb-2">Stewardship Relation Types</label>
                    <p className="text-xs text-gray-500 mb-3">
                      Select which relation types grant stewardship permissions
                    </p>
                    <div className="grid grid-cols-2 gap-3">
                      {['PARENT_OF', 'GUARDIAN_OF', 'MANAGER_OF'].map((relType) => (
                        <label
                          key={relType}
                          className="flex items-center gap-2 cursor-pointer p-3 bg-white/5 rounded-lg border border-white/10 hover:border-white/20 transition-colors"
                        >
                          <input
                            type="checkbox"
                            checked={config.stewardship_relation_types.includes(relType)}
                            onChange={(e) => {
                              setConfig((prev) => ({
                                ...prev,
                                stewardship_relation_types: e.target.checked
                                  ? [...prev.stewardship_relation_types, relType]
                                  : prev.stewardship_relation_types.filter((t) => t !== relType),
                              }))
                            }}
                            className="w-4 h-4 rounded border-white/20 bg-white/5 text-purple-500 focus:ring-purple-400"
                          />
                          <span className="text-white text-sm">{relType.replace(/_/g, ' ')}</span>
                        </label>
                      ))}
                    </div>
                  </div>

                  {config.stewardship_relation_types.includes('PARENT_OF') && (
                    <div className="p-4 bg-amber-500/10 border border-amber-500/30 rounded-lg">
                      <label className="flex items-start gap-3 cursor-pointer">
                        <input
                          type="checkbox"
                          checked={config.require_minor_for_parent}
                          onChange={(e) =>
                            setConfig((prev) => ({ ...prev, require_minor_for_parent: e.target.checked }))
                          }
                          className="mt-0.5 w-4 h-4 rounded border-white/20 bg-white/5 text-amber-500 focus:ring-amber-400"
                        />
                        <div>
                          <span className="text-white font-medium text-sm">Require Minor for PARENT_OF</span>
                          <p className="text-xs text-amber-200/80 mt-0.5">
                            PARENT_OF relation only grants stewardship if the target is below the minor age threshold
                          </p>
                        </div>
                      </label>
                    </div>
                  )}
                </div>
              )}
            </GlassCard>

            {/* Allowed Workflows */}
            <GlassCard>
              <button onClick={() => toggleSection('workflows')} className="w-full flex items-center justify-between">
                <div className="flex items-center gap-3">
                  <div className="p-2 rounded-lg bg-green-500/20 border border-green-500/30">
                    <UserPlus className="w-5 h-5 text-green-400" />
                  </div>
                  <div className="text-left">
                    <h2 className="text-lg font-semibold text-white">Allowed Workflows</h2>
                    <p className="text-sm text-gray-400">Configure how stewardship relationships can be created</p>
                  </div>
                </div>
                {expandedSections.workflows ? (
                  <ChevronDown className="w-5 h-5 text-gray-400" />
                ) : (
                  <ChevronRight className="w-5 h-5 text-gray-400" />
                )}
              </button>

              {expandedSections.workflows && (
                <div className="mt-6 space-y-4">
                  <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
                    {[
                      { key: 'invitation', label: 'Invitation', desc: 'Ward invites steward' },
                      { key: 'admin_assignment', label: 'Admin Assignment', desc: 'Admin assigns steward to ward' },
                      {
                        key: 'steward_creates_ward',
                        label: 'Steward Creates Ward',
                        desc: 'Steward creates new ward account',
                      },
                    ].map(({ key, label, desc }) => (
                      <label
                        key={key}
                        className="flex items-start gap-3 cursor-pointer p-4 bg-white/5 rounded-lg border border-white/10 hover:border-white/20 transition-colors"
                      >
                        <div className="relative mt-0.5">
                          <input
                            type="checkbox"
                            checked={config.allowed_workflows.includes(key)}
                            onChange={(e) => {
                              setConfig((prev) => ({
                                ...prev,
                                allowed_workflows: e.target.checked
                                  ? [...prev.allowed_workflows, key]
                                  : prev.allowed_workflows.filter((w) => w !== key),
                              }))
                            }}
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

                  {config.allowed_workflows.includes('steward_creates_ward') && (
                    <div className="p-4 bg-purple-500/10 border border-purple-500/30 rounded-lg">
                      <label className="flex items-start gap-3 cursor-pointer">
                        <input
                          type="checkbox"
                          checked={config.steward_creates_ward_enabled}
                          onChange={(e) =>
                            setConfig((prev) => ({ ...prev, steward_creates_ward_enabled: e.target.checked }))
                          }
                          className="mt-0.5 w-4 h-4 rounded border-white/20 bg-white/5 text-purple-500 focus:ring-purple-400"
                        />
                        <div>
                          <span className="text-white font-medium text-sm">Enable Steward Account Creation</span>
                          <p className="text-xs text-purple-200/80 mt-0.5">
                            Allow stewards to create new ward accounts directly
                          </p>
                        </div>
                      </label>
                    </div>
                  )}

                  <div className="p-4 bg-blue-500/10 border border-blue-500/30 rounded-lg">
                    <label className="flex items-start gap-3 cursor-pointer">
                      <input
                        type="checkbox"
                        checked={config.require_ward_consent}
                        onChange={(e) => setConfig((prev) => ({ ...prev, require_ward_consent: e.target.checked }))}
                        className="mt-0.5 w-4 h-4 rounded border-white/20 bg-white/5 text-blue-500 focus:ring-blue-400"
                      />
                      <div>
                        <span className="text-white font-medium text-sm">Require Ward Consent</span>
                        <p className="text-xs text-blue-200/80 mt-0.5">
                          Ward must accept stewardship invitation (except for minors or admin assignments)
                        </p>
                      </div>
                    </label>
                  </div>
                </div>
              )}
            </GlassCard>

            {/* Limits */}
            <GlassCard>
              <button onClick={() => toggleSection('limits')} className="w-full flex items-center justify-between">
                <div className="flex items-center gap-3">
                  <div className="p-2 rounded-lg bg-yellow-500/20 border border-yellow-500/30">
                    <Lock className="w-5 h-5 text-yellow-400" />
                  </div>
                  <div className="text-left">
                    <h2 className="text-lg font-semibold text-white">Relationship Limits</h2>
                    <p className="text-sm text-gray-400">Set maximum limits for stewardship relationships</p>
                  </div>
                </div>
                {expandedSections.limits ? (
                  <ChevronDown className="w-5 h-5 text-gray-400" />
                ) : (
                  <ChevronRight className="w-5 h-5 text-gray-400" />
                )}
              </button>

              {expandedSections.limits && (
                <div className="mt-6 grid grid-cols-1 md:grid-cols-3 gap-4">
                  <div>
                    <label className="block text-sm font-medium text-gray-300 mb-2">Max Stewards per Ward</label>
                    <input
                      type="number"
                      value={config.max_stewards_per_ward}
                      onChange={(e) =>
                        setConfig((prev) => ({ ...prev, max_stewards_per_ward: parseInt(e.target.value) || 5 }))
                      }
                      min={1}
                      max={100}
                      className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20 transition-all"
                    />
                    <p className="text-xs text-gray-500 mt-1">Maximum stewards a single ward can have</p>
                  </div>

                  <div>
                    <label className="block text-sm font-medium text-gray-300 mb-2">Max Wards per Steward</label>
                    <input
                      type="number"
                      value={config.max_wards_per_steward}
                      onChange={(e) =>
                        setConfig((prev) => ({ ...prev, max_wards_per_steward: parseInt(e.target.value) || 10 }))
                      }
                      min={1}
                      max={1000}
                      className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20 transition-all"
                    />
                    <p className="text-xs text-gray-500 mt-1">Maximum wards a single steward can manage</p>
                  </div>

                  <div>
                    <label className="block text-sm font-medium text-gray-300 mb-2">Invitation Expiry (days)</label>
                    <input
                      type="number"
                      value={config.invitation_expiry_days}
                      onChange={(e) =>
                        setConfig((prev) => ({ ...prev, invitation_expiry_days: parseInt(e.target.value) || 7 }))
                      }
                      min={1}
                      max={365}
                      className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20 transition-all"
                    />
                    <p className="text-xs text-gray-500 mt-1">Days until invitations expire</p>
                  </div>
                </div>
              )}
            </GlassCard>

            {/* Minor Configuration */}
            <GlassCard>
              <button onClick={() => toggleSection('minors')} className="w-full flex items-center justify-between">
                <div className="flex items-center gap-3">
                  <div className="p-2 rounded-lg bg-pink-500/20 border border-pink-500/30">
                    <Baby className="w-5 h-5 text-pink-400" />
                  </div>
                  <div className="text-left">
                    <h2 className="text-lg font-semibold text-white">Minor Protection</h2>
                    <p className="text-sm text-gray-400">Configure settings for users below the age threshold</p>
                  </div>
                </div>
                {expandedSections.minors ? (
                  <ChevronDown className="w-5 h-5 text-gray-400" />
                ) : (
                  <ChevronRight className="w-5 h-5 text-gray-400" />
                )}
              </button>

              {expandedSections.minors && (
                <div className="mt-6 space-y-4">
                  <div>
                    <label className="block text-sm font-medium text-gray-300 mb-2">Minor Age Threshold</label>
                    <input
                      type="number"
                      value={config.minor_age_threshold}
                      onChange={(e) =>
                        setConfig((prev) => ({ ...prev, minor_age_threshold: parseInt(e.target.value) || 18 }))
                      }
                      min={1}
                      max={100}
                      className="w-full md:w-64 px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20 transition-all"
                    />
                    <p className="text-xs text-gray-500 mt-1">Age below which a user is considered a minor</p>
                  </div>

                  <div
                    className={`p-4 rounded-lg border ${config.allow_minor_login ? 'bg-green-500/10 border-green-500/30' : 'bg-red-500/10 border-red-500/30'}`}
                  >
                    <div className="flex items-center justify-between">
                      <div className="flex items-center gap-3">
                        <Lock className="w-5 h-5 text-gray-400" />
                        <div>
                          <p className="font-medium text-white">Allow Minor Login</p>
                          <p className="text-xs text-gray-400">Whether minors can log in directly to the system</p>
                        </div>
                      </div>
                      <label className="flex items-center gap-2 cursor-pointer">
                        <div className="relative">
                          <input
                            type="checkbox"
                            checked={config.allow_minor_login}
                            onChange={(e) => setConfig((prev) => ({ ...prev, allow_minor_login: e.target.checked }))}
                            className="sr-only peer"
                          />
                          <div
                            className={`w-11 h-6 border rounded-full transition-all ${config.allow_minor_login ? 'bg-green-500 border-green-400' : 'bg-red-500/50 border-red-500/30'}`}
                          ></div>
                          <div className="absolute left-1 top-1 w-4 h-4 bg-white rounded-full transition-transform peer-checked:translate-x-5"></div>
                        </div>
                      </label>
                    </div>
                  </div>

                  <div className="p-4 bg-amber-500/10 border border-amber-500/30 rounded-lg">
                    <div className="flex items-start gap-3">
                      <AlertCircle className="w-5 h-5 text-amber-400 mt-0.5 flex-shrink-0" />
                      <div>
                        <h3 className="text-amber-200 font-semibold mb-1">Minor Protection Notice</h3>
                        <p className="text-amber-100/80 text-sm">
                          Minors require at least one steward. If minor login is disabled, stewards must manage their
                          accounts. Ensure appropriate workflows are enabled for minor protection.
                        </p>
                      </div>
                    </div>
                  </div>
                </div>
              )}
            </GlassCard>
          </div>
        )}

        {/* Messaging Tab */}
        {activeTab === 'messaging' && (
          <div className="space-y-6">
            {/* Messaging General Settings */}
            <GlassCard>
              <button
                onClick={() => toggleSection('messagingGeneral')}
                className="w-full flex items-center justify-between"
              >
                <div className="flex items-center gap-3">
                  <div className="p-2 rounded-lg bg-purple-500/20 border border-purple-500/30">
                    <MessageCircle className="w-5 h-5 text-purple-400" />
                  </div>
                  <div className="text-left">
                    <h2 className="text-lg font-semibold text-white">Messaging Configuration</h2>
                    <p className="text-sm text-gray-400">General messaging settings and limits</p>
                  </div>
                </div>
                {expandedSections.messagingGeneral ? (
                  <ChevronDown className="w-5 h-5 text-gray-400" />
                ) : (
                  <ChevronRight className="w-5 h-5 text-gray-400" />
                )}
              </button>

              {expandedSections.messagingGeneral && (
                <div className="mt-6 space-y-6">
                  {/* Enable Messaging */}
                  <div
                    className={`p-4 rounded-lg border ${messagingConfig.enabled ? 'bg-green-500/10 border-green-500/30' : 'bg-white/5 border-white/10'}`}
                  >
                    <div className="flex items-center justify-between">
                      <div className="flex items-center gap-3">
                        <MessageCircle className="w-5 h-5 text-gray-400" />
                        <div>
                          <p className="font-medium text-white">Enable Messaging</p>
                          <p className="text-xs text-gray-400">Allow users to send messages to each other</p>
                        </div>
                      </div>
                      <label className="flex items-center gap-2 cursor-pointer">
                        <div className="relative">
                          <input
                            type="checkbox"
                            checked={messagingConfig.enabled}
                            onChange={(e) => setMessagingConfig((prev) => ({ ...prev, enabled: e.target.checked }))}
                            className="sr-only peer"
                          />
                          <div className="w-11 h-6 bg-white/10 border border-white/20 rounded-full peer peer-checked:bg-green-500 peer-checked:border-green-400 transition-all"></div>
                          <div className="absolute left-1 top-1 w-4 h-4 bg-white rounded-full transition-transform peer-checked:translate-x-5"></div>
                        </div>
                      </label>
                    </div>
                  </div>

                  {/* Additional Options */}
                  <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
                    <div
                      className={`p-4 rounded-lg border ${messagingConfig.blocked_users_prevent_messaging ? 'bg-blue-500/10 border-blue-500/30' : 'bg-white/5 border-white/10'}`}
                    >
                      <label className="flex items-start gap-3 cursor-pointer">
                        <input
                          type="checkbox"
                          checked={messagingConfig.blocked_users_prevent_messaging}
                          onChange={(e) =>
                            setMessagingConfig((prev) => ({
                              ...prev,
                              blocked_users_prevent_messaging: e.target.checked,
                            }))
                          }
                          className="mt-0.5 w-4 h-4 rounded border-white/20 bg-white/5 text-blue-500 focus:ring-blue-400"
                        />
                        <div>
                          <span className="text-white font-medium text-sm">Block Prevents Messaging</span>
                          <p className="text-xs text-gray-400 mt-0.5">
                            Blocked users cannot send messages to each other
                          </p>
                        </div>
                      </label>
                    </div>

                    <div
                      className={`p-4 rounded-lg border ${messagingConfig.require_email_verification ? 'bg-amber-500/10 border-amber-500/30' : 'bg-white/5 border-white/10'}`}
                    >
                      <label className="flex items-start gap-3 cursor-pointer">
                        <input
                          type="checkbox"
                          checked={messagingConfig.require_email_verification}
                          onChange={(e) =>
                            setMessagingConfig((prev) => ({ ...prev, require_email_verification: e.target.checked }))
                          }
                          className="mt-0.5 w-4 h-4 rounded border-white/20 bg-white/5 text-amber-500 focus:ring-amber-400"
                        />
                        <div>
                          <span className="text-white font-medium text-sm">Require Email Verification</span>
                          <p className="text-xs text-gray-400 mt-0.5">
                            Users must verify email before sending messages
                          </p>
                        </div>
                      </label>
                    </div>
                  </div>

                  {/* Limits */}
                  <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
                    <div>
                      <label className="block text-sm font-medium text-gray-300 mb-2">Max Message Length</label>
                      <input
                        type="number"
                        value={messagingConfig.max_message_length}
                        onChange={(e) =>
                          setMessagingConfig((prev) => ({
                            ...prev,
                            max_message_length: parseInt(e.target.value) || 10000,
                          }))
                        }
                        min={100}
                        max={100000}
                        className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20 transition-all"
                      />
                      <p className="text-xs text-gray-500 mt-1">Maximum characters per message</p>
                    </div>

                    <div>
                      <label className="block text-sm font-medium text-gray-300 mb-2">Rate Limit (per minute)</label>
                      <input
                        type="number"
                        value={messagingConfig.rate_limit_messages_per_minute}
                        onChange={(e) =>
                          setMessagingConfig((prev) => ({
                            ...prev,
                            rate_limit_messages_per_minute: parseInt(e.target.value) || 60,
                          }))
                        }
                        min={1}
                        max={1000}
                        className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20 transition-all"
                      />
                      <p className="text-xs text-gray-500 mt-1">Maximum messages a user can send per minute</p>
                    </div>
                  </div>
                </div>
              )}
            </GlassCard>

            {/* Chat Permissions */}
            <PermissionRuleEditor
              title="Chat Permissions"
              description="Configure who can send chat messages to whom"
              value={messagingConfig.chat_permissions}
              onChange={(chat_permissions) => setMessagingConfig((prev) => ({ ...prev, chat_permissions }))}
              expanded={expandedSections.chatPermissions}
              onToggle={() => toggleSection('chatPermissions')}
              icon={MessageCircle}
              iconColor="blue"
            />

            {/* Task Permissions */}
            <PermissionRuleEditor
              title="Task Permissions"
              description="Configure who can assign tasks to whom"
              value={messagingConfig.task_permissions}
              onChange={(task_permissions) => setMessagingConfig((prev) => ({ ...prev, task_permissions }))}
              expanded={expandedSections.taskPermissions}
              onToggle={() => toggleSection('taskPermissions')}
              icon={ListTodo}
              iconColor="green"
            />
          </div>
        )}
      </Tabs>

      {/* Status Summary */}
      <div className="flex items-start gap-3 p-4 bg-blue-500/10 border border-blue-500/30 rounded-xl">
        <AlertCircle className="w-5 h-5 text-blue-400 flex-shrink-0 mt-0.5" />
        <div className="text-sm text-blue-200">
          <p className="font-medium mb-1">Configuration Status</p>
          <div className="space-y-1 text-blue-300/80">
            <div className="flex items-center gap-2">
              {config.anonymous_enabled === null ? (
                <>
                  <UserX className="w-4 h-4 text-purple-400" />
                  <span>Anonymous access: Inheriting from global settings</span>
                </>
              ) : config.anonymous_enabled ? (
                <>
                  <CheckCircle className="w-4 h-4 text-orange-400" />
                  <span>Anonymous access: Enabled for this repository</span>
                </>
              ) : (
                <>
                  <XCircle className="w-4 h-4 text-red-400" />
                  <span>Anonymous access: Disabled for this repository</span>
                </>
              )}
            </div>
            <div className="flex items-center gap-2">
              {config.stewardship_enabled ? (
                <>
                  <CheckCircle className="w-4 h-4 text-green-400" />
                  <span>Stewardship system is enabled</span>
                </>
              ) : (
                <>
                  <XCircle className="w-4 h-4 text-red-400" />
                  <span>Stewardship system is disabled</span>
                </>
              )}
            </div>
            <div className="flex items-center gap-2">
              {messagingConfig.enabled ? (
                <>
                  <CheckCircle className="w-4 h-4 text-green-400" />
                  <span>Messaging is enabled</span>
                </>
              ) : (
                <>
                  <XCircle className="w-4 h-4 text-red-400" />
                  <span>Messaging is disabled</span>
                </>
              )}
            </div>
            <div className="flex items-center gap-2">
              <Calendar className="w-4 h-4" />
              <span>{config.allowed_workflows.length} workflows enabled</span>
            </div>
            <div className="flex items-center gap-2">
              <Users className="w-4 h-4" />
              <span>{config.stewardship_relation_types.length} relation types configured</span>
            </div>
            <div className="flex items-center gap-2">
              {corsConfig.cors_allowed_origins.length > 0 ? (
                <>
                  <Globe className="w-4 h-4 text-cyan-400" />
                  <span>
                    {corsConfig.cors_allowed_origins.length} CORS origin
                    {corsConfig.cors_allowed_origins.length > 1 ? 's' : ''} configured
                  </span>
                </>
              ) : (
                <>
                  <Globe className="w-4 h-4 text-amber-400" />
                  <span>No CORS origins configured (auth blocked from external apps)</span>
                </>
              )}
            </div>
            <div className="flex items-center gap-2">
              <MessageCircle className="w-4 h-4" />
              <span>
                Chat: {messagingConfig.chat_permissions.rules.length} rule
                {messagingConfig.chat_permissions.rules.length !== 1 ? 's' : ''} ({messagingConfig.chat_permissions.mode}
                )
              </span>
            </div>
          </div>
        </div>
      </div>
    </div>
  )
}
