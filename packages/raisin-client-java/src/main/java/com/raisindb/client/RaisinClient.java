package com.raisindb.client;

import com.raisindb.client.auth.AuthManager;
import com.raisindb.client.auth.TokenStorage;
import com.raisindb.client.connection.ConnectionManager;
import com.raisindb.client.exceptions.ConnectionException;
import com.raisindb.client.operations.Database;
import com.raisindb.client.protocol.EventMessage;
import com.raisindb.client.protocol.ResponseEnvelope;
import com.raisindb.client.utils.RequestTracker;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import java.net.URI;
import java.util.Map;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.Executors;
import java.util.concurrent.ScheduledExecutorService;
import java.util.function.Consumer;

/**
 * Main client for connecting to RaisinDB.
 *
 * <p>Example usage:
 * <pre>{@code
 * RaisinClient client = new RaisinClient("raisin://localhost:8080/sys/default");
 * client.connect();
 * client.authenticate("admin", "password").get();
 *
 * Database db = client.database("my_repo");
 * Workspace workspace = db.workspace("content");
 * // ... perform operations
 *
 * client.close();
 * }</pre>
 */
public class RaisinClient implements AutoCloseable {

    private static final Logger logger = LoggerFactory.getLogger(RaisinClient.class);

    private final String url;
    private final String tenantId;
    private final ConnectionManager connection;
    private final RequestTracker requestTracker;
    private final AuthManager authManager;
    private final ScheduledExecutorService scheduler;
    private final Map<String, Consumer<EventMessage>> eventHandlers;

    /**
     * Create a new RaisinDB client.
     *
     * @param url WebSocket URL (e.g., "raisin://localhost:8080/sys/default")
     */
    public RaisinClient(String url) {
        this(url, null, null, 1000, 30000, null, 30000);
    }

    /**
     * Create a new RaisinDB client with custom configuration.
     *
     * @param url                     WebSocket URL
     * @param tenantId                Tenant ID (extracted from URL if null)
     * @param tokenStorage            Token storage implementation
     * @param initialReconnectDelay   Initial reconnect delay in milliseconds
     * @param maxReconnectDelay       Maximum reconnect delay in milliseconds
     * @param maxReconnectAttempts    Maximum reconnect attempts (null for unlimited)
     * @param requestTimeout          Default request timeout in milliseconds
     */
    public RaisinClient(
            String url,
            String tenantId,
            TokenStorage tokenStorage,
            long initialReconnectDelay,
            long maxReconnectDelay,
            Integer maxReconnectAttempts,
            long requestTimeout
    ) {
        this.url = url;
        this.tenantId = tenantId != null ? tenantId : extractTenantFromUrl(url);
        this.scheduler = Executors.newScheduledThreadPool(4);
        this.eventHandlers = new ConcurrentHashMap<>();

        // Initialize connection manager
        this.connection = new ConnectionManager(
                url, initialReconnectDelay, maxReconnectDelay, maxReconnectAttempts);

        // Initialize request tracker
        this.requestTracker = new RequestTracker(
                data -> {
                    try {
                        connection.send(data);
                    } catch (ConnectionException e) {
                        logger.error("Failed to send request: {}", e.getMessage());
                    }
                },
                requestTimeout
        );

        // Initialize auth manager
        this.authManager = new AuthManager(
                requestTracker, this.tenantId, tokenStorage, scheduler);

        // Set up message handler
        connection.onMessage(this::handleMessage);
    }

    /**
     * Extract tenant ID from URL path.
     */
    private String extractTenantFromUrl(String url) {
        // URL format: raisin://host:port/sys/{tenant_id} or /sys/{tenant_id}/{repository}
        URI uri = URI.create(url.replace("raisin://", "http://"));
        String path = uri.getPath();
        String[] parts = path.split("/sys/");
        if (parts.length < 2) {
            throw new IllegalArgumentException("URL must include /sys/{tenant_id}");
        }
        return parts[1].split("/")[0];
    }

    /**
     * Handle incoming messages from WebSocket.
     */
    private void handleMessage(Object message) {
        if (message instanceof EventMessage) {
            handleEvent((EventMessage) message);
        } else if (message instanceof ResponseEnvelope) {
            requestTracker.handleResponse((ResponseEnvelope) message);
        }
    }

    /**
     * Handle an event message.
     */
    private void handleEvent(EventMessage event) {
        Consumer<EventMessage> handler = eventHandlers.get(event.getSubscriptionId());
        if (handler != null) {
            try {
                handler.accept(event);
            } catch (Exception e) {
                logger.error("Error in event handler: {}", e.getMessage());
            }
        }
    }

