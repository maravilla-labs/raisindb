package com.raisindb.client.exceptions;

/**
 * Exception thrown when a request times out.
 */
public class TimeoutException extends RaisinDBException {

    public TimeoutException(String message) {
        super(message);
    }
}
