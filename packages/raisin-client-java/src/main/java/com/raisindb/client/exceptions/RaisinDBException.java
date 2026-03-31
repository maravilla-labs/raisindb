package com.raisindb.client.exceptions;

/**
 * Base exception for all RaisinDB client errors.
 */
public class RaisinDBException extends Exception {

    public RaisinDBException(String message) {
        super(message);
    }

    public RaisinDBException(String message, Throwable cause) {
        super(message, cause);
    }
}
