package com.raisindb.client.auth;

/**
 * Abstract interface for token storage.
 */
public interface TokenStorage {

    /**
     * Get stored access and refresh tokens.
     *
     * @return Array with [accessToken, refreshToken] or null if not stored
     */
    String[] getTokens();

    /**
     * Store access and refresh tokens.
     *
     * @param accessToken  Access token
     * @param refreshToken Refresh token
     */
    void setTokens(String accessToken, String refreshToken);

    /**
     * Clear stored tokens.
     */
    void clearTokens();
}
