package com.raisindb.client.protocol;

import com.fasterxml.jackson.annotation.JsonInclude;
import com.fasterxml.jackson.annotation.JsonProperty;

/**
 * Response envelope received from server.
 */
@JsonInclude(JsonInclude.Include.NON_NULL)
public class ResponseEnvelope {

    @JsonProperty("request_id")
    private String requestId;

    @JsonProperty("status")
    private ResponseStatus status;

    @JsonProperty("result")
    private Object result;

    @JsonProperty("error")
    private ErrorInfo error;

    @JsonProperty("metadata")
    private ResponseMetadata metadata;

    public ResponseEnvelope() {
    }

    // Getters and setters
    public String getRequestId() { return requestId; }
    public void setRequestId(String requestId) { this.requestId = requestId; }

    public ResponseStatus getStatus() { return status; }
    public void setStatus(ResponseStatus status) { this.status = status; }

    public Object getResult() { return result; }
    public void setResult(Object result) { this.result = result; }

    public ErrorInfo getError() { return error; }
    public void setError(ErrorInfo error) { this.error = error; }

    public ResponseMetadata getMetadata() { return metadata; }
    public void setMetadata(ResponseMetadata metadata) { this.metadata = metadata; }
}
