package com.raisindb.client.exceptions;

/**
 * Exception thrown when connection to server fails.
 */
public class ConnectionException extends RaisinDBException {

    public ConnectionException(String message) {
        super(message);
    }

    public ConnectionException(String message, Throwable cause) {
        super(message, cause);
    }
}
