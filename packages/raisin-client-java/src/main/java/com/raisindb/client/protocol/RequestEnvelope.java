package com.raisindb.client.protocol;

import com.fasterxml.jackson.annotation.JsonProperty;

/**
 * Request envelope sent to server.
 */
public class RequestEnvelope {

    @JsonProperty("request_id")
    private String requestId;

    @JsonProperty("type")
    private RequestType requestType;

    @JsonProperty("context")
    private RequestContext context;

    @JsonProperty("payload")
    private Object payload;

    public RequestEnvelope() {
    }

    public RequestEnvelope(String requestId, RequestType requestType, RequestContext context, Object payload) {
        this.requestId = requestId;
        this.requestType = requestType;
        this.context = context;
        this.payload = payload;
    }

    // Getters and setters
    public String getRequestId() { return requestId; }
    public void setRequestId(String requestId) { this.requestId = requestId; }

    public RequestType getRequestType() { return requestType; }
    public void setRequestType(RequestType requestType) { this.requestType = requestType; }

    public RequestContext getContext() { return context; }
    public void setContext(RequestContext context) { this.context = context; }

    public Object getPayload() { return payload; }
    public void setPayload(Object payload) { this.payload = payload; }
}
