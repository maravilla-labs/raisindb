# RaisinDB Java Client

Official Java client library for [RaisinDB](https://github.com/raisindb/raisindb) - A Git-like document database with branching, workspaces, and real-time events.

## Features

- **CompletableFuture API** - Modern async Java with CompletableFuture for non-blocking operations
- **Auto-Reconnect** - Automatic reconnection with exponential backoff
- **JWT Authentication** - Secure token-based authentication with automatic refresh
- **Fluent API** - Intuitive chainable API: `client.database(name).workspace(name).nodes()`
- **SQL Support** - Execute SQL queries with parameter binding for safety
- **Real-time Events** - Subscribe to database events with flexible filtering
- **WebSocket Transport** - Efficient binary communication using MessagePack
- **Thread Pool** - Configurable thread pool for concurrent operations

## Requirements

- Java 11 or higher
- Maven 3.6+ or Gradle 7+

## Installation

### Maven

Add to your `pom.xml`:

```xml
<dependency>
    <groupId>com.raisindb</groupId>
    <artifactId>raisin-client</artifactId>
    <version>0.1.0</version>
</dependency>
```

### Gradle

Add to your `build.gradle`:

```gradle
dependencies {
    implementation 'com.raisindb:raisin-client:0.1.0'
}
```

## Quick Start

```java
import com.raisindb.client.RaisinClient;
import com.raisindb.client.operations.*;
import com.raisindb.client.protocol.Node;

public class Example {
    public static void main(String[] args) throws Exception {
        // Connect to RaisinDB
        RaisinClient client = new RaisinClient("raisin://localhost:8080/sys/default");
        client.connect();

        // Authenticate
        client.authenticate("admin", "password").get();

        // Get a database and workspace
        Database db = client.database("my_repo");
        Workspace workspace = db.workspace("content");

        // Create a node
        Map<String, Object> properties = new HashMap<>();
        properties.put("title", "Home Page");
        properties.put("published", true);

        Node node = workspace.nodes().create(
            "Page",
            "/home",
            properties,
            Map.of("body", "Welcome to my site!")
        ).get();

        System.out.println("Created node: " + node.getNodeId());

        // Query nodes
        List<Node> nodes = workspace.nodes().queryByPath("/blog/*").get();
        for (Node n : nodes) {
            System.out.println("Found: " + n.getPath() + " - " +
                             n.getProperties().get("title"));
        }

        // Close connection
        client.close();
    }
}
```

## Using try-with-resources

```java
try (RaisinClient client = new RaisinClient("raisin://localhost:8080/sys/default")) {
    client.connect();
    client.authenticate("admin", "password").get();

    Database db = client.database("my_repo");
    Workspace workspace = db.workspace("content");

    // Your operations here
    List<Node> nodes = workspace.nodes().queryByPath("/").get();
}
```

## API Reference

### Client

```java
// Initialize client
RaisinClient client = new RaisinClient("raisin://localhost:8080/sys/default");

// Custom configuration
RaisinClient client = new RaisinClient(
    "raisin://localhost:8080/sys/default",
    "default",                    // tenant ID
    tokenStorage,                 // custom token storage
    1000,                         // initial reconnect delay (ms)
    30000,                        // max reconnect delay (ms)
    null,                         // max reconnect attempts (null = unlimited)
    30000                         // request timeout (ms)
);

// Connect and authenticate
client.connect();
client.authenticate("username", "password").get();

// Check connection status
if (client.isConnected() && client.isAuthenticated()) {
    System.out.println("Ready!");
}

// Close connection
client.close();
```

### Database Operations

```java
// Get a database interface
Database db = client.database("my_repo");

// List workspaces
List<Object> workspaces = db.listWorkspaces().get();

// Create a workspace
Object workspaceInfo = db.createWorkspace("content", "Content workspace").get();

// Execute SQL queries
SqlResult result = db.sql(
    "SELECT * FROM nodes WHERE node_type = ? AND published = ?",
    "Page",
    true
).get();

for (List<Object> row : result.getRows()) {
    System.out.println(row);
}
```

### Workspace Operations

```java
// Get a workspace
Workspace workspace = db.workspace("content");

// Get workspace info
Object info = workspace.getInfo().get();

// Update workspace
workspace.update(
    "Updated description",
    List.of("Page", "Post", "Media"),  // allowed node types
    null                                // allowed root node types
).get();

// Work with different branches
Workspace mainWorkspace = workspace.onBranch("main");
Workspace devWorkspace = workspace.onBranch("dev");
```

### Node Operations

```java
NodeOperations nodes = workspace.nodes();

// Create a node
Map<String, Object> properties = new HashMap<>();
properties.put("title", "About Us");
properties.put("published", true);

Node node = nodes.create(
    "Page",
    "/about",
    properties,
    Map.of("body", "About our company...")
).get();

// Get a node by ID
Node node = nodes.get(nodeId).get();

// Update a node
Node updatedNode = nodes.update(
    node.getNodeId(),
    Map.of("title", "Updated Title"),
    Map.of("body", "New content...")
).get();

// Delete a node
nodes.delete(nodeId).get();

// Query by path (with wildcards)
List<Node> blogPosts = nodes.queryByPath("/blog/*").get();
List<Node> allContent = nodes.queryByPath("/blog/**").get();  // Recursive

// Query by property
List<Node> published = nodes.queryByProperty("published", true).get();
```

### SQL Queries

```java
// Simple query
SqlResult result = db.sql("SELECT * FROM nodes LIMIT 10").get();

// Parameterized query (recommended for safety)
SqlResult result = db.sql(
    "SELECT * FROM nodes WHERE node_type = ? AND created_at > ?",
    "Page",
    "2024-01-01"
).get();

// Access results
System.out.println("Columns: " + result.getColumns());
System.out.println("Row count: " + result.getRowCount());
for (List<Object> row : result.getRows()) {
    System.out.println(row);
}
```

### Event Subscriptions

```java
// Define event handler
Consumer<EventMessage> onEvent = event -> {
    System.out.println("Event: " + event.getEventType());
    System.out.println("Payload: " + event.getPayload());
    System.out.println("Timestamp: " + event.getTimestamp());
};

// Subscribe to all events in a path
Subscription subscription = workspace.events().subscribe(
    onEvent,
    "/blog/*",
    List.of("node:created", "node:updated"),
    null
).get();

// Subscribe to specific event types
Subscription subscription = workspace.events()
    .subscribeToType("node:created", onEvent).get();

// Subscribe to specific node types
Subscription subscription = workspace.events()
    .subscribeToNodeType("Page", onEvent).get();

// Unsubscribe
subscription.unsubscribe().get();
```

## Advanced Usage

### Custom Token Storage

Implement custom token storage for persistent authentication:

```java
public class FileTokenStorage implements TokenStorage {
    private final String filePath;

    public FileTokenStorage(String filePath) {
        this.filePath = filePath;
    }

    @Override
    public String[] getTokens() {
        try {
            String content = Files.readString(Paths.get(filePath));
            return content.split(",");
        } catch (IOException e) {
            return null;
        }
    }

    @Override
    public void setTokens(String accessToken, String refreshToken) {
        try {
            Files.writeString(Paths.get(filePath),
                            accessToken + "," + refreshToken);
        } catch (IOException e) {
            throw new RuntimeException(e);
        }
    }

    @Override
    public void clearTokens() {
        try {
            Files.deleteIfExists(Paths.get(filePath));
        } catch (IOException e) {
            throw new RuntimeException(e);
        }
    }
}

// Use custom storage
TokenStorage storage = new FileTokenStorage(".raisin_tokens");
RaisinClient client = new RaisinClient(
    "raisin://localhost:8080/sys/default",
    null,
    storage,
    1000, 30000, null, 30000
);
```

### Error Handling

```java
import com.raisindb.client.exceptions.*;

try {
    client.connect();
    client.authenticate("admin", "wrong_password").get();
} catch (AuthenticationException e) {
    System.err.println("Auth failed: " + e.getMessage());
} catch (ConnectionException e) {
    System.err.println("Connection failed: " + e.getMessage());
} catch (RequestException e) {
    System.err.println("Request failed: " + e.getCode() + " - " + e.getMessage());
    System.err.println("Details: " + e.getDetails());
} catch (TimeoutException e) {
    System.err.println("Request timed out: " + e.getMessage());
} catch (RaisinDBException e) {
    System.err.println("General error: " + e.getMessage());
}
```

### Concurrent Operations

```java
// Run multiple operations concurrently
CompletableFuture<Node> future1 = workspace.nodes().get(nodeId1);
CompletableFuture<Node> future2 = workspace.nodes().get(nodeId2);
CompletableFuture<Node> future3 = workspace.nodes().get(nodeId3);

CompletableFuture.allOf(future1, future2, future3).get();
List<Node> nodes = List.of(future1.get(), future2.get(), future3.get());

// Create multiple nodes concurrently
List<CompletableFuture<Node>> futures = new ArrayList<>();
for (int i = 0; i < 10; i++) {
    int index = i;
    futures.add(workspace.nodes().create(
        "Page",
        "/page-" + index,
        Map.of("title", "Page " + index),
        null
    ));
}

List<Node> createdNodes = futures.stream()
    .map(CompletableFuture::join)
    .collect(Collectors.toList());
```

## Building from Source

```bash
git clone https://github.com/raisindb/raisindb.git
cd raisindb/packages/raisin-client-java

# Build with Maven
mvn clean install

# Run tests
mvn test

# Generate JavaDoc
mvn javadoc:javadoc
```

## Logging

The client uses SLF4J for logging. Configure your logging framework:

### Logback (logback.xml)

```xml
<configuration>
    <appender name="STDOUT" class="ch.qos.logback.core.ConsoleAppender">
        <encoder>
            <pattern>%d{HH:mm:ss.SSS} [%thread] %-5level %logger{36} - %msg%n</pattern>
        </encoder>
    </appender>

    <logger name="com.raisindb.client" level="DEBUG"/>
    <logger name="jakarta.websocket" level="INFO"/>

    <root level="INFO">
        <appender-ref ref="STDOUT"/>
    </root>
</configuration>
```

## License

MIT License - see [LICENSE](LICENSE) for details.

## Links

- [RaisinDB Repository](https://github.com/raisindb/raisindb)
- [Documentation](https://raisindb.com/docs)
- [JavaDoc](https://raisindb.com/docs/java)
- [Issue Tracker](https://github.com/raisindb/raisindb/issues)

## Support

For questions and support:

- GitHub Issues: https://github.com/raisindb/raisindb/issues
- Discord: https://discord.gg/raisindb
- Email: support@raisindb.com
