package com.raisindb.client.protocol;

import com.fasterxml.jackson.annotation.JsonValue;

/**
 * Response status codes.
 */
public enum ResponseStatus {
    SUCCESS("success"),
    ERROR("error"),
    STREAMING("streaming"),
    COMPLETE("complete"),
    ACKNOWLEDGED("acknowledged");

    private final String value;

    ResponseStatus(String value) {
        this.value = value;
    }

    @JsonValue
    public String getValue() {
        return value;
    }

    @Override
    public String toString() {
        return value;
    }
}
