package com.raisindb.client.protocol;

import com.fasterxml.jackson.annotation.JsonProperty;
import java.time.Instant;

/**
 * Server-initiated event message.
 */
public class EventMessage {

    @JsonProperty("event_id")
    private String eventId;

    @JsonProperty("subscription_id")
    private String subscriptionId;

    @JsonProperty("event_type")
    private String eventType;

    @JsonProperty("payload")
    private Object payload;

    @JsonProperty("timestamp")
    private Instant timestamp;

    public EventMessage() {
    }

    // Getters and setters
    public String getEventId() { return eventId; }
    public void setEventId(String eventId) { this.eventId = eventId; }

    public String getSubscriptionId() { return subscriptionId; }
    public void setSubscriptionId(String subscriptionId) { this.subscriptionId = subscriptionId; }

    public String getEventType() { return eventType; }
    public void setEventType(String eventType) { this.eventType = eventType; }

    public Object getPayload() { return payload; }
    public void setPayload(Object payload) { this.payload = payload; }

    public Instant getTimestamp() { return timestamp; }
    public void setTimestamp(Instant timestamp) { this.timestamp = timestamp; }
}
