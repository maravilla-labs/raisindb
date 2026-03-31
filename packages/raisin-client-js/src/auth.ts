/**
 * JWT authentication manager
 */

import {
  AuthenticatePayload,
  AuthenticateResponse,
  RefreshTokenPayload,
} from './protocol';

/**
 * Token storage interface
 */
export interface TokenStorage {
  getAccessToken(): string | null;
  setAccessToken(token: string): void;
  getRefreshToken(): string | null;
  setRefreshToken(token: string): void;
  clear(): void;
}

/**
 * In-memory token storage (default)
 */
export class MemoryTokenStorage implements TokenStorage {
  private accessToken: string | null = null;
  private refreshToken: string | null = null;

  getAccessToken(): string | null {
    return this.accessToken;
  }

  setAccessToken(token: string): void {
    this.accessToken = token;
  }

  getRefreshToken(): string | null {
    return this.refreshToken;
  }

  setRefreshToken(token: string): void {
    this.refreshToken = token;
  }

  clear(): void {
    this.accessToken = null;
    this.refreshToken = null;
  }
}

/**
 * LocalStorage-based token storage for browser
 */
export class LocalStorageTokenStorage implements TokenStorage {
  private prefix: string;

  constructor(prefix: string = 'raisindb') {
    this.prefix = prefix;
  }

  getAccessToken(): string | null {
    if (typeof localStorage === 'undefined') return null;
    return localStorage.getItem(`${this.prefix}_access_token`);
  }

  setAccessToken(token: string): void {
    if (typeof localStorage === 'undefined') return;
    localStorage.setItem(`${this.prefix}_access_token`, token);
  }

  getRefreshToken(): string | null {
    if (typeof localStorage === 'undefined') return null;
    return localStorage.getItem(`${this.prefix}_refresh_token`);
  }

  setRefreshToken(token: string): void {
    if (typeof localStorage === 'undefined') return;
    localStorage.setItem(`${this.prefix}_refresh_token`, token);
  }

  clear(): void {
    if (typeof localStorage === 'undefined') return;
    localStorage.removeItem(`${this.prefix}_access_token`);
    localStorage.removeItem(`${this.prefix}_refresh_token`);
    localStorage.removeItem(`${this.prefix}_user`);
  }

  /**
   * Store user data in localStorage
   */
  setUser(user: IdentityUser): void {
    if (typeof localStorage === 'undefined') return;
    localStorage.setItem(`${this.prefix}_user`, JSON.stringify(user));
  }

  /**
   * Get stored user data from localStorage
   */
  getUser(): IdentityUser | null {
    if (typeof localStorage === 'undefined') return null;
    const userJson = localStorage.getItem(`${this.prefix}_user`);
    if (!userJson) return null;
    try {
      return JSON.parse(userJson);
    } catch {
      return null;
    }
  }
}

/**
 * Identity user information (from JWT/login response)
 */
export interface IdentityUser {
  /** Identity UUID */
  id: string;
  /** Email address */
  email: string;
  /** Display name */
  displayName: string | null;
  /** Avatar URL */
  avatarUrl: string | null;
  /** Whether email is verified */
  emailVerified: boolean;
  /** User's home node path in the repository (from JWT or RAISIN_CURRENT_USER) */
  home: string | null;
}

/**
 * Identity auth tokens response (from HTTP login/register)
 */
export interface IdentityAuthResponse {
  access_token: string;
  refresh_token: string;
  token_type: string;
  expires_at: number;
  identity: {
    id: string;
    email: string;
    display_name: string | null;
    avatar_url: string | null;
    email_verified: boolean;
    linked_providers: string[];
    /** User's home node path in the repository */
    home: string | null;
  };
}

/**
 * Identity auth error
 */
export interface IdentityAuthError {
  code: string;
  message: string;
}

/**
 * Admin authentication credentials (username/password)
 */
export interface AdminCredentials {
  username: string;
  password: string;
}

/**
 * JWT authentication credentials (identity users)
 * Used by SPAs that have obtained a JWT via HTTP API
 */
