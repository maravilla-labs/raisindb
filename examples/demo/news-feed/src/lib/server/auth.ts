/**
 * Authentication utilities for the news-feed demo.
 *
 * Uses the repo-scoped auth endpoints: /auth/{repo}/register and /auth/{repo}/login
 */

import type { Cookies } from '@sveltejs/kit';
import { env } from '$env/dynamic/private';

// The repository name for this demo
const REPO_ID = 'social_feed_demo_rel4';

// Auth API base URL (defaults to localhost for development)
const AUTH_API_URL = env.AUTH_API_URL || 'http://localhost:8081';

// Cookie settings
const ACCESS_TOKEN_COOKIE = 'access_token';
const REFRESH_TOKEN_COOKIE = 'refresh_token';
const COOKIE_OPTIONS = {
	path: '/',
	httpOnly: true,
	secure: false, // Set to true in production with HTTPS
	sameSite: 'lax' as const,
	maxAge: 60 * 60 * 24 // 24 hours
};

export interface AuthTokensResponse {
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
	};
}

export interface User {
	id: string;
	email: string;
	displayName: string | null;
	avatarUrl: string | null;
	emailVerified: boolean;
}

export interface AuthError {
	code: string;
	message: string;
}

/**
 * Register a new user for this repository
 */
export async function register(
	email: string,
	password: string,
	displayName?: string
): Promise<{ success: true; tokens: AuthTokensResponse } | { success: false; error: AuthError }> {
	try {
		const response = await fetch(`${AUTH_API_URL}/auth/${REPO_ID}/register`, {
			method: 'POST',
			headers: {
				'Content-Type': 'application/json'
			},
			body: JSON.stringify({
				email,
				password,
				display_name: displayName
			})
		});

		if (!response.ok) {
			const contentType = response.headers.get('content-type');
						let error: any = { code: response.status.toString() };
						
						if (contentType && contentType.includes('application/json')) {
							error = await response.json();
						} else {
							const text = await response.text();
							error.message = text || response.statusText;
						}
			return {
				success: false,
				error: {
					code: error.code || 'REGISTRATION_FAILED',
					message: error.message || 'Registration failed'
				}
			};
		}

		const tokens: AuthTokensResponse = await response.json();
		return { success: true, tokens };
	} catch (err) {
		console.error('Registration error:', err);
		return {
			success: false,
			error: {
				code: 'NETWORK_ERROR',
				message: 'Unable to connect to auth server'
			}
		};
	}
}

/**
 * Login an existing user
 */
export async function login(
	email: string,
	password: string,
	rememberMe = false
): Promise<{ success: true; tokens: AuthTokensResponse } | { success: false; error: AuthError }> {
	try {
		const response = await fetch(`${AUTH_API_URL}/auth/${REPO_ID}/login`, {
			method: 'POST',
			headers: {
				'Content-Type': 'application/json'
			},
			body: JSON.stringify({
				email,
				password,
				remember_me: rememberMe
			})
		});

		if (!response.ok) {
			const error = await response.json();
			return {
				success: false,
				error: {
					code: error.code || 'LOGIN_FAILED',
					message: error.message || 'Invalid email or password'
				}
			};
		}

		const tokens: AuthTokensResponse = await response.json();
		return { success: true, tokens };
	} catch (err) {
		console.error('Login error:', err);
		return {
			success: false,
			error: {
				code: 'NETWORK_ERROR',
				message: 'Unable to connect to auth server'
			}
		};
	}
}

/**
 * Set auth cookies after successful login/register
 */
export function setAuthCookies(cookies: Cookies, tokens: AuthTokensResponse): void {
	cookies.set(ACCESS_TOKEN_COOKIE, tokens.access_token, {
		...COOKIE_OPTIONS,
		maxAge: Math.floor((tokens.expires_at - Date.now()) / 1000)
	});

	cookies.set(REFRESH_TOKEN_COOKIE, tokens.refresh_token, {
		...COOKIE_OPTIONS,
		maxAge: 60 * 60 * 24 * 30 // 30 days for refresh token
	});
}

/**
 * Clear auth cookies on logout
 */
export function clearAuthCookies(cookies: Cookies): void {
	cookies.delete(ACCESS_TOKEN_COOKIE, { path: '/' });
	cookies.delete(REFRESH_TOKEN_COOKIE, { path: '/' });
}

/**
 * Get user from access token
 */
export function getUserFromToken(accessToken: string): User | null {
	try {
		// JWT is base64 encoded: header.payload.signature
		const [, payloadBase64] = accessToken.split('.');
		if (!payloadBase64) return null;

		// Decode the payload
		const payload = JSON.parse(Buffer.from(payloadBase64, 'base64').toString('utf-8'));

		return {
			id: payload.sub,
			email: payload.email,
			displayName: payload.display_name || null,
			avatarUrl: payload.avatar_url || null,
			emailVerified: payload.global_flags?.email_verified ?? false
		};
	} catch {
		return null;
	}
}

/**
 * Get current user from cookies
 */
export function getSessionUser(cookies: Cookies): User | null {
	const accessToken = cookies.get(ACCESS_TOKEN_COOKIE);
	if (!accessToken) return null;

	// Check if token is expired
	try {
		const [, payloadBase64] = accessToken.split('.');
		if (!payloadBase64) return null;

		const payload = JSON.parse(Buffer.from(payloadBase64, 'base64').toString('utf-8'));

		// Check expiration
		if (payload.exp && payload.exp * 1000 < Date.now()) {
			// Token is expired - TODO: implement token refresh
			return null;
		}

		return getUserFromToken(accessToken);
	} catch {
		return null;
	}
}

/**
 * Get the access token from cookies (for API calls)
 */
export function getAccessToken(cookies: Cookies): string | null {
	return cookies.get(ACCESS_TOKEN_COOKIE) || null;
}
