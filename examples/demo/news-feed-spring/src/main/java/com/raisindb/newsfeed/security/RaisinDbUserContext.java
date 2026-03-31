package com.raisindb.newsfeed.security;

/**
 * User context for RaisinDB operations.
 * Contains both user info and access token for database queries.
 */
public class RaisinDbUserContext {

    private String accessToken;
    private UserContext user;

    public RaisinDbUserContext() {
    }

    public RaisinDbUserContext(String accessToken, UserContext user) {
        this.accessToken = accessToken;
        this.user = user;
    }

    public String getAccessToken() {
        return accessToken;
    }

    public void setAccessToken(String accessToken) {
        this.accessToken = accessToken;
    }

    public UserContext getUser() {
        return user;
    }

    public void setUser(UserContext user) {
        this.user = user;
    }

    public boolean isAuthenticated() {
        return user != null && accessToken != null && !accessToken.isEmpty();
    }
}
