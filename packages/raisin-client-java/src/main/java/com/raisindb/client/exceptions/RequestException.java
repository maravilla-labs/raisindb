package com.raisindb.client.exceptions;

/**
 * Exception thrown when a request fails.
 */
public class RequestException extends RaisinDBException {

    private final String code;
    private final Object details;

    public RequestException(String message, String code) {
        this(message, code, null);
    }

    public RequestException(String message, String code, Object details) {
        super(message);
        this.code = code;
        this.details = details;
    }

    public String getCode() {
        return code;
    }

    public Object getDetails() {
        return details;
    }
}
