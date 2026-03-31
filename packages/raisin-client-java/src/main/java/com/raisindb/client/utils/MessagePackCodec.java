package com.raisindb.client.utils;

import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.datatype.jsr310.JavaTimeModule;
import org.msgpack.jackson.dataformat.MessagePackFactory;

import com.raisindb.client.protocol.EventMessage;
import com.raisindb.client.protocol.RequestEnvelope;
import com.raisindb.client.protocol.ResponseEnvelope;

import java.io.IOException;
import java.nio.ByteBuffer;

/**
 * MessagePack codec for encoding/decoding protocol messages.
 */
public class MessagePackCodec {

    private final ObjectMapper mapper;

    public MessagePackCodec() {
        this.mapper = new ObjectMapper(new MessagePackFactory());
        this.mapper.registerModule(new JavaTimeModule());
    }

    /**
     * Encode a request to MessagePack bytes.
     */
    public byte[] encodeRequest(RequestEnvelope request) throws IOException {
        return mapper.writeValueAsBytes(request);
    }

    /**
     * Encode a request to MessagePack ByteBuffer.
     */
    public ByteBuffer encodeRequestToBuffer(RequestEnvelope request) throws IOException {
        return ByteBuffer.wrap(encodeRequest(request));
    }

    /**
     * Decode MessagePack bytes to a response or event.
     * Returns ResponseEnvelope if it's a response, EventMessage if it's an event.
     */
    public Object decode(byte[] data) throws IOException {
        // First, peek at the structure to determine if it's a response or event
        @SuppressWarnings("unchecked")
        java.util.Map<String, Object> map = mapper.readValue(data, java.util.Map.class);

        if (map.containsKey("subscription_id")) {
            // It's an event message
            return mapper.readValue(data, EventMessage.class);
        } else {
            // It's a response envelope
            return mapper.readValue(data, ResponseEnvelope.class);
        }
    }

    /**
     * Decode MessagePack bytes to a specific type.
     */
    public <T> T decode(byte[] data, Class<T> valueType) throws IOException {
        return mapper.readValue(data, valueType);
    }

    /**
     * Get the underlying ObjectMapper for custom operations.
     */
    public ObjectMapper getMapper() {
        return mapper;
    }
}
