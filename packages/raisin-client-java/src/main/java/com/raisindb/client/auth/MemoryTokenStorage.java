package com.raisindb.client.auth;

/**
 * In-memory token storage (default implementation).
 */
public class MemoryTokenStorage implements TokenStorage {

    private String accessToken;
    private String refreshToken;

    @Override
    public String[] getTokens() {
        if (accessToken != null && refreshToken != null) {
            return new String[]{accessToken, refreshToken};
        }
        return null;
    }

    @Override
    public void setTokens(String accessToken, String refreshToken) {
        this.accessToken = accessToken;
        this.refreshToken = refreshToken;
    }

    @Override
    public void clearTokens() {
        this.accessToken = null;
        this.refreshToken = null;
    }
}
