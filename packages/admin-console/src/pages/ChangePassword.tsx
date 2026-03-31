import { useState, FormEvent } from 'react'
import { useNavigate } from 'react-router-dom'
import { useAuth } from '../contexts/AuthContext'
import { ApiError } from '../api/client'
import { Lock, AlertCircle, CheckCircle, AlertTriangle } from 'lucide-react'
import logoImage from '../assets/raisin-logo.png'

export default function ChangePassword() {
  const [oldPassword, setOldPassword] = useState('')
  const [newPassword, setNewPassword] = useState('')
  const [confirmPassword, setConfirmPassword] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [success, setSuccess] = useState(false)
  const [isLoading, setIsLoading] = useState(false)

  const { changePassword, user, logout } = useAuth()
  const navigate = useNavigate()

  const handleSubmit = async (e: FormEvent) => {
    e.preventDefault()
    setError(null)
    setSuccess(false)

    // Validate passwords match
    if (newPassword !== confirmPassword) {
      setError('New passwords do not match')
      return
    }

    // Validate password strength
    if (newPassword.length < 8) {
      setError('New password must be at least 8 characters long')
      return
    }

    setIsLoading(true)

    try {
      await changePassword(oldPassword, newPassword)
      setSuccess(true)

      // If this was a forced password change, redirect to home after a delay
      if (user?.must_change_password) {
        setTimeout(() => {
          navigate('/')
        }, 2000)
      } else {
        // Clear form
        setOldPassword('')
        setNewPassword('')
        setConfirmPassword('')
      }
    } catch (err) {
      if (err instanceof ApiError) {
        setError(err.message)
      } else {
        setError('Password change failed. Please try again.')
      }
    } finally {
      setIsLoading(false)
    }
  }

  const handleCancel = () => {
    if (user?.must_change_password) {
      // If password change is required, logout instead of canceling
      logout()
      navigate('/login')
    } else {
      navigate(-1)
    }
  }

  return (
    <div className="min-h-screen flex items-center justify-center bg-gradient-to-br from-zinc-950 via-primary-950/30 to-black p-4">
      <div className="w-full max-w-md">
        {/* Logo and Title */}
        <div className="text-center mb-8 animate-fade-in">
          <div className="flex justify-center mb-4">
            <img src={logoImage} alt="RaisinDB Logo" className="w-20 h-20" />
          </div>
          <h1 className="text-3xl font-bold text-white mb-2">Change Password</h1>
          <p className="text-zinc-400">Update your security credentials</p>
        </div>

        {/* Change Password Form */}
        <div className="glass-dark p-8 rounded-2xl border border-white/10 shadow-2xl animate-fade-in">
          {/* Warning for forced password change */}
          {user?.must_change_password && (
            <div className="mb-6 p-4 rounded-lg bg-yellow-500/10 border border-yellow-500/20 flex items-start gap-3">
              <AlertTriangle className="w-5 h-5 text-yellow-400 flex-shrink-0 mt-0.5" />
              <div>
                <h3 className="text-sm font-semibold text-yellow-200 mb-1">Password Change Required</h3>
                <p className="text-sm text-yellow-300/80">You must change your password before continuing</p>
              </div>
            </div>
          )}

          {error && (
            <div className="mb-6 p-4 rounded-lg bg-red-500/10 border border-red-500/20 flex items-start gap-3 animate-shake">
              <AlertCircle className="w-5 h-5 text-red-400 flex-shrink-0 mt-0.5" />
              <p className="text-sm text-red-200">{error}</p>
            </div>
          )}

          {success && (
            <div className="mb-6 p-4 rounded-lg bg-green-500/10 border border-green-500/20 flex items-start gap-3">
              <CheckCircle className="w-5 h-5 text-green-400 flex-shrink-0 mt-0.5" />
              <p className="text-sm text-green-200">Password changed successfully!</p>
            </div>
          )}

          <form onSubmit={handleSubmit} className="space-y-5">
            {/* Current Password */}
            <div>
              <label htmlFor="old-password" className="block text-sm font-medium text-zinc-300 mb-2">
                Current Password
              </label>
              <div className="relative">
                <Lock className="absolute left-3 top-1/2 -translate-y-1/2 w-5 h-5 text-zinc-500" />
                <input
                  id="old-password"
                  name="old-password"
                  type="password"
                  autoComplete="current-password"
                  required
                  value={oldPassword}
                  onChange={(e) => setOldPassword(e.target.value)}
                  disabled={isLoading}
                  className="w-full pl-10 pr-4 py-3 bg-white/5 border border-white/10 rounded-lg text-white placeholder-zinc-500 focus:outline-none focus:ring-2 focus:ring-primary-500 focus:border-transparent transition-all disabled:opacity-50 disabled:cursor-not-allowed"
                  placeholder="Enter current password"
                />
              </div>
            </div>

            {/* New Password */}
            <div>
              <label htmlFor="new-password" className="block text-sm font-medium text-zinc-300 mb-2">
                New Password
              </label>
              <div className="relative">
                <Lock className="absolute left-3 top-1/2 -translate-y-1/2 w-5 h-5 text-zinc-500" />
                <input
                  id="new-password"
                  name="new-password"
                  type="password"
                  autoComplete="new-password"
                  required
                  value={newPassword}
                  onChange={(e) => setNewPassword(e.target.value)}
                  disabled={isLoading}
                  className="w-full pl-10 pr-4 py-3 bg-white/5 border border-white/10 rounded-lg text-white placeholder-zinc-500 focus:outline-none focus:ring-2 focus:ring-primary-500 focus:border-transparent transition-all disabled:opacity-50 disabled:cursor-not-allowed"
                  placeholder="Enter new password (min 8 characters)"
                />
              </div>
            </div>

            {/* Confirm Password */}
            <div>
              <label htmlFor="confirm-password" className="block text-sm font-medium text-zinc-300 mb-2">
                Confirm New Password
              </label>
              <div className="relative">
                <Lock className="absolute left-3 top-1/2 -translate-y-1/2 w-5 h-5 text-zinc-500" />
                <input
                  id="confirm-password"
                  name="confirm-password"
                  type="password"
                  autoComplete="new-password"
                  required
                  value={confirmPassword}
                  onChange={(e) => setConfirmPassword(e.target.value)}
                  disabled={isLoading}
                  className="w-full pl-10 pr-4 py-3 bg-white/5 border border-white/10 rounded-lg text-white placeholder-zinc-500 focus:outline-none focus:ring-2 focus:ring-primary-500 focus:border-transparent transition-all disabled:opacity-50 disabled:cursor-not-allowed"
                  placeholder="Confirm new password"
                />
              </div>
            </div>

            {/* Buttons */}
            <div className="flex gap-3 pt-2">
              <button
                type="button"
                onClick={handleCancel}
                disabled={isLoading}
                className="flex-1 py-3 px-4 bg-white/5 hover:bg-white/10 border border-white/10 text-zinc-300 font-medium rounded-lg transition-all duration-200 disabled:opacity-50 disabled:cursor-not-allowed"
              >
                {user?.must_change_password ? 'Logout' : 'Cancel'}
              </button>
              <button
                type="submit"
                disabled={isLoading}
                className="flex-1 py-3 px-4 bg-primary-500 hover:bg-primary-600 text-white font-medium rounded-lg transition-all duration-200 disabled:opacity-50 disabled:cursor-not-allowed hover:scale-[1.02] active:scale-[0.98] shadow-lg shadow-primary-500/20"
              >
                {isLoading ? (
                  <div className="flex items-center justify-center gap-2">
                    <div className="animate-spin rounded-full h-5 w-5 border-b-2 border-white"></div>
                    <span>Changing...</span>
                  </div>
                ) : (
                  'Change Password'
                )}
              </button>
            </div>
          </form>
        </div>

        {/* Footer */}
        <div className="text-center mt-6 text-sm text-zinc-500">
          RaisinDB v0.1.0 • {new Date().getFullYear()}
        </div>
      </div>
    </div>
  )
}
