# Standalone Server

RaisinDB includes a reference HTTP server implementation (`raisin-server`) that you can run standalone.

## Running the Server

```bash
# Clone the repository
git clone https://github.com/yourusername/raisindb
cd raisindb

# Run the server (with default features: storage-rocksdb, websocket, pgwire, ai)
cargo run --bin raisin-server
```

The server will start on `http://localhost:8080` by default.

## Configuration

The server uses a TOML configuration file and CLI arguments. You can also use the `--config` flag:

```bash
# Run with a config file
cargo run --bin raisin-server -- --config examples/cluster/node1.toml

# Or use environment variables for logging
RUST_LOG=info cargo run --bin raisin-server
```

## API Endpoints

### Health Check

```bash
curl http://localhost:8080/health
```

### Create a Node

```bash
curl -X POST http://localhost:8080/api/repository/default/ \
  -H "Content-Type: application/json" \
  -d '{
    "name": "my-page",
    "node_type": "raisin:Folder",
    "properties": {}
  }'
```

### List Nodes

```bash
curl http://localhost:8080/api/repository/default/
```

### Get Node by ID

```bash
curl http://localhost:8080/api/repository/default/\$ref/NODE_ID
```

## Production Deployment

For production use, consider:

1. Using a reverse proxy (nginx, Caddy)
2. Enabling TLS/SSL
3. Setting up monitoring
4. Configuring backups for RocksDB data

See the [raisin-server documentation](../../crates/raisin-server/) for more details.
