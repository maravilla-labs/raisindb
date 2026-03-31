// SPDX-License-Identifier: BSL-1.1

import { createContext, useContext, useState, useEffect, ReactNode } from 'react'
import { authApi, LoginRequest, LoginResponse } from '../api/auth'

interface AuthContextType {
  user: LoginResponse | null
  token: string | null
  tenantId: string
  isAuthenticated: boolean
  isLoading: boolean
  login: (username: string, password: string) => Promise<void>
  logout: () => void
  changePassword: (oldPassword: string, newPassword: string) => Promise<void>
}

const AuthContext = createContext<AuthContextType | undefined>(undefined)

const TOKEN_STORAGE_KEY = 'raisindb_auth_token'
const USER_STORAGE_KEY = 'raisindb_auth_user'
const TENANT_STORAGE_KEY = 'raisindb_tenant_id'

interface AuthProviderProps {
  children: ReactNode
  defaultTenantId?: string
}

export function AuthProvider({ children, defaultTenantId = 'default' }: AuthProviderProps) {
  const [user, setUser] = useState<LoginResponse | null>(null)
  const [token, setToken] = useState<string | null>(null)
  const [tenantId] = useState<string>(
    localStorage.getItem(TENANT_STORAGE_KEY) || defaultTenantId
  )
  const [isLoading, setIsLoading] = useState(true)

  // Initialize auth state from localStorage
  useEffect(() => {
    const storedToken = localStorage.getItem(TOKEN_STORAGE_KEY)
    const storedUser = localStorage.getItem(USER_STORAGE_KEY)

    if (storedToken && storedUser) {
      try {
        const parsedUser = JSON.parse(storedUser) as LoginResponse

        // Check if token is expired
        const now = Math.floor(Date.now() / 1000)
        if (parsedUser.expires_at && parsedUser.expires_at > now) {
          setToken(storedToken)
          setUser(parsedUser)
        } else {
          // Token expired, clear storage
          localStorage.removeItem(TOKEN_STORAGE_KEY)
          localStorage.removeItem(USER_STORAGE_KEY)
        }
      } catch (error) {
        console.error('Failed to parse stored user data:', error)
        localStorage.removeItem(TOKEN_STORAGE_KEY)
        localStorage.removeItem(USER_STORAGE_KEY)
      }
    }

    setIsLoading(false)
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
    localStorage.setItem(TENANT_STORAGE_KEY, tenantId)

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
