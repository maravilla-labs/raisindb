package com.raisindb.client.utils;

import com.raisindb.client.exceptions.RequestException;
import com.raisindb.client.exceptions.TimeoutException;
import com.raisindb.client.protocol.*;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import java.util.Map;
import java.util.UUID;
import java.util.concurrent.*;
import java.util.function.Consumer;

/**
 * Tracks pending requests and resolves them when responses arrive.
 */
public class RequestTracker {

    private static final Logger logger = LoggerFactory.getLogger(RequestTracker.class);

    private final Consumer<byte[]> sendFunc;
    private final long defaultTimeout;
    private final Map<String, CompletableFuture<Object>> pendingRequests;
    private final ScheduledExecutorService scheduler;
    private final MessagePackCodec codec;

    public RequestTracker(Consumer<byte[]> sendFunc, long defaultTimeout) {
        this.sendFunc = sendFunc;
        this.defaultTimeout = defaultTimeout;
        this.pendingRequests = new ConcurrentHashMap<>();
        this.scheduler = Executors.newScheduledThreadPool(2);
        this.codec = new MessagePackCodec();
    }

    /**
     * Send a request and wait for the response.
     *
     * @param requestType Type of request
     * @param context     Request context
     * @param payload     Request payload
     * @return CompletableFuture with the response result
     */
    public CompletableFuture<Object> sendRequest(
            RequestType requestType,
            RequestContext context,
            Object payload
    ) {
        return sendRequest(requestType, context, payload, defaultTimeout);
    }

    /**
     * Send a request with custom timeout.
     */
    public CompletableFuture<Object> sendRequest(
            RequestType requestType,
            RequestContext context,
            Object payload,
            long timeout
    ) {
        String requestId = UUID.randomUUID().toString();
        CompletableFuture<Object> future = new CompletableFuture<>();

        // Store pending request
        pendingRequests.put(requestId, future);

        // Set timeout
        ScheduledFuture<?> timeoutHandle = scheduler.schedule(() -> {
            CompletableFuture<Object> pending = pendingRequests.remove(requestId);
            if (pending != null && !pending.isDone()) {
                pending.completeExceptionally(
                        new TimeoutException("Request " + requestId + " timed out"));
            }
        }, timeout, TimeUnit.MILLISECONDS);

        // Cancel timeout when future completes
        future.whenComplete((result, error) -> timeoutHandle.cancel(false));

        try {
            // Create and send request envelope
            RequestEnvelope envelope = new RequestEnvelope(requestId, requestType, context, payload);
            byte[] data = codec.encodeRequest(envelope);
            sendFunc.accept(data);

        } catch (Exception e) {
            pendingRequests.remove(requestId);
            future.completeExceptionally(e);
        }

        return future;
    }

    /**
     * Handle an incoming response.
     */
    public void handleResponse(ResponseEnvelope response) {
        CompletableFuture<Object> future = pendingRequests.remove(response.getRequestId());

        if (future == null) {
            // Response for unknown request (might have timed out)
            logger.warn("Received response for unknown request: {}", response.getRequestId());
            return;
        }

        if (response.getStatus() == ResponseStatus.ERROR) {
            ErrorInfo error = response.getError();
            if (error != null) {
                future.completeExceptionally(
                        new RequestException(error.getMessage(), error.getCode(), error.getDetails()));
            } else {
                future.completeExceptionally(
                        new RequestException("Unknown error", "UNKNOWN"));
            }
        } else if (response.getStatus() == ResponseStatus.SUCCESS ||
                   response.getStatus() == ResponseStatus.COMPLETE) {
            future.complete(response.getResult());
        } else if (response.getStatus() == ResponseStatus.STREAMING) {
            // For now, just return the first chunk
            // TODO: Implement proper streaming support
            future.complete(response.getResult());
        }
    }

    /**
     * Shutdown the request tracker.
     */
    public void shutdown() {
        scheduler.shutdown();
        try {
            if (!scheduler.awaitTermination(5, TimeUnit.SECONDS)) {
                scheduler.shutdownNow();
            }
        } catch (InterruptedException e) {
            scheduler.shutdownNow();
            Thread.currentThread().interrupt();
        }
    }

    public MessagePackCodec getCodec() {
        return codec;
    }
}
