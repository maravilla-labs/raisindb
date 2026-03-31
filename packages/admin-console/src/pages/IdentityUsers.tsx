import { useState, useEffect } from 'react'
import { useAuth } from '../contexts/AuthContext'
import { identityUsersApi, IdentityUser } from '../api/identity-users'
import { ApiError } from '../api/client'
import {
  Users,
  Mail,
  AlertCircle,
  CheckCircle,
  XCircle,
  Trash2,
  Search,
  Shield,
  Clock,
  ExternalLink,
  UserX,
  UserCheck,
} from 'lucide-react'
import ConfirmDialog from '../components/ConfirmDialog'
import { useToast, ToastContainer } from '../components/Toast'

export default function IdentityUsers() {
  const { tenantId } = useAuth()
  const [users, setUsers] = useState<IdentityUser[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [searchEmail, setSearchEmail] = useState('')
  const [deleteConfirm, setDeleteConfirm] = useState<{
    message: string
    onConfirm: () => void
  } | null>(null)
  const { toasts, error: showError, success: showSuccess, closeToast } = useToast()

  const loadUsers = async () => {
    try {
      setLoading(true)
      const params = searchEmail ? { email: searchEmail } : undefined
      const data = await identityUsersApi.list(tenantId, params)
      setUsers(data)
      setError(null)
    } catch (err) {
      if (err instanceof ApiError) {
        setError(err.message)
      } else {
        setError('Failed to load identity users')
      }
    } finally {
      setLoading(false)
    }
  }

  useEffect(() => {
    loadUsers()
  }, [tenantId])

  const handleSearch = (e: React.FormEvent) => {
    e.preventDefault()
    loadUsers()
  }

  const handleToggleActive = async (user: IdentityUser) => {
    try {
      await identityUsersApi.update(tenantId, user.id, {
        is_active: !user.is_active,
      })
      showSuccess('Success', `User ${user.is_active ? 'deactivated' : 'activated'}`)
      loadUsers()
    } catch (err) {
      if (err instanceof ApiError) {
        showError('Error', `Failed to update user: ${err.message}`)
      } else {
        showError('Error', 'Failed to update user')
      }
    }
  }

  const handleDeleteUser = async (user: IdentityUser) => {
    setDeleteConfirm({
      message: `Are you sure you want to delete user "${user.email}"? This will also delete all their sessions.`,
      onConfirm: async () => {
        try {
          await identityUsersApi.delete(tenantId, user.id)
          showSuccess('Success', 'User deleted')
          loadUsers()
        } catch (err) {
          if (err instanceof ApiError) {
            showError('Error', `Failed to delete user: ${err.message}`)
          } else {
            showError('Error', 'Failed to delete user')
          }
        }
      },
    })
  }

  if (loading) {
    return (
      <div className="flex items-center justify-center min-h-[60vh]">
        <div className="flex flex-col items-center gap-4">
          <div className="animate-spin rounded-full h-12 w-12 border-b-2 border-primary-500"></div>
          <p className="text-zinc-400 text-sm">Loading identity users...</p>
        </div>
      </div>
    )
  }

  return (
    <div className="max-w-7xl mx-auto">
      {/* Header */}
      <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4 mb-8">
        <div>
          <h1 className="text-3xl font-bold text-white mb-2">Identity Users</h1>
          <p className="text-zinc-400">
            Manage users who registered through the authentication system
          </p>
        </div>
      </div>

      {/* Search */}
      <form onSubmit={handleSearch} className="mb-6">
        <div className="flex gap-3">
          <div className="relative flex-1 max-w-md">
            <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-5 h-5 text-zinc-500" />
            <input
              type="text"
              value={searchEmail}
              onChange={(e) => setSearchEmail(e.target.value)}
              placeholder="Search by email..."
              className="w-full pl-10 pr-4 py-2.5 bg-white/5 border border-white/10 rounded-lg text-white placeholder-zinc-500 focus:outline-none focus:ring-2 focus:ring-primary-500 focus:border-transparent transition-all"
            />
          </div>
          <button
            type="submit"
            className="px-4 py-2.5 bg-primary-500 hover:bg-primary-600 text-white font-medium rounded-lg transition-all"
          >
            Search
          </button>
        </div>
      </form>

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
                  Email
                </th>
                <th className="px-3 py-4 text-left text-sm font-semibold text-zinc-300">
                  Display Name
                </th>
                <th className="px-3 py-4 text-left text-sm font-semibold text-zinc-300">
                  Providers
                </th>
                <th className="px-3 py-4 text-left text-sm font-semibold text-zinc-300">
                  Status
                </th>
                <th className="px-3 py-4 text-left text-sm font-semibold text-zinc-300">
                  Last Login
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
              {users.length === 0 ? (
                <tr>
                  <td colSpan={7} className="py-12 text-center">
                    <Users className="w-12 h-12 text-zinc-600 mx-auto mb-3" />
                    <p className="text-zinc-400">No identity users found</p>
                    <p className="text-zinc-500 text-sm mt-1">
                      Users will appear here when they register
                    </p>
                  </td>
                </tr>
              ) : (
                users.map((user) => (
                  <tr
                    key={user.id}
                    className="hover:bg-white/5 transition-colors duration-150"
                  >
                    <td className="whitespace-nowrap py-4 pl-6 pr-3 text-sm">
                      <div className="flex items-center gap-2">
                        <Mail className="w-4 h-4 text-zinc-500" />
                        <span className="font-medium text-white">{user.email}</span>
                        {user.email_verified && (
                          <span className="inline-flex items-center gap-1 rounded-full bg-green-500/10 border border-green-500/20 px-2 py-0.5 text-xs font-medium text-green-300">
                            <CheckCircle className="w-3 h-3" />
                            Verified
                          </span>
                        )}
                      </div>
                    </td>
                    <td className="whitespace-nowrap px-3 py-4 text-sm text-zinc-400">
                      {user.display_name || <span className="text-zinc-600">-</span>}
                    </td>
                    <td className="px-3 py-4 text-sm">
                      <div className="flex flex-wrap gap-1.5">
                        {user.linked_providers.length === 0 ? (
                          <span className="inline-flex items-center gap-1 rounded-md bg-zinc-500/10 border border-zinc-500/20 px-2 py-1 text-xs font-medium text-zinc-400">
                            <Shield className="w-3 h-3" />
                            Local
                          </span>
                        ) : (
                          user.linked_providers.map((provider) => (
                            <span
                              key={provider}
                              className="inline-flex items-center gap-1 rounded-md bg-blue-500/10 border border-blue-500/20 px-2 py-1 text-xs font-medium text-blue-300"
                            >
                              <ExternalLink className="w-3 h-3" />
                              {provider}
                            </span>
                          ))
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
                      {user.last_login_at ? (
                        <div className="flex items-center gap-1.5">
                          <Clock className="w-3.5 h-3.5 text-zinc-500" />
                          {new Date(user.last_login_at).toLocaleDateString()}
                        </div>
                      ) : (
                        <span className="text-zinc-600">Never</span>
                      )}
                    </td>
                    <td className="whitespace-nowrap px-3 py-4 text-sm text-zinc-400">
                      {new Date(user.created_at).toLocaleDateString()}
                    </td>
                    <td className="relative whitespace-nowrap py-4 pl-3 pr-6 text-right text-sm">
                      <div className="flex items-center justify-end gap-2">
                        <button
                          onClick={() => handleToggleActive(user)}
                          className={`inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg transition-all duration-200 border border-transparent ${
                            user.is_active
                              ? 'text-amber-400 hover:text-amber-300 hover:bg-amber-500/10 hover:border-amber-500/20'
                              : 'text-green-400 hover:text-green-300 hover:bg-green-500/10 hover:border-green-500/20'
                          }`}
                        >
                          {user.is_active ? (
                            <>
                              <UserX className="w-4 h-4" />
                              Deactivate
                            </>
                          ) : (
                            <>
                              <UserCheck className="w-4 h-4" />
                              Activate
                            </>
                          )}
                        </button>
                        <button
                          onClick={() => handleDeleteUser(user)}
                          className="inline-flex items-center gap-1.5 px-3 py-1.5 text-red-400 hover:text-red-300 hover:bg-red-500/10 border border-transparent hover:border-red-500/20 rounded-lg transition-all duration-200"
                        >
                          <Trash2 className="w-4 h-4" />
                          Delete
                        </button>
                      </div>
                    </td>
                  </tr>
                ))
              )}
            </tbody>
          </table>
        </div>
      </div>

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
