#!/usr/bin/env bash
# Start a 3-node RaisinDB cluster with replication enabled
#
# This script starts 3 nodes with:
# - Unique cluster node IDs (required for operation capture)
# - Separate data directories
# - Different HTTP and replication ports
# - Replication enabled

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

echo -e "${BLUE}╔════════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║  RaisinDB 3-Node Cluster Startup Script   ║${NC}"
echo -e "${BLUE}╚════════════════════════════════════════════╝${NC}"
echo

# Check if cargo is available
if ! command -v cargo &> /dev/null; then
    echo -e "${RED}❌ Error: cargo not found. Please install Rust.${NC}"
    exit 1
fi

# Build the server in release mode
echo -e "${YELLOW}📦 Building raisin-server in release mode...${NC}"
cargo build --release --bin raisin-server
echo -e "${GREEN}✅ Build complete${NC}"
echo

# Clean up old data directories (optional - comment out to preserve data)
echo -e "${YELLOW}🧹 Cleaning up old data directories...${NC}"
rm -rf data/node1 data/node2 data/node3
mkdir -p data/node1 data/node2 data/node3
echo -e "${GREEN}✅ Data directories ready${NC}"
echo

# Kill any existing raisin-server processes
echo -e "${YELLOW}🛑 Stopping any existing raisin-server processes...${NC}"
pkill -f raisin-server || true
sleep 2
echo -e "${GREEN}✅ Cleanup complete${NC}"
echo

# Start Node 1
echo -e "${BLUE}🚀 Starting Node 1...${NC}"
RUST_LOG=info \
RAISIN_CLUSTER_NODE_ID=node1 \
RAISIN_REPLICATION_PORT=9001 \
RAISIN_PORT=3001 \
RAISIN_DATA_DIR=./data/node1 \
RAISIN_REPLICATION_PEERS="node2=127.0.0.1:9002,node3=127.0.0.1:9003" \
  ./target/release/raisin-server > logs/node1.log 2>&1 &
NODE1_PID=$!
echo -e "${GREEN}✅ Node 1 started (PID: $NODE1_PID, HTTP: 3001, Replication: 9001)${NC}"
echo

# Wait a moment for Node 1 to initialize
sleep 3

# Start Node 2
echo -e "${BLUE}🚀 Starting Node 2...${NC}"
RUST_LOG=info \
RAISIN_CLUSTER_NODE_ID=node2 \
RAISIN_REPLICATION_PORT=9002 \
RAISIN_PORT=3002 \
RAISIN_DATA_DIR=./data/node2 \
RAISIN_REPLICATION_PEERS="node1=127.0.0.1:9001,node3=127.0.0.1:9003" \
  ./target/release/raisin-server > logs/node2.log 2>&1 &
NODE2_PID=$!
echo -e "${GREEN}✅ Node 2 started (PID: $NODE2_PID, HTTP: 3002, Replication: 9002)${NC}"
echo

# Wait a moment for Node 2 to initialize
sleep 3

# Start Node 3
echo -e "${BLUE}🚀 Starting Node 3...${NC}"
RUST_LOG=info \
RAISIN_CLUSTER_NODE_ID=node3 \
RAISIN_REPLICATION_PORT=9003 \
RAISIN_PORT=3003 \
RAISIN_DATA_DIR=./data/node3 \
RAISIN_REPLICATION_PEERS="node1=127.0.0.1:9001,node2=127.0.0.1:9002" \
  ./target/release/raisin-server > logs/node3.log 2>&1 &
NODE3_PID=$!
echo -e "${GREEN}✅ Node 3 started (PID: $NODE3_PID, HTTP: 3003, Replication: 9003)${NC}"
echo

echo -e "${GREEN}╔════════════════════════════════════════════╗${NC}"
echo -e "${GREEN}║     Cluster started successfully! 🎉       ║${NC}"
echo -e "${GREEN}╚════════════════════════════════════════════╝${NC}"
echo
echo -e "${YELLOW}📝 Node Information:${NC}"
echo -e "  ${BLUE}Node 1:${NC} http://localhost:3001 (PID: $NODE1_PID)"
echo -e "  ${BLUE}Node 2:${NC} http://localhost:3002 (PID: $NODE2_PID)"
echo -e "  ${BLUE}Node 3:${NC} http://localhost:3003 (PID: $NODE3_PID)"
echo
echo -e "${YELLOW}📂 Data Directories:${NC}"
echo -e "  ${BLUE}Node 1:${NC} ./data/node1"
echo -e "  ${BLUE}Node 2:${NC} ./data/node2"
echo -e "  ${BLUE}Node 3:${NC} ./data/node3"
echo
echo -e "${YELLOW}📋 Log Files:${NC}"
echo -e "  ${BLUE}Node 1:${NC} ./logs/node1.log"
echo -e "  ${BLUE}Node 2:${NC} ./logs/node2.log"
echo -e "  ${BLUE}Node 3:${NC} ./logs/node3.log"
echo
echo -e "${YELLOW}🛠️  Useful Commands:${NC}"
echo -e "  ${BLUE}View Node 1 logs:${NC} tail -f logs/node1.log"
echo -e "  ${BLUE}View Node 2 logs:${NC} tail -f logs/node2.log"
echo -e "  ${BLUE}View Node 3 logs:${NC} tail -f logs/node3.log"
echo -e "  ${BLUE}Stop cluster:${NC} pkill -f raisin-server"
echo
echo -e "${YELLOW}⚠️  Important:${NC}"
echo -e "  Replication is ${GREEN}ENABLED${NC} with CLUSTER_NODE_ID set for each node"
echo -e "  Operations will now be captured and replicated across the cluster"
echo
