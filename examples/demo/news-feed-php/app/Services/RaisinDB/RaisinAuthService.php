<?php

namespace App\Services\RaisinDB;

use Illuminate\Support\Facades\Http;
use Illuminate\Support\Facades\Cookie;

/**
 * Authentication service for RaisinDB identity auth.
 *
 * Uses the repo-scoped auth endpoints: /auth/{repo}/register and /auth/{repo}/login
 */
class RaisinAuthService
{
    /**
     * Repository ID for this demo
     */
    protected string $repoId;

    /**
     * Auth API base URL
     */
    protected string $apiUrl;

    /**
     * Cookie names
     */
    const ACCESS_TOKEN_COOKIE = 'access_token';
    const REFRESH_TOKEN_COOKIE = 'refresh_token';

    public function __construct()
    {
        $this->repoId = config('services.raisindb.repo_id', 'social_feed_demo_rel4');
        $this->apiUrl = config('services.raisindb.auth_url', 'http://localhost:8081');
    }

    /**
     * Register a new user
     *
     * @return array{success: bool, tokens?: array, error?: array}
     */
    public function register(string $email, string $password, ?string $displayName = null): array
    {
        try {
            $response = Http::post("{$this->apiUrl}/auth/{$this->repoId}/register", [
                'email' => $email,
                'password' => $password,
                'display_name' => $displayName,
            ]);

            if ($response->successful()) {
                return [
                    'success' => true,
                    'tokens' => $response->json(),
                ];
            }

            $error = $response->json();
            return [
                'success' => false,
                'error' => [
                    'code' => $error['code'] ?? 'REGISTRATION_FAILED',
                    'message' => $error['message'] ?? 'Registration failed',
                ],
            ];
        } catch (\Exception $e) {
            return [
                'success' => false,
                'error' => [
                    'code' => 'NETWORK_ERROR',
                    'message' => 'Unable to connect to auth server: ' . $e->getMessage(),
                ],
            ];
        }
    }

    /**
     * Login an existing user
     *
     * @return array{success: bool, tokens?: array, error?: array}
     */
    public function login(string $email, string $password, bool $rememberMe = false): array
    {
        try {
            $response = Http::post("{$this->apiUrl}/auth/{$this->repoId}/login", [
                'email' => $email,
                'password' => $password,
                'remember_me' => $rememberMe,
            ]);

            if ($response->successful()) {
                return [
                    'success' => true,
                    'tokens' => $response->json(),
                ];
            }

            $error = $response->json();
            return [
                'success' => false,
                'error' => [
                    'code' => $error['code'] ?? 'LOGIN_FAILED',
                    'message' => $error['message'] ?? 'Invalid email or password',
                ],
            ];
        } catch (\Exception $e) {
            return [
                'success' => false,
                'error' => [
                    'code' => 'NETWORK_ERROR',
                    'message' => 'Unable to connect to auth server: ' . $e->getMessage(),
                ],
            ];
        }
    }

    /**
     * Get user from access token (decode JWT payload)
     */
    public function getUserFromToken(string $accessToken): ?array
    {
        try {
            $parts = explode('.', $accessToken);
            if (count($parts) !== 3) {
                return null;
            }

            $payload = json_decode(base64_decode(strtr($parts[1], '-_', '+/')), true);
            if (!$payload) {
                return null;
            }

            // Check expiration
            if (isset($payload['exp']) && $payload['exp'] < time()) {
                return null;
            }

            return [
                'id' => $payload['sub'] ?? null,
                'email' => $payload['email'] ?? null,
                'display_name' => $payload['display_name'] ?? null,
                'avatar_url' => $payload['avatar_url'] ?? null,
                'email_verified' => $payload['global_flags']['email_verified'] ?? false,
            ];
        } catch (\Exception $e) {
            return null;
        }
    }

    /**
     * Get current user from cookies
     */
    public function getSessionUser(): ?array
    {
        $accessToken = Cookie::get(self::ACCESS_TOKEN_COOKIE);
        if (!$accessToken) {
            return null;
        }

        return $this->getUserFromToken($accessToken);
    }

    /**
     * Get access token from cookies
     */
    public function getAccessToken(): ?string
    {
        return Cookie::get(self::ACCESS_TOKEN_COOKIE);
    }

    /**
     * Create auth cookies from tokens response
     *
     * @return \Symfony\Component\HttpFoundation\Cookie[]
     */
    public function createAuthCookies(array $tokens): array
    {
        $accessTokenExpiry = isset($tokens['expires_at'])
            ? (int) (($tokens['expires_at'] - (time() * 1000)) / 1000)
            : 86400; // 24 hours default

        return [
            cookie(
                self::ACCESS_TOKEN_COOKIE,
                $tokens['access_token'],
                $accessTokenExpiry / 60, // Convert to minutes
                '/',
                null,
                false, // secure
                true   // httpOnly
            ),
            cookie(
                self::REFRESH_TOKEN_COOKIE,
                $tokens['refresh_token'],
                43200, // 30 days in minutes
                '/',
                null,
                false, // secure
                true   // httpOnly
            ),
        ];
    }

    /**
     * Create cookies to clear auth
     *
     * @return \Symfony\Component\HttpFoundation\Cookie[]
     */
    public function createLogoutCookies(): array
    {
        return [
            cookie()->forget(self::ACCESS_TOKEN_COOKIE),
            cookie()->forget(self::REFRESH_TOKEN_COOKIE),
        ];
    }
}
