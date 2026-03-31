# RaisinDB 3-Node Cluster Example

This directory contains example configuration files for running a 3-node RaisinDB cluster with full mesh replication.

## Configuration Files

- `node1.toml` - Configuration for Node 1 (HTTP: 8081, Replication: 9001)
- `node2.toml` - Configuration for Node 2 (HTTP: 8082, Replication: 9002)
- `node3.toml` - Configuration for Node 3 (HTTP: 8083, Replication: 9003)

## Starting the Cluster

### Option 1: Using TOML Configuration Files

```bash
# Terminal 1 - Start Node 1
./target/release/raisin-server --config examples/cluster/node1.toml

# Terminal 2 - Start Node 2
./target/release/raisin-server --config examples/cluster/node2.toml

# Terminal 3 - Start Node 3
./target/release/raisin-server --config examples/cluster/node3.toml
```

### Option 2: Using CLI Arguments

```bash
# Terminal 1 - Start Node 1
./target/release/raisin-server \
  --port 8081 \
  --data-dir ./data/node1 \
  --cluster-node-id node1 \
  --replication-port 9001 \
  --replication-peers "node2=127.0.0.1:9002,node3=127.0.0.1:9003" \
  --initial-admin-password "admin123"

# Terminal 2 - Start Node 2
./target/release/raisin-server \
  --port 8082 \
  --data-dir ./data/node2 \
  --cluster-node-id node2 \
  --replication-port 9002 \
  --replication-peers "node1=127.0.0.1:9001,node3=127.0.0.1:9003" \
  --initial-admin-password "admin123"

# Terminal 3 - Start Node 3
./target/release/raisin-server \
  --port 8083 \
  --data-dir ./data/node3 \
  --cluster-node-id node3 \
  --replication-port 9003 \
  --replication-peers "node1=127.0.0.1:9001,node2=127.0.0.1:9002" \
  --initial-admin-password "admin123"
```

### Option 3: Using Environment Variables

```bash
# Terminal 1 - Node 1
export RAISIN_PORT=8081
export RAISIN_DATA_DIR=./data/node1
export RAISIN_CLUSTER_NODE_ID=node1
export RAISIN_REPLICATION_PORT=9001
export RAISIN_REPLICATION_PEERS="node2=127.0.0.1:9002,node3=127.0.0.1:9003"
export RAISIN_ADMIN_PASSWORD="admin123"
./target/release/raisin-server

# Terminal 2 - Node 2
export RAISIN_PORT=8082
export RAISIN_DATA_DIR=./data/node2
export RAISIN_CLUSTER_NODE_ID=node2
export RAISIN_REPLICATION_PORT=9002
export RAISIN_REPLICATION_PEERS="node1=127.0.0.1:9001,node3=127.0.0.1:9003"
export RAISIN_ADMIN_PASSWORD="admin123"
./target/release/raisin-server

# Terminal 3 - Node 3
export RAISIN_PORT=8083
export RAISIN_DATA_DIR=./data/node3
export RAISIN_CLUSTER_NODE_ID=node3
export RAISIN_REPLICATION_PORT=9003
export RAISIN_REPLICATION_PEERS="node1=127.0.0.1:9001,node2=127.0.0.1:9002"
export RAISIN_ADMIN_PASSWORD="admin123"
./target/release/raisin-server
```

## Testing the Cluster

### 1. Check Health of All Nodes

```bash
curl http://localhost:8081/management/health
curl http://localhost:8082/management/health
curl http://localhost:8083/management/health
```

### 2. Authenticate to Node 1

```bash
curl -X POST http://localhost:8081/api/raisindb/sys/default/auth \
  -H "Content-Type: application/json" \
  -d '{
    "username": "admin",
    "password": "admin123!@#",
    "interface": "console"
  }'
```

Save the returned `token` for subsequent requests.

### 3. Create a Document on Node 1

```bash
TOKEN="your-token-from-step-2"

curl -X POST "http://localhost:8081/api/repository/workspace/main/head/workspace/" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "node": {
      "id": "test-doc-1",
      "name": "Test Document",
      "node_type": "Document",
      "properties": {
        "title": "Hello from Node 1",
        "created_on": "node1"
      }
    }
  }'
```

### 4. Verify Replication to Node 2

Wait a few seconds for replication, then:

```bash
# Authenticate to Node 2
curl -X POST http://localhost:8082/api/raisindb/sys/default/auth \
  -H "Content-Type: application/json" \
  -d '{
    "username": "admin",
    "password": "admin123!@#",
    "interface": "console"
  }'

# Get the document (use token from Node 2 auth)
TOKEN2="token-from-node2"
curl "http://localhost:8082/api/repository/workspace/main/head/workspace/\$ref/test-doc-1" \
  -H "Authorization: Bearer $TOKEN2"
```

You should see the same document that was created on Node 1!

### 5. Verify Replication to Node 3

```bash
# Authenticate to Node 3
curl -X POST http://localhost:8083/api/raisindb/sys/default/auth \
  -H "Content-Type: application/json" \
  -d '{
    "username": "admin",
    "password": "admin123!@#",
    "interface": "console"
  }'

# Get the document (use token from Node 3 auth)
TOKEN3="token-from-node3"
curl "http://localhost:8083/api/repository/workspace/main/head/workspace/\$ref/test-doc-1" \
  -H "Authorization: Bearer $TOKEN3"
```

## Configuration Priority

When using multiple configuration sources, the priority is:

1. **CLI Arguments** (highest priority)
2. **TOML Configuration File**
3. **Environment Variables**
4. **Default Values** (lowest priority)

This allows you to:
- Define base configuration in TOML files
- Override specific settings via environment variables
- Override everything via CLI arguments for testing

## Security Notes

⚠️ **IMPORTANT**: The example configurations use a simple password (`admin123!@#`) for demonstration purposes.

**For production deployments**:
1. Use strong, unique passwords for each environment
2. Store passwords in secure secret management systems
3. Use HTTPS/TLS for all communication
4. Configure firewall rules to restrict replication ports
5. Regularly rotate admin passwords
6. Enable authentication on WebSocket connections

## Stopping the Cluster

Press `Ctrl+C` in each terminal to gracefully shut down each node.

To clean up data:
```bash
rm -rf ./data/node1 ./data/node2 ./data/node3
```
