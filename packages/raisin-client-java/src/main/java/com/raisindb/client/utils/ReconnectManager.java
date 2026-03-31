package com.raisindb.client.utils;

/**
 * Manages reconnection attempts with exponential backoff.
 */
public class ReconnectManager {

    private final long initialDelay;
    private final long maxDelay;
    private final Integer maxAttempts;
    private long currentDelay;
    private int attempt;
    private boolean isReconnecting;

    public ReconnectManager(long initialDelay, long maxDelay, Integer maxAttempts) {
        this.initialDelay = initialDelay;
        this.maxDelay = maxDelay;
        this.maxAttempts = maxAttempts;
        this.currentDelay = initialDelay;
        this.attempt = 0;
        this.isReconnecting = false;
    }

    /**
     * Reset reconnection state after successful connection.
     */
    public void reset() {
        this.currentDelay = initialDelay;
        this.attempt = 0;
        this.isReconnecting = false;
    }

    /**
     * Check if another reconnection attempt should be made.
     */
    public boolean shouldReconnect() {
        if (maxAttempts == null) {
            return true;
        }
        return attempt < maxAttempts;
    }

    /**
     * Get the next delay duration and increment attempt counter.
     */
    public long nextDelay() {
        long delay = currentDelay;
        currentDelay = Math.min(currentDelay * 2, maxDelay);
        attempt++;
        return delay;
    }

    /**
     * Wait for the appropriate delay before attempting reconnection.
     */
    public void waitBeforeReconnect() throws InterruptedException {
        long delay = nextDelay();
        Thread.sleep(delay);
    }

    // Getters
    public int getAttempt() { return attempt; }
    public boolean isReconnecting() { return isReconnecting; }
    public void setReconnecting(boolean reconnecting) { isReconnecting = reconnecting; }
}
