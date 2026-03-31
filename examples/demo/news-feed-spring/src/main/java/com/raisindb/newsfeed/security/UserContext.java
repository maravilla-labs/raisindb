package com.raisindb.newsfeed.security;

/**
 * User context extracted from JWT token.
 */
public class UserContext {

    private String id;
    private String email;
    private String displayName;

    public UserContext() {
    }

    public String getId() {
        return id;
    }

    public void setId(String id) {
        this.id = id;
    }

    public String getEmail() {
        return email;
    }

    public void setEmail(String email) {
        this.email = email;
    }

    public String getDisplayName() {
        return displayName;
    }

    public void setDisplayName(String displayName) {
        this.displayName = displayName;
    }

    /**
     * Get the display name or email as fallback.
     */
    public String getDisplayNameOrEmail() {
        if (displayName != null && !displayName.isEmpty()) {
            return displayName;
        }
        return email;
    }
}
