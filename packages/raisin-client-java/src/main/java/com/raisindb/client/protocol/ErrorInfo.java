package com.raisindb.client.protocol;

import com.fasterxml.jackson.annotation.JsonInclude;
import com.fasterxml.jackson.annotation.JsonProperty;

/**
 * Error information in response.
 */
@JsonInclude(JsonInclude.Include.NON_NULL)
public class ErrorInfo {

    @JsonProperty("code")
    private String code;

    @JsonProperty("message")
    private String message;

    @JsonProperty("details")
    private Object details;

    public ErrorInfo() {
    }

    public ErrorInfo(String code, String message) {
        this.code = code;
        this.message = message;
    }

    public ErrorInfo(String code, String message, Object details) {
        this.code = code;
        this.message = message;
        this.details = details;
    }

    // Getters and setters
    public String getCode() { return code; }
    public void setCode(String code) { this.code = code; }

    public String getMessage() { return message; }
    public void setMessage(String message) { this.message = message; }

    public Object getDetails() { return details; }
    public void setDetails(Object details) { this.details = details; }
}
