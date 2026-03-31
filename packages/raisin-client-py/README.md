# RaisinDB Python Client

Official Python client for [RaisinDB](https://github.com/raisindb/raisindb) - A Git-like document database with branching, workspaces, and real-time events.

## Features

- **Async/Await API** - Built on asyncio for high-performance concurrent operations
- **Auto-Reconnect** - Automatic reconnection with exponential backoff
- **JWT Authentication** - Secure token-based authentication with automatic refresh
- **Type Hints** - Full type hints for better IDE support and type checking
- **Fluent API** - Intuitive chainable API: `client.database(name).workspace(name).nodes()`
- **SQL Support** - Execute SQL queries with parameter binding for safety
- **Real-time Events** - Subscribe to database events with flexible filtering
- **Connection Pooling** - Efficient resource management

## Installation

```bash
pip install raisindb-client
```

For development:

```bash
pip install raisindb-client[dev]
```

## Quick Start

```python
import asyncio
from raisindb import RaisinClient

async def main():
    # Connect to RaisinDB
    client = RaisinClient("raisin://localhost:8080/sys/default")
    await client.connect()

    # Authenticate
    await client.authenticate("admin", "password")

    # Get a database and workspace
    db = client.database("my_repo")
    workspace = db.workspace("content")

    # Create a node
    node = await workspace.nodes().create(
        node_type="Page",
        path="/home",
        properties={"title": "Home Page", "published": True},
        content={"body": "Welcome to my site!"}
    )
    print(f"Created node: {node.node_id}")

    # Query nodes
    nodes = await workspace.nodes().query_by_path("/blog/*")
    for node in nodes:
        print(f"Found: {node.path} - {node.properties.get('title')}")

    # Close connection
    await client.close()

if __name__ == "__main__":
    asyncio.run(main())
```

## Using Async Context Manager

```python
async with RaisinClient("raisin://localhost:8080/sys/default") as client:
    await client.authenticate("admin", "password")

    db = client.database("my_repo")
    workspace = db.workspace("content")

    # Your operations here
    nodes = await workspace.nodes().query_by_path("/")
```

## API Reference

### Client

```python
# Initialize client
client = RaisinClient(
    url="raisin://localhost:8080/sys/default",
    tenant_id="default",  # Optional, extracted from URL if not provided
    request_timeout=30.0,  # Default request timeout in seconds
    initial_reconnect_delay=1.0,  # Initial reconnect delay
    max_reconnect_delay=30.0,  # Maximum reconnect delay
    max_reconnect_attempts=None,  # None = unlimited
)

# Connect and authenticate
await client.connect()
await client.authenticate("username", "password")

# Check connection status
if client.is_connected and client.is_authenticated:
    print("Ready!")

# Close connection
await client.close()
```

### Database Operations

```python
# Get a database interface
db = client.database("my_repo")

# List workspaces
workspaces = await db.list_workspaces()

# Create a workspace
workspace_info = await db.create_workspace(
    name="content",
    description="Content workspace"
)

# Execute SQL queries
result = await db.sql(
    "SELECT * FROM nodes WHERE node_type = ? AND published = ?",
    "Page",
    True
)

for row in result.rows:
    print(row)
```

### Workspace Operations

```python
# Get a workspace
workspace = db.workspace("content")

# Get workspace info
info = await workspace.get_info()
print(f"Workspace: {info.name}, created: {info.created_at}")

# Update workspace
await workspace.update(
    description="Updated description",
    allowed_node_types=["Page", "Post", "Media"]
)

# Work with different branches
main_workspace = workspace.on_branch("main")
dev_workspace = workspace.on_branch("dev")
```

### Node Operations

```python
nodes = workspace.nodes()

# Create a node
node = await nodes.create(
    node_type="Page",
    path="/about",
    properties={
        "title": "About Us",
        "published": True,
        "author": "john@example.com"
    },
    content={"body": "About our company..."}
)

# Get a node by ID
node = await nodes.get(node_id)

# Update a node
updated_node = await nodes.update(
    node_id=node.node_id,
    properties={"title": "Updated Title"},
    content={"body": "New content..."}
)

# Delete a node
await nodes.delete(node_id)

# Query nodes
all_pages = await nodes.query({
    "node_type": "Page",
    "properties.published": True
})

# Query by path (with wildcards)
blog_posts = await nodes.query_by_path("/blog/*")
all_content = await nodes.query_by_path("/blog/**")  # Recursive

# Query by property
published = await nodes.query_by_property("published", True)
```

### SQL Queries

```python
# Simple query
result = await db.sql("SELECT * FROM nodes LIMIT 10")

# Parameterized query (recommended for safety)
result = await db.sql(
    "SELECT * FROM nodes WHERE node_type = ? AND created_at > ?",
    "Page",
    "2024-01-01"
)

# Access results
print(f"Columns: {result.columns}")
print(f"Row count: {result.row_count}")
for row in result.rows:
    print(row)
```

### Event Subscriptions

```python
# Define event handler
async def on_event(event):
    print(f"Event: {event.event_type}")
    print(f"Payload: {event.payload}")
    print(f"Timestamp: {event.timestamp}")

# Subscribe to all events in a path
subscription = await workspace.events().subscribe(
    callback=on_event,
    path="/blog/*",
    event_types=["node:created", "node:updated"]
)

# Subscribe to specific event types
subscription = await workspace.events().subscribe_to_type(
    "node:created",
    on_event
)

# Subscribe to specific node types
subscription = await workspace.events().subscribe_to_node_type(
    "Page",
    on_event
)

# Unsubscribe
await subscription.unsubscribe()
```

## Advanced Usage

### Custom Token Storage

Implement custom token storage for persistent authentication:

```python
from raisindb.auth import TokenStorage

class FileTokenStorage(TokenStorage):
    def __init__(self, file_path: str):
        self.file_path = file_path

    async def get_tokens(self):
        try:
            with open(self.file_path, 'r') as f:
                data = json.load(f)
                return (data['access_token'], data['refresh_token'])
        except FileNotFoundError:
            return None

    async def set_tokens(self, access_token: str, refresh_token: str):
        with open(self.file_path, 'w') as f:
            json.dump({
                'access_token': access_token,
                'refresh_token': refresh_token
            }, f)

    async def clear_tokens(self):
        if os.path.exists(self.file_path):
            os.remove(self.file_path)

# Use custom storage
storage = FileTokenStorage(".raisin_tokens")
client = RaisinClient(
    "raisin://localhost:8080/sys/default",
    token_storage=storage
)
```

### Error Handling

```python
from raisindb.exceptions import (
    RaisinDBError,
    ConnectionError,
    AuthenticationError,
    RequestError,
    TimeoutError
)

try:
    await client.connect()
    await client.authenticate("admin", "wrong_password")
except AuthenticationError as e:
    print(f"Auth failed: {e}")
except ConnectionError as e:
    print(f"Connection failed: {e}")
except RequestError as e:
    print(f"Request failed: {e.code} - {e}")
    print(f"Details: {e.details}")
except TimeoutError as e:
    print(f"Request timed out: {e}")
except RaisinDBError as e:
    print(f"General error: {e}")
```

### Concurrent Operations

```python
# Run multiple operations concurrently
results = await asyncio.gather(
    workspace.nodes().get(node_id_1),
    workspace.nodes().get(node_id_2),
    workspace.nodes().get(node_id_3),
)

# Create multiple nodes concurrently
creates = [
    workspace.nodes().create("Page", f"/page-{i}", {"title": f"Page {i}"})
    for i in range(10)
]
created_nodes = await asyncio.gather(*creates)
```

## Connection URL Format

The client supports the following URL formats:

```python
# Standard format
"raisin://localhost:8080/sys/{tenant_id}"
"raisin://localhost:8080/sys/{tenant_id}/{repository}"

# Direct WebSocket URLs also work
"ws://localhost:8080/ws"
"wss://secure.example.com:443/ws"
```

## Configuration

### Environment Variables

You can configure the client using environment variables:

```bash
export RAISIN_URL="raisin://localhost:8080/sys/default"
export RAISIN_USERNAME="admin"
export RAISIN_PASSWORD="password"
```

```python
import os

client = RaisinClient(os.getenv("RAISIN_URL"))
await client.connect()
await client.authenticate(
    os.getenv("RAISIN_USERNAME"),
    os.getenv("RAISIN_PASSWORD")
)
```

### Logging

Enable detailed logging:

```python
import logging

logging.basicConfig(
    level=logging.DEBUG,
    format='%(asctime)s - %(name)s - %(levelname)s - %(message)s'
)

# Or configure specific loggers
logging.getLogger('raisindb').setLevel(logging.DEBUG)
logging.getLogger('websockets').setLevel(logging.INFO)
```

## Requirements

- Python 3.9+
- websockets
- msgpack
- typing-extensions

## Development

### Setup

```bash
git clone https://github.com/raisindb/raisindb.git
cd raisindb/packages/raisin-client-py

# Create virtual environment
python -m venv venv
source venv/bin/activate  # On Windows: venv\Scripts\activate

# Install dependencies
pip install -e ".[dev]"
```

### Running Tests

```bash
pytest
pytest --cov=raisindb --cov-report=html
```

### Code Quality

```bash
# Format code
black raisindb

# Lint
ruff check raisindb

# Type check
mypy raisindb
```

## Examples

See the [examples/](examples/) directory for complete examples:

- Basic CRUD operations
- SQL queries
- Event subscriptions
- Branch management
- Custom authentication

## License

MIT License - see [LICENSE](LICENSE) for details.

## Links

- [RaisinDB Repository](https://github.com/raisindb/raisindb)
- [Documentation](https://raisindb.com/docs)
- [Issue Tracker](https://github.com/raisindb/raisindb/issues)
- [Changelog](CHANGELOG.md)

## Support

For questions and support:

- GitHub Issues: https://github.com/raisindb/raisindb/issues
- Discord: https://discord.gg/raisindb
- Email: support@raisindb.com
