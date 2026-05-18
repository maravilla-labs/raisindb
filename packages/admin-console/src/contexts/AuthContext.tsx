// SPDX-License-Identifier: BSL-1.1

import { createContext, useContext, useState, useEffect, ReactNode } from 'react'
import { authApi, LoginRequest, LoginResponse } from '../api/auth'
import {
  fetchBootstrap,
  getCurrentTenantId,
  getCurrentServerVersion,
  getCurrentDevMode,
} from '../api/bootstrap'

interface AuthContextType {
  user: LoginResponse | null
  token: string | null
  /**
   * Read-only resolved tenant for this session.
   *
   * Sourced from `/api/admin/bootstrap` on boot — which itself comes from the
   * server's `x-tenant-id` middleware (proxy-injected for multi-tenant,
   * defaulted to "default" for single-operator dev). The console deliberately
   * does NOT expose a setter — cross-tenant operations are operator-only and
   * live under `/management/admin/*`, not on the customer-facing admin SPA.
   */
  tenantId: string
  /** raisindb server version, e.g. "0.1.19". */
  serverVersion: string
  /** True when the server reports it's running in dev / single-operator mode. */
  devMode: boolean
  isAuthenticated: boolean
  isLoading: boolean
  login: (username: string, password: string) => Promise<void>
  logout: () => void
  changePassword: (oldPassword: string, newPassword: string) => Promise<void>
}

const AuthContext = createContext<AuthContextType | undefined>(undefined)

const TOKEN_STORAGE_KEY = 'raisindb_auth_token'
const USER_STORAGE_KEY = 'raisindb_auth_user'

interface AuthProviderProps {
  children: ReactNode
}

export function AuthProvider({ children }: AuthProviderProps) {
  const [user, setUser] = useState<LoginResponse | null>(null)
  const [token, setToken] = useState<string | null>(null)
  // Seed from the bootstrap module's cache (localStorage-backed) so that
  // the first paint already shows something sensible; the async bootstrap
  // fetch below overwrites with the server-authoritative value.
  const [tenantId, setTenantIdState] = useState<string>(getCurrentTenantId())
  const [serverVersion, setServerVersion] = useState<string>(getCurrentServerVersion())
  const [devMode, setDevMode] = useState<boolean>(getCurrentDevMode())
  const [isLoading, setIsLoading] = useState(true)

  // Boot: fetch /api/admin/bootstrap and rehydrate auth state from storage.
  useEffect(() => {
    let cancelled = false

    const boot = async () => {
      // 1. Resolve tenant + version from the server. Failures fall back to
      //    the localStorage-seeded cache so disconnected dev still works.
      try {
        const data = await fetchBootstrap()
        if (cancelled) return
        const resolvedTenant = data.tenant_id || 'default'
        const resolvedVersion = data.version || serverVersion
        const resolvedDevMode = data.dev_mode === true
        setTenantIdState((prev) => (prev === resolvedTenant ? prev : resolvedTenant))
        setServerVersion((prev) => (prev === resolvedVersion ? prev : resolvedVersion))
        setDevMode((prev) => (prev === resolvedDevMode ? prev : resolvedDevMode))
      } catch (err) {
        // Non-fatal: keep cached values, log for diagnostics.
        console.warn('Bootstrap fetch failed; using cached tenant/version', err)
      }

      // 2. Rehydrate auth state from localStorage.
      const storedToken = localStorage.getItem(TOKEN_STORAGE_KEY)
      const storedUser = localStorage.getItem(USER_STORAGE_KEY)

      if (storedToken && storedUser) {
        try {
          const parsedUser = JSON.parse(storedUser) as LoginResponse
          const now = Math.floor(Date.now() / 1000)
          if (parsedUser.expires_at && parsedUser.expires_at > now) {
            if (!cancelled) {
              setToken(storedToken)
              setUser(parsedUser)
            }
          } else {
            localStorage.removeItem(TOKEN_STORAGE_KEY)
            localStorage.removeItem(USER_STORAGE_KEY)
          }
        } catch (error) {
          console.error('Failed to parse stored user data:', error)
          localStorage.removeItem(TOKEN_STORAGE_KEY)
          localStorage.removeItem(USER_STORAGE_KEY)
        }
      }

      if (!cancelled) setIsLoading(false)
    }

    boot()

    return () => {
      cancelled = true
    }
    // Intentionally empty deps — boot runs once on mount.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  const login = async (username: string, password: string) => {
    const request: LoginRequest = {
      username,
      password,
      interface: 'console'
    }

    const response = await authApi.login(tenantId, request)

    // Store auth data
    localStorage.setItem(TOKEN_STORAGE_KEY, response.token)
    localStorage.setItem(USER_STORAGE_KEY, JSON.stringify(response))

    setToken(response.token)
    setUser(response)
  }

  const logout = () => {
    localStorage.removeItem(TOKEN_STORAGE_KEY)
    localStorage.removeItem(USER_STORAGE_KEY)
    setToken(null)
    setUser(null)
  }

  const changePassword = async (oldPassword: string, newPassword: string) => {
    if (!token) {
      throw new Error('Not authenticated')
    }

    await authApi.changePassword(
      tenantId,
      { old_password: oldPassword, new_password: newPassword },
      token
    )

    // If password change was successful and user had must_change_password flag,
    // update the user state
    if (user?.must_change_password) {
      const updatedUser = { ...user, must_change_password: false }
      setUser(updatedUser)
      localStorage.setItem(USER_STORAGE_KEY, JSON.stringify(updatedUser))
    }
  }

  const value: AuthContextType = {
    user,
    token,
    tenantId,
    serverVersion,
    devMode,
    isAuthenticated: !!token && !!user,
    isLoading,
    login,
    logout,
    changePassword
  }

  return <AuthContext.Provider value={value}>{children}</AuthContext.Provider>
}

export function useAuth(): AuthContextType {
  const context = useContext(AuthContext)
  if (context === undefined) {
    throw new Error('useAuth must be used within an AuthProvider')
  }
  return context
}
