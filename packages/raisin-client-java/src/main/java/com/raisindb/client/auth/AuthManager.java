package com.raisindb.client.auth;

import com.raisindb.client.exceptions.AuthenticationException;
import com.raisindb.client.protocol.RequestContext;
import com.raisindb.client.protocol.RequestType;
import com.raisindb.client.utils.RequestTracker;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import java.util.HashMap;
import java.util.Map;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.ExecutionException;
import java.util.concurrent.ScheduledExecutorService;
import java.util.concurrent.ScheduledFuture;
import java.util.concurrent.TimeUnit;

/**
 * Manages authentication and token lifecycle.
 */
public class AuthManager {

    private static final Logger logger = LoggerFactory.getLogger(AuthManager.class);

    private final RequestTracker requestTracker;
    private final String tenantId;
    private final TokenStorage tokenStorage;
    private final ScheduledExecutorService scheduler;

    private boolean authenticated;
    private ScheduledFuture<?> refreshTask;

    public AuthManager(
            RequestTracker requestTracker,
            String tenantId,
            TokenStorage tokenStorage,
            ScheduledExecutorService scheduler
    ) {
        this.requestTracker = requestTracker;
        this.tenantId = tenantId;
        this.tokenStorage = tokenStorage != null ? tokenStorage : new MemoryTokenStorage();
        this.scheduler = scheduler;
        this.authenticated = false;
    }

    /**
     * Authenticate with username and password.
     */
    public CompletableFuture<Void> authenticate(String username, String password) {
        RequestContext context = new RequestContext(tenantId);

        Map<String, Object> payload = new HashMap<>();
        payload.put("username", username);
        payload.put("password", password);

        return requestTracker.sendRequest(RequestType.AUTHENTICATE, context, payload)
                .thenCompose(result -> {
                    try {
                        @SuppressWarnings("unchecked")
                        Map<String, Object> response = (Map<String, Object>) result;

                        String accessToken = (String) response.get("access_token");
                        String refreshToken = (String) response.get("refresh_token");

                        if (accessToken == null || refreshToken == null) {
                            throw new AuthenticationException("Invalid authentication response");
                        }

                        tokenStorage.setTokens(accessToken, refreshToken);
                        authenticated = true;

                        // Start automatic token refresh
                        int expiresIn = response.containsKey("expires_in") ?
                                ((Number) response.get("expires_in")).intValue() : 3600;
                        startTokenRefresh(expiresIn);

                        logger.info("Authenticated as {}", username);
                        return CompletableFuture.completedFuture(null);

                    } catch (Exception e) {
                        logger.error("Authentication failed: {}", e.getMessage());
                        return CompletableFuture.failedFuture(
                                new AuthenticationException("Authentication failed", e));
                    }
                });
    }

    /**
     * Refresh the access token using the refresh token.
     */
    public CompletableFuture<Void> refreshAccessToken() {
        String[] tokens = tokenStorage.getTokens();
        if (tokens == null) {
            return CompletableFuture.failedFuture(
                    new AuthenticationException("No refresh token available"));
        }

        RequestContext context = new RequestContext(tenantId);

        Map<String, Object> payload = new HashMap<>();
        payload.put("refresh_token", tokens[1]);

        return requestTracker.sendRequest(RequestType.REFRESH_TOKEN, context, payload)
                .thenCompose(result -> {
                    try {
                        @SuppressWarnings("unchecked")
                        Map<String, Object> response = (Map<String, Object>) result;

                        String accessToken = (String) response.get("access_token");
                        String refreshToken = (String) response.get("refresh_token");

                        if (accessToken == null || refreshToken == null) {
                            throw new AuthenticationException("Invalid refresh response");
                        }

                        tokenStorage.setTokens(accessToken, refreshToken);

                        logger.info("Access token refreshed");
                        return CompletableFuture.completedFuture(null);

                    } catch (Exception e) {
                        logger.error("Token refresh failed: {}", e.getMessage());
                        authenticated = false;
                        return CompletableFuture.failedFuture(
                                new AuthenticationException("Token refresh failed", e));
                    }
                });
    }

    /**
     * Start automatic token refresh.
     */
    private void startTokenRefresh(int expiresIn) {
        if (refreshTask != null) {
            refreshTask.cancel(false);
        }

        // Refresh token 1 minute before expiry
        long refreshDelay = Math.max(expiresIn - 60, 60);

        refreshTask = scheduler.scheduleAtFixedRate(() -> {
            try {
                refreshAccessToken().get();
            } catch (InterruptedException | ExecutionException e) {
                logger.error("Automatic token refresh failed: {}", e.getMessage());
            }
        }, refreshDelay, refreshDelay, TimeUnit.SECONDS);
    }

    /**
     * Logout and clear tokens.
     */
    public void logout() {
        if (refreshTask != null) {
            refreshTask.cancel(false);
            refreshTask = null;
        }

        tokenStorage.clearTokens();
        authenticated = false;
        logger.info("Logged out");
    }

    /**
     * Get the current access token.
     */
    public String getAccessToken() {
        String[] tokens = tokenStorage.getTokens();
        return tokens != null ? tokens[0] : null;
    }

    /**
     * Check if currently authenticated.
     */
    public boolean isAuthenticated() {
        return authenticated;
    }
}
