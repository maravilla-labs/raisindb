package com.raisindb.client.connection;

import com.raisindb.client.exceptions.ConnectionException;
import com.raisindb.client.protocol.EventMessage;
import com.raisindb.client.protocol.ResponseEnvelope;
import com.raisindb.client.utils.MessagePackCodec;
import com.raisindb.client.utils.ReconnectManager;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import jakarta.websocket.*;
import org.glassfish.tyrus.client.ClientManager;

import java.io.IOException;
import java.net.URI;
import java.nio.ByteBuffer;
import java.util.List;
import java.util.concurrent.CopyOnWriteArrayList;
import java.util.function.Consumer;

/**
 * Manages WebSocket connection with automatic reconnection.
 */
@ClientEndpoint
public class ConnectionManager {

    private static final Logger logger = LoggerFactory.getLogger(ConnectionManager.class);

    private final String url;
    private final ReconnectManager reconnectManager;
    private final MessagePackCodec codec;
    private final List<Consumer<Object>> messageHandlers;
    private final List<Runnable> closeHandlers;
    private final List<Consumer<Exception>> errorHandlers;

    private Session session;
    private boolean connected;
    private boolean closing;
    private final ClientManager client;

    public ConnectionManager(
            String url,
            long initialReconnectDelay,
            long maxReconnectDelay,
            Integer maxReconnectAttempts
    ) {
        this.url = convertUrl(url);
        this.reconnectManager = new ReconnectManager(
                initialReconnectDelay, maxReconnectDelay, maxReconnectAttempts);
        this.codec = new MessagePackCodec();
        this.messageHandlers = new CopyOnWriteArrayList<>();
        this.closeHandlers = new CopyOnWriteArrayList<>();
        this.errorHandlers = new CopyOnWriteArrayList<>();
        this.connected = false;
        this.closing = false;
        this.client = ClientManager.createClient();
    }

    /**
     * Convert raisin:// URL to ws:// or wss:// URL.
     */
    private String convertUrl(String url) {
        if (url.startsWith("raisin://")) {
            URI uri = URI.create(url);
            String scheme = uri.getPort() == 443 ? "wss" : "ws";
            return url.replace("raisin://", scheme + "://") + "/ws";
        } else if (url.startsWith("ws://") || url.startsWith("wss://")) {
            return url;
        } else {
            throw new IllegalArgumentException("Invalid URL scheme: " + url);
        }
    }

    /**
     * Register a message handler.
     */
    public void onMessage(Consumer<Object> handler) {
        messageHandlers.add(handler);
    }

    /**
     * Register a close handler.
     */
    public void onClose(Runnable handler) {
        closeHandlers.add(handler);
    }

    /**
     * Register an error handler.
     */
    public void onError(Consumer<Exception> handler) {
        errorHandlers.add(handler);
    }

    /**
     * Connect to the WebSocket server.
     */
    public void connect() throws ConnectionException {
        if (connected) {
            return;
        }

        try {
            session = client.connectToServer(this, URI.create(url));
            connected = true;
            closing = false;
            reconnectManager.reset();

            logger.info("Connected to {}", url);

        } catch (Exception e) {
            logger.error("Failed to connect: {}", e.getMessage());
            throw new ConnectionException("Failed to connect to " + url, e);
        }
    }

    /**
     * Close the WebSocket connection.
     */
    public void close() {
        closing = true;
        connected = false;

        if (session != null) {
            try {
                session.close();
            } catch (IOException e) {
                logger.error("Error closing session: {}", e.getMessage());
            }
            session = null;
        }

        for (Runnable handler : closeHandlers) {
            try {
                handler.run();
            } catch (Exception e) {
                logger.error("Error in close handler: {}", e.getMessage());
            }
        }
    }

    /**
     * Send binary data over the WebSocket.
     */
    public void send(byte[] data) throws ConnectionException {
        if (!connected || session == null) {
            throw new ConnectionException("Not connected");
        }

        try {
            session.getBasicRemote().sendBinary(ByteBuffer.wrap(data));
        } catch (IOException e) {
            logger.error("Failed to send message: {}", e.getMessage());
            handleDisconnect();
            throw new ConnectionException("Failed to send message", e);
        }
    }

    /**
     * WebSocket message handler.
     */
    @OnMessage
    public void onWebSocketMessage(ByteBuffer buffer) {
        try {
            byte[] data = new byte[buffer.remaining()];
            buffer.get(data);

            // Decode MessagePack response or event
            Object decoded = codec.decode(data);

            // Notify handlers
            for (Consumer<Object> handler : messageHandlers) {
                try {
                    handler.accept(decoded);
                } catch (Exception e) {
                    logger.error("Error in message handler: {}", e.getMessage());
                }
            }

        } catch (Exception e) {
            logger.error("Error receiving message: {}", e.getMessage());
            for (Consumer<Exception> handler : errorHandlers) {
                try {
                    handler.accept(e);
                } catch (Exception err) {
                    logger.error("Error in error handler: {}", err.getMessage());
                }
            }
        }
    }

    /**
     * WebSocket close handler.
     */
    @OnClose
    public void onWebSocketClose(Session session, CloseReason closeReason) {
        logger.warn("WebSocket connection closed: {}", closeReason);
        handleDisconnect();
    }

    /**
     * WebSocket error handler.
     */
    @OnError
    public void onWebSocketError(Session session, Throwable throwable) {
        logger.error("WebSocket error: {}", throwable.getMessage());
        for (Consumer<Exception> handler : errorHandlers) {
            try {
                if (throwable instanceof Exception) {
                    handler.accept((Exception) throwable);
                } else {
                    handler.accept(new Exception(throwable));
                }
            } catch (Exception e) {
                logger.error("Error in error handler: {}", e.getMessage());
            }
        }
    }

    /**
     * Handle disconnection and attempt reconnection.
     */
    private void handleDisconnect() {
        if (closing) {
            return;
        }

        connected = false;

        // Attempt reconnection in background thread
        new Thread(() -> {
            while (reconnectManager.shouldReconnect() && !closing) {
                try {
                    reconnectManager.waitBeforeReconnect();
                    logger.info("Attempting to reconnect (attempt {})...",
                            reconnectManager.getAttempt());
                    connect();
                    return; // Successfully reconnected

                } catch (Exception e) {
                    logger.error("Reconnection failed: {}", e.getMessage());
                }
            }

            // Max reconnect attempts reached or closing
            if (!closing) {
                logger.error("Max reconnection attempts reached");
                for (Runnable handler : closeHandlers) {
                    try {
                        handler.run();
                    } catch (Exception e) {
                        logger.error("Error in close handler: {}", e.getMessage());
                    }
                }
            }
        }).start();
    }

    /**
     * Check if the connection is active.
     */
    public boolean isConnected() {
        return connected && session != null && session.isOpen();
    }
}
