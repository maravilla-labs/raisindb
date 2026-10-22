import { useState } from 'react'
import { X } from 'lucide-react'
import { identityUsersApi, IdentityUser } from '../api/identity-users'
import { ApiError } from '../api/client'

/**
 * Operator password reset for an identity user. Also clears any lockout, since
 * the server replaces the credentials record.
 */
export default function SetPasswordModal({
  tenantId,
  user,
  onDone,
  onClose,
}: {
  tenantId: string
  user: IdentityUser
  onDone: () => void
  onClose: () => void
}) {
  const [password, setPassword] = useState('')
  const [mustChange, setMustChange] = useState(true)
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const submit = async (e: React.FormEvent) => {
    e.preventDefault()
    setSaving(true)
    setError(null)
    try {
      await identityUsersApi.update(tenantId, user.id, {
        password,
        must_change_password: mustChange,
      })
      onDone()
      onClose()
    } catch (err) {
      setError(err instanceof ApiError ? err.message : 'Failed to set password')
    } finally {
      setSaving(false)
    }
  }

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4">
      <form
        onSubmit={submit}
        className="w-full max-w-md rounded-xl border border-zinc-800 bg-zinc-900 p-6 space-y-4"
      >
        <div className="flex items-start justify-between">
          <div>
            <h2 className="text-lg font-semibold text-white">Set password</h2>
            <p className="text-sm text-zinc-400">{user.email}</p>
          </div>
          <button type="button" onClick={onClose} className="text-zinc-500 hover:text-white">
            <X className="w-5 h-5" />
          </button>
        </div>

        <div>
          <label htmlFor="new-password" className="block text-sm font-medium text-zinc-300 mb-2">
            New password
          </label>
          <input
            id="new-password"
            type="password"
            autoComplete="new-password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            required
            className="w-full rounded-lg bg-zinc-800 border border-zinc-700 px-3 py-2 text-white"
          />
        </div>

        <label className="flex items-center gap-2 text-sm text-zinc-300">
          <input
            type="checkbox"
            checked={mustChange}
            onChange={(e) => setMustChange(e.target.checked)}
          />
          Require the user to choose their own on next login
        </label>

        {error && <p className="text-sm text-red-400">{error}</p>}

        <div className="flex justify-end gap-2">
          <button
            type="button"
            onClick={onClose}
            className="px-4 py-2 rounded-lg text-zinc-300 hover:bg-zinc-800"
          >
            Cancel
          </button>
          <button
            type="submit"
            disabled={saving || !password}
            className="px-4 py-2 rounded-lg bg-orange-600 text-white hover:bg-orange-500 disabled:opacity-50"
          >
            {saving ? 'Saving…' : 'Set password'}
          </button>
        </div>
      </form>
    </div>
  )
}