export interface JwtCredentials {
  type: 'jwt';
  token: string;
}

/**
 * Authentication credentials - either admin or JWT
 */
export type Credentials = AdminCredentials | JwtCredentials;

/**
 * Type guard to check if credentials are JWT-based
 */
export function isJwtCredentials(credentials: Credentials): credentials is JwtCredentials {
  return 'type' in credentials && credentials.type === 'jwt';
}

/**
 * Authentication manager
 */
export class AuthManager {
  private _storage: TokenStorage;
  private expiresAt: number | null = null;
  private refreshTimer?: NodeJS.Timeout;

  constructor(storage?: TokenStorage) {
    this._storage = storage ?? new MemoryTokenStorage();
  }

  /**
   * Get storage (for direct token manipulation)
   */
  get storage(): TokenStorage {
    return this._storage;
  }

  /**
   * Store authentication response
   *
   * @param response - Authentication response from server
   */
  setTokens(response: AuthenticateResponse): void {
    this._storage.setAccessToken(response.access_token);
    this._storage.setRefreshToken(response.refresh_token);

    // Calculate expiration time (subtract 60 seconds for safety margin)
    const expiresInMs = (response.expires_in - 60) * 1000;
    this.expiresAt = Date.now() + expiresInMs;

    // Schedule automatic refresh
    this.scheduleTokenRefresh(expiresInMs);
  }

  /**
   * Get current access token
   */
  getAccessToken(): string | null {
    return this._storage.getAccessToken();
  }

  /**
   * Get current refresh token
   */
  getRefreshToken(): string | null {
    return this._storage.getRefreshToken();
  }

  /**
   * Check if authenticated
   */
  isAuthenticated(): boolean {
    const token = this._storage.getAccessToken();
    return token !== null && !this.isTokenExpired();
  }

  /**
   * Check if token is expired
   */
  isTokenExpired(): boolean {
    if (this.expiresAt === null) {
      return true;
    }
    return Date.now() >= this.expiresAt;
  }

  /**
   * Check if token needs refresh (within 5 minutes of expiration)
   */
  needsRefresh(): boolean {
    if (this.expiresAt === null) {
      return false;
    }
    const fiveMinutes = 5 * 60 * 1000;
    return Date.now() >= this.expiresAt - fiveMinutes;
  }

  /**
   * Clear authentication state
   */
  clear(): void {
    this._storage.clear();
    this.expiresAt = null;
    this.cancelTokenRefresh();
  }

  /**
   * Schedule automatic token refresh
   *
   * @param delayMs - Delay in milliseconds
   */
  private scheduleTokenRefresh(delayMs: number): void {
    this.cancelTokenRefresh();

    this.refreshTimer = setTimeout(() => {
      // Emit event that refresh is needed
      // The client should handle this by calling the refresh endpoint
    }, delayMs);
  }

  /**
   * Cancel scheduled token refresh
   */
  private cancelTokenRefresh(): void {
    if (this.refreshTimer) {
      clearTimeout(this.refreshTimer);
      this.refreshTimer = undefined;
    }
  }

  /**
   * Create authentication payload for admin auth
   *
   * @param credentials - Admin credentials
   */
  static createAuthPayload(credentials: AdminCredentials): AuthenticatePayload {
    return {
      username: credentials.username,
      password: credentials.password,
    };
  }

  /**
   * Create refresh token payload
   *
   * @param refreshToken - Refresh token
   */
  static createRefreshPayload(refreshToken: string): RefreshTokenPayload {
    return {
      refresh_token: refreshToken,
    };
  }

  /**
   * Parse JWT token (without verification)
   * This is just for extracting expiration time, not for security
   *
   * @param token - JWT token
   * @returns Parsed token payload or null if invalid
   */
  static parseToken(token: string): Record<string, unknown> | null {
    try {
      const parts = token.split('.');
      if (parts.length !== 3) {
        return null;
      }

      const payload = parts[1];
      const decoded = atob(payload.replace(/-/g, '+').replace(/_/g, '/'));
      return JSON.parse(decoded);
    } catch (error) {
      return null;
    }
  }
}
