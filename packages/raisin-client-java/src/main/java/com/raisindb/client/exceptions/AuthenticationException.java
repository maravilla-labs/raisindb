package com.raisindb.client.exceptions;

/**
 * Exception thrown when authentication fails.
 */
public class AuthenticationException extends RaisinDBException {

    public AuthenticationException(String message) {
        super(message);
    }

    public AuthenticationException(String message, Throwable cause) {
        super(message, cause);
    }
}
