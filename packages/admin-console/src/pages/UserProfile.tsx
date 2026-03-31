import { useState, useEffect } from 'react'
import { useAuth } from '../contexts/AuthContext'
import { profileApi, UserProfile as UserProfileType } from '../api/profile'
import { apiKeysApi, ApiKey, CreateApiKeyResponse } from '../api/api-keys'
import { ApiError } from '../api/client'
import {
  User,
  Key,
  Database,
  Shield,
  Terminal,
  Code,
  AlertCircle,
  Plus,
  Copy,
  Eye,
  EyeOff,
  CheckCircle,
  XCircle,
  Trash2,
  RefreshCw,
} from 'lucide-react'
import ConfirmDialog from '../components/ConfirmDialog'
import { useToast, ToastContainer } from '../components/Toast'

export default function UserProfile() {
  const { tenantId } = useAuth()
  const [profile, setProfile] = useState<UserProfileType | null>(null)
  const [apiKeys, setApiKeys] = useState<ApiKey[]>([])
  const [repositories, setRepositories] = useState<string[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)

  // Create key modal
  const [showCreateModal, setShowCreateModal] = useState(false)
  const [newKeyName, setNewKeyName] = useState('')
  const [creating, setCreating] = useState(false)
  const [createError, setCreateError] = useState<string | null>(null)

  // New key display
  const [newKeyResponse, setNewKeyResponse] = useState<CreateApiKeyResponse | null>(null)
  const [showToken, setShowToken] = useState(false)
  const [copied, setCopied] = useState(false)

  // Selected repository for connection string
  const [selectedRepo, setSelectedRepo] = useState<string>('')
  const [showConnectionToken, setShowConnectionToken] = useState(false)

  // Delete confirmation
  const [deleteConfirm, setDeleteConfirm] = useState<{ keyId: string; name: string } | null>(null)

  const { toasts, success: showSuccess, error: showError, closeToast } = useToast()

  const loadData = async () => {
    try {
      setLoading(true)
      const [profileData, keysData, reposData] = await Promise.all([
        profileApi.get(),
        apiKeysApi.list(),
        profileApi.getRepositories(),
      ])
      setProfile(profileData)
      setApiKeys(keysData)
      setRepositories(reposData)
      if (reposData.length > 0 && !selectedRepo) {
        setSelectedRepo(reposData[0])
      }
      setError(null)
    } catch (err) {
      if (err instanceof ApiError) {
        setError(err.message)
      } else {
        setError('Failed to load profile data')
      }
    } finally {
      setLoading(false)
    }
  }

  useEffect(() => {
    loadData()
  }, [])

  const handleCreateKey = async (e: React.FormEvent) => {
    e.preventDefault()
    setCreating(true)
    setCreateError(null)

    try {
      const response = await apiKeysApi.create(newKeyName)
      setNewKeyResponse(response)
      setNewKeyName('')
      setShowCreateModal(false)
      loadData() // Refresh the list
    } catch (err) {
      if (err instanceof ApiError) {
        setCreateError(err.message)
      } else {
        setCreateError('Failed to create API key')
      }
    } finally {
      setCreating(false)
    }
  }

  const handleRevokeKey = async (keyId: string) => {
    try {
      await apiKeysApi.revoke(keyId)
      showSuccess('Success', 'API key revoked successfully')
      loadData()
    } catch (err) {
      if (err instanceof ApiError) {
        showError('Error', `Failed to revoke key: ${err.message}`)
      } else {
        showError('Error', 'Failed to revoke API key')
      }
    }
  }

  const copyToClipboard = async (text: string) => {
    try {
      await navigator.clipboard.writeText(text)
      setCopied(true)
      setTimeout(() => setCopied(false), 2000)
    } catch {
      showError('Error', 'Failed to copy to clipboard')
    }
  }

  const getConnectionString = (token: string) => {
    const host = window.location.hostname
    return `postgresql://${tenantId}:${token}@${host}:5432/${selectedRepo}`
  }

  const getPsqlCommand = (token: string) => {
    return `psql "${getConnectionString(token)}"`
  }

  if (loading) {
    return (
      <div className="flex items-center justify-center min-h-[60vh]">
        <div className="flex flex-col items-center gap-4">
          <div className="animate-spin rounded-full h-12 w-12 border-b-2 border-primary-500"></div>
          <p className="text-zinc-400 text-sm">Loading profile...</p>
        </div>
      </div>
    )
  }

  if (!profile) {
    return (
      <div className="max-w-4xl mx-auto">
        <div className="p-6 rounded-lg bg-red-500/10 border border-red-500/20 flex items-start gap-3">
          <AlertCircle className="w-5 h-5 text-red-400 flex-shrink-0 mt-0.5" />
          <p className="text-sm text-red-200">{error || 'Failed to load profile'}</p>
        </div>
      </div>
    )
  }

  return (
    <div className="max-w-4xl mx-auto">
      {/* Header */}
      <div className="mb-8">
        <h1 className="text-3xl font-bold text-white mb-2">My Profile</h1>
        <p className="text-zinc-400">
          Manage your account settings and API access keys
        </p>
      </div>

      {error && (
        <div className="mb-6 p-4 rounded-lg bg-red-500/10 border border-red-500/20 flex items-start gap-3">
          <AlertCircle className="w-5 h-5 text-red-400 flex-shrink-0 mt-0.5" />
          <p className="text-sm text-red-200">{error}</p>
        </div>
      )}

      {/* Profile Info Card */}
      <div className="glass-dark rounded-xl border border-white/10 p-6 mb-6">
        <div className="flex items-center gap-4 mb-6">
          <div className="p-3 rounded-xl bg-primary-500/10 border border-primary-500/20">
            <User className="w-8 h-8 text-primary-400" />
          </div>
          <div>
            <h2 className="text-xl font-bold text-white">{profile.username}</h2>
            <p className="text-zinc-400">{profile.email || 'No email set'}</p>
          </div>
        </div>

        <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
          <div>
            <span className="text-sm text-zinc-500">Tenant</span>
            <p className="text-white font-medium">{profile.tenant_id}</p>
          </div>
          <div>
            <span className="text-sm text-zinc-500">Access Permissions</span>
            <div className="flex flex-wrap gap-1.5 mt-1">
              {profile.access_flags.console_login && (
                <span className="inline-flex items-center gap-1 rounded-md bg-blue-500/10 border border-blue-500/20 px-2 py-1 text-xs font-medium text-blue-300">
                  <Shield className="w-3 h-3" />
                  Console
                </span>
              )}
              {profile.access_flags.cli_access && (
                <span className="inline-flex items-center gap-1 rounded-md bg-green-500/10 border border-green-500/20 px-2 py-1 text-xs font-medium text-green-300">
                  <Terminal className="w-3 h-3" />
                  CLI
                </span>
              )}
              {profile.access_flags.api_access && (
                <span className="inline-flex items-center gap-1 rounded-md bg-purple-500/10 border border-purple-500/20 px-2 py-1 text-xs font-medium text-purple-300">
                  <Code className="w-3 h-3" />
                  API
                </span>
              )}
              {profile.access_flags.pgwire_access && (
                <span className="inline-flex items-center gap-1 rounded-md bg-orange-500/10 border border-orange-500/20 px-2 py-1 text-xs font-medium text-orange-300">
                  <Database className="w-3 h-3" />
                  PostgreSQL
                </span>
              )}
            </div>
          </div>
        </div>
      </div>

      {/* API Keys Section */}
      <div className="glass-dark rounded-xl border border-white/10 p-6 mb-6">
        <div className="flex items-center justify-between mb-6">
          <div className="flex items-center gap-3">
            <div className="p-2 rounded-lg bg-yellow-500/10 border border-yellow-500/20">
              <Key className="w-6 h-6 text-yellow-400" />
            </div>
            <div>
              <h2 className="text-xl font-bold text-white">API Access Keys</h2>
              <p className="text-sm text-zinc-400">Create and manage API keys for programmatic access</p>
            </div>
          </div>
          <button
            onClick={() => setShowCreateModal(true)}
            className="flex items-center gap-2 px-4 py-2 bg-primary-500 hover:bg-primary-600 text-white font-medium rounded-lg transition-all duration-200 hover:scale-[1.02] active:scale-[0.98]"
          >
            <Plus className="w-4 h-4" />
            Generate Key
          </button>
        </div>

        {/* New Key Alert */}
        {newKeyResponse && (
          <div className="mb-6 p-4 rounded-lg bg-green-500/10 border border-green-500/20">
            <div className="flex items-start gap-3">
              <CheckCircle className="w-5 h-5 text-green-400 flex-shrink-0 mt-0.5" />
              <div className="flex-1">
                <h3 className="font-semibold text-green-300 mb-2">API Key Created Successfully!</h3>
                <p className="text-sm text-green-200 mb-3">
                  Copy your API key now. For security, it won't be shown again.
                </p>
                <div className="flex items-center gap-2">
                  <div className="flex-1 font-mono text-sm bg-black/30 rounded-lg px-3 py-2 text-white overflow-x-auto">
                    {showToken ? newKeyResponse.token : '••••••••••••••••••••••••••••••••'}
                  </div>
                  <button
                    onClick={() => setShowToken(!showToken)}
                    className="p-2 hover:bg-white/10 rounded-lg transition-colors"
                    title={showToken ? 'Hide' : 'Show'}
                  >
                    {showToken ? (
                      <EyeOff className="w-5 h-5 text-zinc-400" />
                    ) : (
                      <Eye className="w-5 h-5 text-zinc-400" />
                    )}
                  </button>
                  <button
                    onClick={() => copyToClipboard(newKeyResponse.token)}
                    className="p-2 hover:bg-white/10 rounded-lg transition-colors"
                    title="Copy"
                  >
                    {copied ? (
                      <CheckCircle className="w-5 h-5 text-green-400" />
                    ) : (
                      <Copy className="w-5 h-5 text-zinc-400" />
                    )}
                  </button>
                </div>
                <button
                  onClick={() => setNewKeyResponse(null)}
                  className="mt-3 text-sm text-green-300 hover:text-green-200"
                >
                  Dismiss
                </button>
              </div>
            </div>
          </div>
        )}

        {/* API Keys Table */}
        <div className="overflow-x-auto">
          <table className="min-w-full divide-y divide-white/10">
            <thead>
              <tr>
                <th className="py-3 px-4 text-left text-sm font-semibold text-zinc-300">Name</th>
                <th className="py-3 px-4 text-left text-sm font-semibold text-zinc-300">Key Prefix</th>
                <th className="py-3 px-4 text-left text-sm font-semibold text-zinc-300">Created</th>
                <th className="py-3 px-4 text-left text-sm font-semibold text-zinc-300">Last Used</th>
                <th className="py-3 px-4 text-left text-sm font-semibold text-zinc-300">Status</th>
                <th className="py-3 px-4"></th>
              </tr>
            </thead>
            <tbody className="divide-y divide-white/5">
              {apiKeys.length === 0 ? (
                <tr>
                  <td colSpan={6} className="py-8 text-center text-zinc-500">
                    No API keys yet. Create one to get started.
                  </td>
                </tr>
              ) : (
                apiKeys.map((key) => (
                  <tr key={key.key_id} className="hover:bg-white/5 transition-colors">
                    <td className="py-3 px-4 text-sm text-white font-medium">{key.name}</td>
                    <td className="py-3 px-4 text-sm font-mono text-zinc-400">{key.key_prefix}...</td>
                    <td className="py-3 px-4 text-sm text-zinc-400">
                      {new Date(key.created_at).toLocaleDateString()}
                    </td>
                    <td className="py-3 px-4 text-sm text-zinc-400">
                      {key.last_used_at ? new Date(key.last_used_at).toLocaleDateString() : 'Never'}
                    </td>
                    <td className="py-3 px-4">
                      {key.is_active ? (
                        <span className="inline-flex items-center gap-1 rounded-full bg-green-500/10 border border-green-500/20 px-2.5 py-0.5 text-xs font-medium text-green-300">
                          <CheckCircle className="w-3 h-3" />
                          Active
                        </span>
                      ) : (
                        <span className="inline-flex items-center gap-1 rounded-full bg-red-500/10 border border-red-500/20 px-2.5 py-0.5 text-xs font-medium text-red-300">
                          <XCircle className="w-3 h-3" />
                          Revoked
                        </span>
                      )}
                    </td>
                    <td className="py-3 px-4 text-right">
                      {key.is_active && (
                        <button
                          onClick={() => setDeleteConfirm({ keyId: key.key_id, name: key.name })}
                          className="inline-flex items-center gap-1.5 px-3 py-1.5 text-red-400 hover:text-red-300 hover:bg-red-500/10 border border-transparent hover:border-red-500/20 rounded-lg transition-all duration-200"
                        >
                          <Trash2 className="w-4 h-4" />
                          Revoke
                        </button>
                      )}
                    </td>
                  </tr>
                ))
              )}
            </tbody>
          </table>
        </div>
      </div>

      {/* PostgreSQL Connection Section - Only shown if pgwire_access is true */}
      {profile.access_flags.pgwire_access && (
        <div className="glass-dark rounded-xl border border-white/10 p-6">
          <div className="flex items-center gap-3 mb-6">
            <div className="p-2 rounded-lg bg-orange-500/10 border border-orange-500/20">
              <Database className="w-6 h-6 text-orange-400" />
            </div>
            <div>
              <h2 className="text-xl font-bold text-white">PostgreSQL Connection</h2>
              <p className="text-sm text-zinc-400">Connect to RaisinDB using PostgreSQL-compatible clients</p>
            </div>
          </div>

          {repositories.length === 0 ? (
            <p className="text-zinc-500">No repositories available.</p>
          ) : (
            <>
              <div className="mb-4">
                <label className="block text-sm font-medium text-zinc-300 mb-2">Repository</label>
                <select
                  value={selectedRepo}
                  onChange={(e) => setSelectedRepo(e.target.value)}
                  className="w-full md:w-64 px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white focus:outline-none focus:ring-2 focus:ring-primary-500"
                >
                  {repositories.map((repo) => (
                    <option key={repo} value={repo} className="bg-zinc-900">
                      {repo}
                    </option>
                  ))}
                </select>
              </div>

              <div className="space-y-4">
                <div>
                  <label className="block text-sm font-medium text-zinc-300 mb-2">Connection String</label>
                  <div className="flex items-center gap-2">
                    <div className="flex-1 font-mono text-sm bg-black/30 rounded-lg px-3 py-2 text-zinc-300 overflow-x-auto">
                      {showConnectionToken && newKeyResponse
                        ? getConnectionString(newKeyResponse.token)
                        : `postgresql://${tenantId}:YOUR_API_KEY@${window.location.hostname}:5432/${selectedRepo}`}
                    </div>
                    {newKeyResponse && (
                      <>
                        <button
                          onClick={() => setShowConnectionToken(!showConnectionToken)}
                          className="p-2 hover:bg-white/10 rounded-lg transition-colors"
                          title={showConnectionToken ? 'Hide' : 'Show with token'}
                        >
                          {showConnectionToken ? (
                            <EyeOff className="w-5 h-5 text-zinc-400" />
                          ) : (
                            <Eye className="w-5 h-5 text-zinc-400" />
                          )}
                        </button>
                        <button
                          onClick={() => copyToClipboard(getConnectionString(newKeyResponse.token))}
                          className="p-2 hover:bg-white/10 rounded-lg transition-colors"
                          title="Copy"
                        >
                          <Copy className="w-5 h-5 text-zinc-400" />
                        </button>
                      </>
                    )}
                  </div>
                </div>

                <div>
                  <label className="block text-sm font-medium text-zinc-300 mb-2">psql Command</label>
                  <div className="flex items-center gap-2">
                    <div className="flex-1 font-mono text-sm bg-black/30 rounded-lg px-3 py-2 text-zinc-300 overflow-x-auto">
                      {showConnectionToken && newKeyResponse
                        ? getPsqlCommand(newKeyResponse.token)
                        : `psql "postgresql://${tenantId}:YOUR_API_KEY@${window.location.hostname}:5432/${selectedRepo}"`}
                    </div>
                    {newKeyResponse && (
                      <button
                        onClick={() => copyToClipboard(getPsqlCommand(newKeyResponse.token))}
                        className="p-2 hover:bg-white/10 rounded-lg transition-colors"
                        title="Copy"
                      >
                        <Copy className="w-5 h-5 text-zinc-400" />
                      </button>
                    )}
                  </div>
                </div>

                {!newKeyResponse && (
                  <p className="text-sm text-zinc-500">
                    Generate an API key above to see the complete connection string with your token.
                  </p>
                )}
              </div>
            </>
          )}
        </div>
      )}

      {/* Create Key Modal */}
      {showCreateModal && (
        <div className="fixed inset-0 z-50 overflow-y-auto">
          <div className="flex items-center justify-center min-h-screen px-4 pt-4 pb-20 text-center sm:p-0">
            <div
              className="fixed inset-0 bg-black/60 backdrop-blur-sm transition-opacity"
              onClick={() => !creating && setShowCreateModal(false)}
            ></div>

            <div className="inline-block align-bottom glass-dark rounded-2xl border border-white/10 px-6 pt-6 pb-6 text-left overflow-hidden shadow-2xl transform transition-all sm:my-8 sm:align-middle sm:max-w-lg sm:w-full">
              <form onSubmit={handleCreateKey}>
                <div className="flex items-center gap-3 mb-6">
                  <div className="p-2 rounded-lg bg-primary-500/10 border border-primary-500/20">
                    <Key className="w-6 h-6 text-primary-400" />
                  </div>
                  <h3 className="text-2xl font-bold text-white">Generate API Key</h3>
                </div>

                {createError && (
                  <div className="mb-6 p-4 rounded-lg bg-red-500/10 border border-red-500/20 flex items-start gap-3">
                    <AlertCircle className="w-5 h-5 text-red-400 flex-shrink-0 mt-0.5" />
                    <p className="text-sm text-red-200">{createError}</p>
                  </div>
                )}

                <div className="mb-6">
                  <label htmlFor="keyName" className="block text-sm font-medium text-zinc-300 mb-2">
                    Key Name
                  </label>
                  <input
                    type="text"
                    id="keyName"
                    required
                    value={newKeyName}
                    onChange={(e) => setNewKeyName(e.target.value)}
                    disabled={creating}
                    className="w-full px-4 py-3 bg-white/5 border border-white/10 rounded-lg text-white placeholder-zinc-500 focus:outline-none focus:ring-2 focus:ring-primary-500 focus:border-transparent transition-all disabled:opacity-50"
                    placeholder="e.g., CI/CD Pipeline, Development"
                  />
                  <p className="mt-2 text-sm text-zinc-500">
                    Give your key a descriptive name to remember its purpose.
                  </p>
                </div>

                <div className="flex gap-3 pt-4 border-t border-white/10">
                  <button
                    type="button"
                    onClick={() => setShowCreateModal(false)}
                    disabled={creating}
                    className="flex-1 py-3 px-4 bg-white/5 hover:bg-white/10 border border-white/10 text-zinc-300 font-medium rounded-lg transition-all disabled:opacity-50"
                  >
                    Cancel
                  </button>
                  <button
                    type="submit"
                    disabled={creating}
                    className="flex-1 py-3 px-4 bg-primary-500 hover:bg-primary-600 text-white font-medium rounded-lg transition-all disabled:opacity-50 hover:scale-[1.02] active:scale-[0.98]"
                  >
                    {creating ? (
                      <div className="flex items-center justify-center gap-2">
                        <RefreshCw className="w-5 h-5 animate-spin" />
                        Generating...
                      </div>
                    ) : (
                      'Generate Key'
                    )}
                  </button>
                </div>
              </form>
            </div>
          </div>
        </div>
      )}

      {/* Delete Confirmation */}
      <ConfirmDialog
        open={deleteConfirm !== null}
        title="Revoke API Key"
        message={`Are you sure you want to revoke the API key "${deleteConfirm?.name}"? This action cannot be undone and any applications using this key will stop working.`}
        variant="danger"
        confirmText="Revoke"
        onConfirm={() => {
          if (deleteConfirm) {
            handleRevokeKey(deleteConfirm.keyId)
          }
          setDeleteConfirm(null)
        }}
        onCancel={() => setDeleteConfirm(null)}
      />

      <ToastContainer toasts={toasts} onClose={closeToast} />
    </div>
  )
}