    /**
     * Register an event handler for a subscription.
     */
    void registerEventHandler(String subscriptionId, Consumer<EventMessage> handler) {
        eventHandlers.put(subscriptionId, handler);
    }

    /**
     * Unregister an event handler.
     */
    void unregisterEventHandler(String subscriptionId) {
        eventHandlers.remove(subscriptionId);
    }

    /**
     * Connect to the RaisinDB server.
     *
     * @throws ConnectionException if connection fails
     */
    public void connect() throws ConnectionException {
        connection.connect();
    }

    /**
     * Authenticate with username and password.
     *
     * @param username Username
     * @param password Password
     * @return CompletableFuture that completes when authentication succeeds
     */
    public CompletableFuture<Void> authenticate(String username, String password) {
        return authManager.authenticate(username, password);
    }

    /**
     * Get a database (repository) interface.
     *
     * @param name Repository name
     * @return Database instance
     */
    public Database database(String name) {
        return new Database(this, name);
    }

    /**
     * Create a new repository.
     *
     * @param repositoryId Repository identifier
     * @param description  Optional description
     * @param config       Optional repository configuration
     * @return CompletableFuture with created repository info
     */
    public CompletableFuture<Object> createRepository(String repositoryId, String description, Map<String, Object> config) {
        RequestContext context = RequestContext.builder()
                .tenantId(tenantId)
                .build();

        Map<String, Object> payload = new HashMap<>();
        payload.put("repository_id", repositoryId);
        if (description != null) {
            payload.put("description", description);
        }
        if (config != null) {
            payload.put("config", config);
        }

        return requestTracker.sendRequest(RequestType.REPOSITORY_CREATE, context, payload);
    }

    /**
     * Create a new repository with default configuration.
     *
     * @param repositoryId Repository identifier
     * @return CompletableFuture with created repository info
     */
    public CompletableFuture<Object> createRepository(String repositoryId) {
        return createRepository(repositoryId, null, null);
    }

    /**
     * Get repository information.
     *
     * @param repositoryId Repository identifier
     * @return CompletableFuture with repository info
     */
    public CompletableFuture<Object> getRepository(String repositoryId) {
        RequestContext context = RequestContext.builder()
                .tenantId(tenantId)
                .build();

        Map<String, Object> payload = new HashMap<>();
        payload.put("repository_id", repositoryId);

        return requestTracker.sendRequest(RequestType.REPOSITORY_GET, context, payload);
    }

    /**
     * List all repositories for the current tenant.
     *
     * @return CompletableFuture with list of repositories
     */
    public CompletableFuture<List<Object>> listRepositories() {
        RequestContext context = RequestContext.builder()
                .tenantId(tenantId)
                .build();

        return requestTracker.sendRequest(RequestType.REPOSITORY_LIST, context, Collections.emptyMap())
                .thenApply(result -> {
                    if (result instanceof List) {
                        return (List<Object>) result;
                    }
                    return Collections.emptyList();
                });
    }

    /**
     * Update repository configuration.
     *
     * @param repositoryId Repository identifier
     * @param description  Optional description
     * @param config       Optional repository configuration
     * @return CompletableFuture with result
     */
    public CompletableFuture<Object> updateRepository(String repositoryId, String description, Map<String, Object> config) {
        RequestContext context = RequestContext.builder()
                .tenantId(tenantId)
                .build();

        Map<String, Object> payload = new HashMap<>();
        payload.put("repository_id", repositoryId);
        if (description != null) {
            payload.put("description", description);
        }
        if (config != null) {
            payload.put("config", config);
        }

        return requestTracker.sendRequest(RequestType.REPOSITORY_UPDATE, context, payload);
    }

    /**
     * Delete a repository.
     *
     * @param repositoryId Repository identifier
     * @return CompletableFuture with result
     */
    public CompletableFuture<Object> deleteRepository(String repositoryId) {
        RequestContext context = RequestContext.builder()
                .tenantId(tenantId)
                .build();

        Map<String, Object> payload = new HashMap<>();
        payload.put("repository_id", repositoryId);

        return requestTracker.sendRequest(RequestType.REPOSITORY_DELETE, context, payload);
    }

    /**
     * Check if connected to the server.
     */
    public boolean isConnected() {
        return connection.isConnected();
    }

    /**
     * Check if authenticated.
     */
    public boolean isAuthenticated() {
        return authManager.isAuthenticated();
    }

    /**
     * Close the connection and clean up resources.
     */
    @Override
    public void close() {
        authManager.logout();
        connection.close();
        requestTracker.shutdown();
        scheduler.shutdown();
    }

    // Package-private getters for internal use
    RequestTracker getRequestTracker() {
        return requestTracker;
    }

    String getTenantId() {
        return tenantId;
    }
}
