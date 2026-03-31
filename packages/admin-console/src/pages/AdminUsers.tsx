import { useState, useEffect } from 'react'
import { useAuth } from '../contexts/AuthContext'
import { adminUsersApi, AdminUser } from '../api/admin-users'
import { ApiError } from '../api/client'
import {
  UserPlus,
  Mail,
  Lock,
  AlertCircle,
  Shield,
  Terminal,
  Code,
  Database,
  CheckCircle,
  XCircle,
  Trash2,
  AlertTriangle,
  Edit,
  Eye,
} from 'lucide-react'
import ConfirmDialog from '../components/ConfirmDialog'
import { useToast, ToastContainer } from '../components/Toast'

export default function AdminUsers() {
  const { tenantId } = useAuth()
  const [users, setUsers] = useState<AdminUser[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [showCreateModal, setShowCreateModal] = useState(false)
  const [showEditModal, setShowEditModal] = useState(false)
  const [selectedUser, setSelectedUser] = useState<AdminUser | null>(null)

  // Form state for creating user
  const [newUsername, setNewUsername] = useState('')
  const [newEmail, setNewEmail] = useState('')
  const [newPassword, setNewPassword] = useState('')
  const [consoleLogin, setConsoleLogin] = useState(true)
  const [cliAccess, setCliAccess] = useState(true)
  const [apiAccess, setApiAccess] = useState(true)
  const [pgwireAccess, setPgwireAccess] = useState(false)
  const [canImpersonate, setCanImpersonate] = useState(false)
  const [creating, setCreating] = useState(false)
  const [createError, setCreateError] = useState<string | null>(null)

  // Form state for editing user
  const [editEmail, setEditEmail] = useState('')
  const [editPassword, setEditPassword] = useState('')
  const [changePassword, setChangePassword] = useState(false)
  const [editConsoleLogin, setEditConsoleLogin] = useState(true)
  const [editCliAccess, setEditCliAccess] = useState(true)
  const [editApiAccess, setEditApiAccess] = useState(true)
  const [editPgwireAccess, setEditPgwireAccess] = useState(false)
  const [editCanImpersonate, setEditCanImpersonate] = useState(false)
  const [editing, setEditing] = useState(false)
  const [editError, setEditError] = useState<string | null>(null)
  const [deleteConfirm, setDeleteConfirm] = useState<{ message: string; onConfirm: () => void } | null>(null)
  const { toasts, error: showError, closeToast } = useToast()

  const loadUsers = async () => {
    try {
      setLoading(true)
      const data = await adminUsersApi.list(tenantId)
      setUsers(data)
      setError(null)
    } catch (err) {
      if (err instanceof ApiError) {
        setError(err.message)
      } else {
        setError('Failed to load admin users')
      }
    } finally {
      setLoading(false)
    }
  }

  useEffect(() => {
    loadUsers()
  }, [tenantId])

  const handleCreateUser = async (e: React.FormEvent) => {
    e.preventDefault()
    setCreating(true)
    setCreateError(null)

    try {
      await adminUsersApi.create(tenantId, {
        username: newUsername,
        email: newEmail || undefined,
        password: newPassword,
        access_flags: {
          console_login: consoleLogin,
          cli_access: cliAccess,
          api_access: apiAccess,
          pgwire_access: pgwireAccess,
          can_impersonate: canImpersonate
        }
      })

      // Clear form and reload
      setNewUsername('')
      setNewEmail('')
      setNewPassword('')
      setShowCreateModal(false)
      loadUsers()
    } catch (err) {
      if (err instanceof ApiError) {
        setCreateError(err.message)
      } else {
        setCreateError('Failed to create user')
      }
    } finally {
      setCreating(false)
    }
  }

  const handleDeleteUser = async (username: string) => {
    setDeleteConfirm({
      message: `Are you sure you want to delete user "${username}"?`,
      onConfirm: async () => {
        try {
          await adminUsersApi.delete(tenantId, username)
          loadUsers()
        } catch (err) {
          if (err instanceof ApiError) {
            showError('Error', `Failed to delete user: ${err.message}`)
          } else {
            showError('Error', 'Failed to delete user')
          }
        }
      }
    })
  }

  const handleToggleActive = async (user: AdminUser) => {
    try {
      await adminUsersApi.update(tenantId, user.username, {
        is_active: !user.is_active
      })
      loadUsers()
    } catch (err) {
      if (err instanceof ApiError) {
        showError('Error', `Failed to update user: ${err.message}`)
      } else {
        showError('Error', 'Failed to update user')
      }
    }
  }

  const handleEditUser = (user: AdminUser) => {
    setSelectedUser(user)
    setEditEmail(user.email || '')
    setEditPassword('')
    setChangePassword(false)
    setEditConsoleLogin(user.access_flags.console_login)
    setEditCliAccess(user.access_flags.cli_access)
    setEditApiAccess(user.access_flags.api_access)
    setEditPgwireAccess(user.access_flags.pgwire_access)
    setEditCanImpersonate(user.access_flags.can_impersonate ?? false)
    setEditError(null)
    setShowEditModal(true)
  }

  const handleUpdateUser = async (e: React.FormEvent) => {
    e.preventDefault()
    if (!selectedUser) return

    setEditing(true)
    setEditError(null)

    try {
      const updates: any = {
        access_flags: {
          console_login: editConsoleLogin,
          cli_access: editCliAccess,
          api_access: editApiAccess,
          pgwire_access: editPgwireAccess,
          can_impersonate: editCanImpersonate
        }
      }

      // Only include email if it's different
      if (editEmail !== selectedUser.email) {
        updates.email = editEmail || undefined
      }

      // Only include password if user wants to change it
      if (changePassword && editPassword) {
        updates.password = editPassword
      }

      await adminUsersApi.update(tenantId, selectedUser.username, updates)

      // Clear form and reload
      setShowEditModal(false)
      setSelectedUser(null)
      loadUsers()
    } catch (err) {
      if (err instanceof ApiError) {
        setEditError(err.message)
      } else {
        setEditError('Failed to update user')
      }
    } finally {
      setEditing(false)
    }
  }

  if (loading) {
    return (
      <div className="flex items-center justify-center min-h-[60vh]">
        <div className="flex flex-col items-center gap-4">
          <div className="animate-spin rounded-full h-12 w-12 border-b-2 border-primary-500"></div>
          <p className="text-zinc-400 text-sm">Loading admin users...</p>
        </div>
      </div>
    )
  }

  return (
    <div className="max-w-7xl mx-auto">
      {/* Header */}
      <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4 mb-8">
        <div>
          <h1 className="text-3xl font-bold text-white mb-2">Admin Users</h1>
          <p className="text-zinc-400">
            Manage admin users and access control for the RaisinDB admin console
          </p>
        </div>
        <button
          onClick={() => setShowCreateModal(true)}
          className="flex items-center gap-2 px-6 py-3 bg-primary-500 hover:bg-primary-600 text-white font-medium rounded-lg transition-all duration-200 hover:scale-[1.02] active:scale-[0.98] shadow-lg shadow-primary-500/20"
        >
          <UserPlus className="w-5 h-5" />
          <span>Add User</span>
        </button>
      </div>

      {error && (
        <div className="mb-6 p-4 rounded-lg bg-red-500/10 border border-red-500/20 flex items-start gap-3">
          <AlertCircle className="w-5 h-5 text-red-400 flex-shrink-0 mt-0.5" />
          <p className="text-sm text-red-200">{error}</p>
        </div>
      )}

      {/* Users Table */}
      <div className="glass-dark rounded-xl border border-white/10 overflow-hidden">
        <div className="overflow-x-auto">
          <table className="min-w-full divide-y divide-white/10">
            <thead>
              <tr className="border-b border-white/10">
                <th className="py-4 pl-6 pr-3 text-left text-sm font-semibold text-zinc-300">
                  Username
                </th>
                <th className="px-3 py-4 text-left text-sm font-semibold text-zinc-300">
                  Email
                </th>
                <th className="px-3 py-4 text-left text-sm font-semibold text-zinc-300">
                  Access Permissions
                </th>
                <th className="px-3 py-4 text-left text-sm font-semibold text-zinc-300">
                  Status
                </th>
                <th className="px-3 py-4 text-left text-sm font-semibold text-zinc-300">
                  Created
                </th>
                <th className="relative py-4 pl-3 pr-6">
                  <span className="sr-only">Actions</span>
                </th>
              </tr>
            </thead>
            <tbody className="divide-y divide-white/5">
              {users.map((user) => (
                <tr
                  key={user.user_id}
                  className="hover:bg-white/5 transition-colors duration-150"
                >
                  <td className="whitespace-nowrap py-4 pl-6 pr-3 text-sm font-medium text-white">
                    <div className="flex items-center gap-2">
                      <span>{user.username}</span>
                      {user.must_change_password && (
                        <span className="inline-flex items-center gap-1 rounded-full bg-yellow-500/10 border border-yellow-500/20 px-2.5 py-0.5 text-xs font-medium text-yellow-300">
                          <AlertTriangle className="w-3 h-3" />
                          Must change password
                        </span>
                      )}
                    </div>
                  </td>
                  <td className="whitespace-nowrap px-3 py-4 text-sm text-zinc-400">
                    {user.email || <span className="text-zinc-600">—</span>}
                  </td>
                  <td className="px-3 py-4 text-sm">
                    <div className="flex flex-wrap gap-1.5">
                      {user.access_flags.console_login && (
                        <span className="inline-flex items-center gap-1 rounded-md bg-blue-500/10 border border-blue-500/20 px-2 py-1 text-xs font-medium text-blue-300">
                          <Shield className="w-3 h-3" />
                          Console
                        </span>
                      )}
                      {user.access_flags.cli_access && (
                        <span className="inline-flex items-center gap-1 rounded-md bg-green-500/10 border border-green-500/20 px-2 py-1 text-xs font-medium text-green-300">
                          <Terminal className="w-3 h-3" />
                          CLI
                        </span>
                      )}
                      {user.access_flags.api_access && (
                        <span className="inline-flex items-center gap-1 rounded-md bg-purple-500/10 border border-purple-500/20 px-2 py-1 text-xs font-medium text-purple-300">
                          <Code className="w-3 h-3" />
                          API
                        </span>
                      )}
                      {user.access_flags.pgwire_access && (
                        <span className="inline-flex items-center gap-1 rounded-md bg-orange-500/10 border border-orange-500/20 px-2 py-1 text-xs font-medium text-orange-300">
                          <Database className="w-3 h-3" />
                          PostgreSQL
                        </span>
                      )}
                      {user.access_flags.can_impersonate && (
                        <span className="inline-flex items-center gap-1 rounded-md bg-amber-500/10 border border-amber-500/20 px-2 py-1 text-xs font-medium text-amber-300">
                          <Eye className="w-3 h-3" />
                          Impersonate
                        </span>
                      )}
                    </div>
                  </td>
                  <td className="whitespace-nowrap px-3 py-4 text-sm">
                    <button
                      onClick={() => handleToggleActive(user)}
                      className={`inline-flex items-center gap-1.5 rounded-full px-3 py-1 text-xs font-medium transition-all duration-200 ${
                        user.is_active
                          ? 'bg-green-500/10 border border-green-500/20 text-green-300 hover:bg-green-500/20'
                          : 'bg-red-500/10 border border-red-500/20 text-red-300 hover:bg-red-500/20'
                      }`}
                    >
                      {user.is_active ? (
                        <>
                          <CheckCircle className="w-3 h-3" />
                          Active
                        </>
                      ) : (
                        <>
                          <XCircle className="w-3 h-3" />
                          Inactive
                        </>
                      )}
                    </button>
                  </td>
                  <td className="whitespace-nowrap px-3 py-4 text-sm text-zinc-400">
                    {new Date(user.created_at).toLocaleDateString()}
                  </td>
                  <td className="relative whitespace-nowrap py-4 pl-3 pr-6 text-right text-sm">
                    <div className="flex items-center justify-end gap-2">
                      <button
                        onClick={() => handleEditUser(user)}
                        className="inline-flex items-center gap-1.5 px-3 py-1.5 text-primary-400 hover:text-primary-300 hover:bg-primary-500/10 border border-transparent hover:border-primary-500/20 rounded-lg transition-all duration-200"
                      >
                        <Edit className="w-4 h-4" />
                        Edit
                      </button>
                      <button
                        onClick={() => handleDeleteUser(user.username)}
                        className="inline-flex items-center gap-1.5 px-3 py-1.5 text-red-400 hover:text-red-300 hover:bg-red-500/10 border border-transparent hover:border-red-500/20 rounded-lg transition-all duration-200"
                      >
                        <Trash2 className="w-4 h-4" />
                        Delete
                      </button>
                    </div>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </div>

      {/* Create User Modal */}
      {showCreateModal && (
        <div className="fixed inset-0 z-50 overflow-y-auto">
          <div className="flex items-center justify-center min-h-screen px-4 pt-4 pb-20 text-center sm:p-0">
            {/* Backdrop */}
            <div
              className="fixed inset-0 bg-black/60 backdrop-blur-sm transition-opacity"
              onClick={() => !creating && setShowCreateModal(false)}
            ></div>

            {/* Modal */}
            <div className="inline-block align-bottom glass-dark rounded-2xl border border-white/10 px-6 pt-6 pb-6 text-left overflow-hidden shadow-2xl transform transition-all sm:my-8 sm:align-middle sm:max-w-lg sm:w-full">
              <form onSubmit={handleCreateUser}>
                <div>
                  <div className="flex items-center gap-3 mb-6">
                    <div className="p-2 rounded-lg bg-primary-500/10 border border-primary-500/20">
                      <UserPlus className="w-6 h-6 text-primary-400" />
                    </div>
                    <h3 className="text-2xl font-bold text-white">Create Admin User</h3>
                  </div>

                  {createError && (
                    <div className="mb-6 p-4 rounded-lg bg-red-500/10 border border-red-500/20 flex items-start gap-3">
                      <AlertCircle className="w-5 h-5 text-red-400 flex-shrink-0 mt-0.5" />
                      <p className="text-sm text-red-200">{createError}</p>
                    </div>
                  )}
                  <div className="space-y-5">
                    {/* Username */}
                    <div>
                      <label htmlFor="username" className="block text-sm font-medium text-zinc-300 mb-2">
                        Username
                      </label>
                      <div className="relative">
                        <UserPlus className="absolute left-3 top-1/2 -translate-y-1/2 w-5 h-5 text-zinc-500" />
                        <input
                          type="text"
                          id="username"
                          required
                          value={newUsername}
                          onChange={(e) => setNewUsername(e.target.value)}
                          disabled={creating}
                          className="w-full pl-10 pr-4 py-3 bg-white/5 border border-white/10 rounded-lg text-white placeholder-zinc-500 focus:outline-none focus:ring-2 focus:ring-primary-500 focus:border-transparent transition-all disabled:opacity-50 disabled:cursor-not-allowed"
                          placeholder="Enter username"
                        />
                      </div>
                    </div>

                    {/* Email */}
                    <div>
                      <label htmlFor="email" className="block text-sm font-medium text-zinc-300 mb-2">
                        Email <span className="text-zinc-500">(optional)</span>
                      </label>
                      <div className="relative">
                        <Mail className="absolute left-3 top-1/2 -translate-y-1/2 w-5 h-5 text-zinc-500" />
                        <input
                          type="email"
                          id="email"
                          value={newEmail}
                          onChange={(e) => setNewEmail(e.target.value)}
                          disabled={creating}
                          className="w-full pl-10 pr-4 py-3 bg-white/5 border border-white/10 rounded-lg text-white placeholder-zinc-500 focus:outline-none focus:ring-2 focus:ring-primary-500 focus:border-transparent transition-all disabled:opacity-50 disabled:cursor-not-allowed"
                          placeholder="Enter email address"
                        />
                      </div>
                    </div>

                    {/* Password */}
                    <div>
                      <label htmlFor="password" className="block text-sm font-medium text-zinc-300 mb-2">
                        Password
                      </label>
                      <div className="relative">
                        <Lock className="absolute left-3 top-1/2 -translate-y-1/2 w-5 h-5 text-zinc-500" />
                        <input
                          type="password"
                          id="password"
                          required
                          value={newPassword}
                          onChange={(e) => setNewPassword(e.target.value)}
                          disabled={creating}
                          className="w-full pl-10 pr-4 py-3 bg-white/5 border border-white/10 rounded-lg text-white placeholder-zinc-500 focus:outline-none focus:ring-2 focus:ring-primary-500 focus:border-transparent transition-all disabled:opacity-50 disabled:cursor-not-allowed"
                          placeholder="Enter password (min 8 characters)"
                        />
                      </div>
                    </div>

                    {/* Access Permissions */}
                    <div>
                      <label className="block text-sm font-medium text-zinc-300 mb-3">
                        Access Permissions
                      </label>
                      <div className="space-y-3">
                        <label
                          htmlFor="console_login"
                          className="flex items-center gap-3 p-3 rounded-lg bg-white/5 border border-white/10 hover:bg-white/10 transition-all cursor-pointer"
                        >
                          <input
                            type="checkbox"
                            id="console_login"
                            checked={consoleLogin}
                            onChange={(e) => setConsoleLogin(e.target.checked)}
                            disabled={creating}
                            className="h-4 w-4 text-primary-500 bg-white/5 border-white/20 rounded focus:ring-2 focus:ring-primary-500 focus:ring-offset-0 disabled:opacity-50"
                          />
                          <div className="flex items-center gap-2 flex-1">
                            <Shield className="w-4 h-4 text-blue-400" />
                            <span className="text-sm text-white font-medium">Console Login</span>
                          </div>
                        </label>

                        <label
                          htmlFor="cli_access"
                          className="flex items-center gap-3 p-3 rounded-lg bg-white/5 border border-white/10 hover:bg-white/10 transition-all cursor-pointer"
                        >
                          <input
                            type="checkbox"
                            id="cli_access"
                            checked={cliAccess}
                            onChange={(e) => setCliAccess(e.target.checked)}
                            disabled={creating}
                            className="h-4 w-4 text-primary-500 bg-white/5 border-white/20 rounded focus:ring-2 focus:ring-primary-500 focus:ring-offset-0 disabled:opacity-50"
                          />
                          <div className="flex items-center gap-2 flex-1">
                            <Terminal className="w-4 h-4 text-green-400" />
                            <span className="text-sm text-white font-medium">CLI Access</span>
                          </div>
                        </label>

                        <label
                          htmlFor="api_access"
                          className="flex items-center gap-3 p-3 rounded-lg bg-white/5 border border-white/10 hover:bg-white/10 transition-all cursor-pointer"
                        >
                          <input
                            type="checkbox"
                            id="api_access"
                            checked={apiAccess}
                            onChange={(e) => setApiAccess(e.target.checked)}
                            disabled={creating}
                            className="h-4 w-4 text-primary-500 bg-white/5 border-white/20 rounded focus:ring-2 focus:ring-primary-500 focus:ring-offset-0 disabled:opacity-50"
                          />
                          <div className="flex items-center gap-2 flex-1">
                            <Code className="w-4 h-4 text-purple-400" />
                            <span className="text-sm text-white font-medium">API Access</span>
                          </div>
                        </label>

                        <label
                          htmlFor="pgwire_access"
                          className="flex items-center gap-3 p-3 rounded-lg bg-white/5 border border-white/10 hover:bg-white/10 transition-all cursor-pointer"
                        >
                          <input
                            type="checkbox"
                            id="pgwire_access"
                            checked={pgwireAccess}
                            onChange={(e) => setPgwireAccess(e.target.checked)}
                            disabled={creating}
                            className="h-4 w-4 text-primary-500 bg-white/5 border-white/20 rounded focus:ring-2 focus:ring-primary-500 focus:ring-offset-0 disabled:opacity-50"
                          />
                          <div className="flex items-center gap-2 flex-1">
                            <Database className="w-4 h-4 text-orange-400" />
                            <span className="text-sm text-white font-medium">PostgreSQL Access</span>
                          </div>
                        </label>

                        <label
                          htmlFor="can_impersonate"
                          className="flex items-center gap-3 p-3 rounded-lg bg-white/5 border border-white/10 hover:bg-white/10 transition-all cursor-pointer"
                        >
                          <input
                            type="checkbox"
                            id="can_impersonate"
                            checked={canImpersonate}
                            onChange={(e) => setCanImpersonate(e.target.checked)}
                            disabled={creating}
                            className="h-4 w-4 text-primary-500 bg-white/5 border-white/20 rounded focus:ring-2 focus:ring-primary-500 focus:ring-offset-0 disabled:opacity-50"
                          />
                          <div className="flex items-center gap-2 flex-1">
                            <Eye className="w-4 h-4 text-amber-400" />
                            <span className="text-sm text-white font-medium">Can Impersonate Users</span>
                          </div>
                        </label>
                      </div>
                    </div>
                  </div>
                </div>

                {/* Modal Actions */}
                <div className="flex gap-3 pt-6 border-t border-white/10 mt-6">
                  <button
                    type="button"
                    onClick={() => setShowCreateModal(false)}
                    disabled={creating}
                    className="flex-1 py-3 px-4 bg-white/5 hover:bg-white/10 border border-white/10 text-zinc-300 font-medium rounded-lg transition-all duration-200 disabled:opacity-50 disabled:cursor-not-allowed"
                  >
                    Cancel
                  </button>
                  <button
                    type="submit"
                    disabled={creating}
                    className="flex-1 py-3 px-4 bg-primary-500 hover:bg-primary-600 text-white font-medium rounded-lg transition-all duration-200 disabled:opacity-50 disabled:cursor-not-allowed hover:scale-[1.02] active:scale-[0.98] shadow-lg shadow-primary-500/20"
                  >
                    {creating ? (
                      <div className="flex items-center justify-center gap-2">
                        <div className="animate-spin rounded-full h-5 w-5 border-b-2 border-white"></div>
                        <span>Creating...</span>
                      </div>
                    ) : (
                      'Create User'
                    )}
                  </button>
                </div>
              </form>
            </div>
          </div>
        </div>
      )}

      {/* Edit User Modal */}
      {showEditModal && selectedUser && (
        <div className="fixed inset-0 z-50 overflow-y-auto">
          <div className="flex items-center justify-center min-h-screen px-4 pt-4 pb-20 text-center sm:p-0">
            {/* Backdrop */}
            <div
              className="fixed inset-0 bg-black/60 backdrop-blur-sm transition-opacity"
              onClick={() => !editing && setShowEditModal(false)}
            ></div>

            {/* Modal */}
            <div className="inline-block align-bottom glass-dark rounded-2xl border border-white/10 px-6 pt-6 pb-6 text-left overflow-hidden shadow-2xl transform transition-all sm:my-8 sm:align-middle sm:max-w-lg sm:w-full">
              <form onSubmit={handleUpdateUser}>
                <div>
                  <div className="flex items-center gap-3 mb-6">
                    <div className="p-2 rounded-lg bg-primary-500/10 border border-primary-500/20">
                      <Edit className="w-6 h-6 text-primary-400" />
                    </div>
                    <div className="flex-1">
                      <h3 className="text-2xl font-bold text-white">Edit Admin User</h3>
                      <p className="text-sm text-zinc-400 mt-1">Editing user: {selectedUser.username}</p>
                    </div>
                  </div>

                  {editError && (
                    <div className="mb-6 p-4 rounded-lg bg-red-500/10 border border-red-500/20 flex items-start gap-3">
                      <AlertCircle className="w-5 h-5 text-red-400 flex-shrink-0 mt-0.5" />
                      <p className="text-sm text-red-200">{editError}</p>
                    </div>
                  )}

                  <div className="space-y-5">
                    {/* Email */}
                    <div>
                      <label htmlFor="edit-email" className="block text-sm font-medium text-zinc-300 mb-2">
                        Email <span className="text-zinc-500">(optional)</span>
                      </label>
                      <div className="relative">
                        <Mail className="absolute left-3 top-1/2 -translate-y-1/2 w-5 h-5 text-zinc-500" />
                        <input
                          type="email"
                          id="edit-email"
                          value={editEmail}
                          onChange={(e) => setEditEmail(e.target.value)}
                          disabled={editing}
                          className="w-full pl-10 pr-4 py-3 bg-white/5 border border-white/10 rounded-lg text-white placeholder-zinc-500 focus:outline-none focus:ring-2 focus:ring-primary-500 focus:border-transparent transition-all disabled:opacity-50 disabled:cursor-not-allowed"
                          placeholder="Enter email address"
                        />
                      </div>
                    </div>

                    {/* Change Password Toggle */}
                    <div>
                      <label
                        htmlFor="change-password-toggle"
                        className="flex items-center gap-3 p-3 rounded-lg bg-white/5 border border-white/10 hover:bg-white/10 transition-all cursor-pointer"
                      >
                        <input
                          type="checkbox"
                          id="change-password-toggle"
                          checked={changePassword}
                          onChange={(e) => setChangePassword(e.target.checked)}
                          disabled={editing}
                          className="h-4 w-4 text-primary-500 bg-white/5 border-white/20 rounded focus:ring-2 focus:ring-primary-500 focus:ring-offset-0 disabled:opacity-50"
                        />
                        <div className="flex items-center gap-2 flex-1">
                          <Lock className="w-4 h-4 text-yellow-400" />
                          <span className="text-sm text-white font-medium">Change Password</span>
                        </div>
                      </label>
                    </div>

                    {/* Password (only shown if change password is checked) */}
                    {changePassword && (
                      <div>
                        <label htmlFor="edit-password" className="block text-sm font-medium text-zinc-300 mb-2">
                          New Password
                        </label>
                        <div className="relative">
                          <Lock className="absolute left-3 top-1/2 -translate-y-1/2 w-5 h-5 text-zinc-500" />
                          <input
                            type="password"
                            id="edit-password"
                            required={changePassword}
                            value={editPassword}
                            onChange={(e) => setEditPassword(e.target.value)}
                            disabled={editing}
                            className="w-full pl-10 pr-4 py-3 bg-white/5 border border-white/10 rounded-lg text-white placeholder-zinc-500 focus:outline-none focus:ring-2 focus:ring-primary-500 focus:border-transparent transition-all disabled:opacity-50 disabled:cursor-not-allowed"
                            placeholder="Enter new password (min 8 characters)"
                          />
                        </div>
                      </div>
                    )}

                    {/* Access Permissions */}
                    <div>
                      <label className="block text-sm font-medium text-zinc-300 mb-3">
                        Access Permissions
                      </label>
                      <div className="space-y-3">
                        <label
                          htmlFor="edit-console-login"
                          className="flex items-center gap-3 p-3 rounded-lg bg-white/5 border border-white/10 hover:bg-white/10 transition-all cursor-pointer"
                        >
                          <input
                            type="checkbox"
                            id="edit-console-login"
                            checked={editConsoleLogin}
                            onChange={(e) => setEditConsoleLogin(e.target.checked)}
                            disabled={editing}
                            className="h-4 w-4 text-primary-500 bg-white/5 border-white/20 rounded focus:ring-2 focus:ring-primary-500 focus:ring-offset-0 disabled:opacity-50"
                          />
                          <div className="flex items-center gap-2 flex-1">
                            <Shield className="w-4 h-4 text-blue-400" />
                            <span className="text-sm text-white font-medium">Console Login</span>
                          </div>
                        </label>

                        <label
                          htmlFor="edit-cli-access"
                          className="flex items-center gap-3 p-3 rounded-lg bg-white/5 border border-white/10 hover:bg-white/10 transition-all cursor-pointer"
                        >
                          <input
                            type="checkbox"
                            id="edit-cli-access"
                            checked={editCliAccess}
                            onChange={(e) => setEditCliAccess(e.target.checked)}
                            disabled={editing}
                            className="h-4 w-4 text-primary-500 bg-white/5 border-white/20 rounded focus:ring-2 focus:ring-primary-500 focus:ring-offset-0 disabled:opacity-50"
                          />
                          <div className="flex items-center gap-2 flex-1">
                            <Terminal className="w-4 h-4 text-green-400" />
                            <span className="text-sm text-white font-medium">CLI Access</span>
                          </div>
                        </label>

                        <label
                          htmlFor="edit-api-access"
                          className="flex items-center gap-3 p-3 rounded-lg bg-white/5 border border-white/10 hover:bg-white/10 transition-all cursor-pointer"
                        >
                          <input
                            type="checkbox"
                            id="edit-api-access"
                            checked={editApiAccess}
                            onChange={(e) => setEditApiAccess(e.target.checked)}
                            disabled={editing}
                            className="h-4 w-4 text-primary-500 bg-white/5 border-white/20 rounded focus:ring-2 focus:ring-primary-500 focus:ring-offset-0 disabled:opacity-50"
                          />
                          <div className="flex items-center gap-2 flex-1">
                            <Code className="w-4 h-4 text-purple-400" />
                            <span className="text-sm text-white font-medium">API Access</span>
                          </div>
                        </label>

                        <label
                          htmlFor="edit-pgwire-access"
                          className="flex items-center gap-3 p-3 rounded-lg bg-white/5 border border-white/10 hover:bg-white/10 transition-all cursor-pointer"
                        >
                          <input
                            type="checkbox"
                            id="edit-pgwire-access"
                            checked={editPgwireAccess}
                            onChange={(e) => setEditPgwireAccess(e.target.checked)}
                            disabled={editing}
                            className="h-4 w-4 text-primary-500 bg-white/5 border-white/20 rounded focus:ring-2 focus:ring-primary-500 focus:ring-offset-0 disabled:opacity-50"
                          />
                          <div className="flex items-center gap-2 flex-1">
                            <Database className="w-4 h-4 text-orange-400" />
                            <span className="text-sm text-white font-medium">PostgreSQL Access</span>
                          </div>
                        </label>

                        <label
                          htmlFor="edit-can-impersonate"
                          className="flex items-center gap-3 p-3 rounded-lg bg-white/5 border border-white/10 hover:bg-white/10 transition-all cursor-pointer"
                        >
                          <input
                            type="checkbox"
                            id="edit-can-impersonate"
                            checked={editCanImpersonate}
                            onChange={(e) => setEditCanImpersonate(e.target.checked)}
                            disabled={editing}
                            className="h-4 w-4 text-primary-500 bg-white/5 border-white/20 rounded focus:ring-2 focus:ring-primary-500 focus:ring-offset-0 disabled:opacity-50"
                          />
                          <div className="flex items-center gap-2 flex-1">
                            <Eye className="w-4 h-4 text-amber-400" />
                            <span className="text-sm text-white font-medium">Can Impersonate Users</span>
                          </div>
                        </label>
                      </div>
                    </div>
                  </div>
                </div>

                {/* Modal Actions */}
                <div className="flex gap-3 pt-6 border-t border-white/10 mt-6">
                  <button
                    type="button"
                    onClick={() => setShowEditModal(false)}
                    disabled={editing}
                    className="flex-1 py-3 px-4 bg-white/5 hover:bg-white/10 border border-white/10 text-zinc-300 font-medium rounded-lg transition-all duration-200 disabled:opacity-50 disabled:cursor-not-allowed"
                  >
                    Cancel
                  </button>
                  <button
                    type="submit"
                    disabled={editing}
                    className="flex-1 py-3 px-4 bg-primary-500 hover:bg-primary-600 text-white font-medium rounded-lg transition-all duration-200 disabled:opacity-50 disabled:cursor-not-allowed hover:scale-[1.02] active:scale-[0.98] shadow-lg shadow-primary-500/20"
                  >
                    {editing ? (
                      <div className="flex items-center justify-center gap-2">
                        <div className="animate-spin rounded-full h-5 w-5 border-b-2 border-white"></div>
                        <span>Updating...</span>
                      </div>
                    ) : (
                      'Update User'
                    )}
                  </button>
                </div>
              </form>
            </div>
          </div>
        </div>
      )}
      <ConfirmDialog
        open={deleteConfirm !== null}
        title="Confirm Deletion"
        message={deleteConfirm?.message || ''}
        variant="danger"
        confirmText="Delete"
        onConfirm={() => {
          deleteConfirm?.onConfirm()
          setDeleteConfirm(null)
        }}
        onCancel={() => setDeleteConfirm(null)}
      />
      <ToastContainer toasts={toasts} onClose={closeToast} />
    </div>
  )
}
