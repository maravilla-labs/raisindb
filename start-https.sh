#!/bin/bash
# Start RaisinDB with HTTPS via Caddy reverse proxy
#
# Usage: ./start-https.sh
#
# This starts:
# - Caddy reverse proxy on :8443 → :8081 (self-signed cert)
# - RaisinDB server on :8081
#
# Access via: https://YOUR_IP:8443

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Change to project root
cd "$(dirname "$0")"

# Check if caddy is installed
if ! command -v caddy &> /dev/null; then
    echo -e "${RED}Error: Caddy is not installed${NC}"
    echo "Install with: brew install caddy"
    exit 1
fi

# Check if server binary exists
SERVER_BIN="./target/release/raisin-server"
if [ ! -f "$SERVER_BIN" ]; then
    echo -e "${YELLOW}Server binary not found. Building...${NC}"
    cargo build --release --package raisin-server --features "storage-rocksdb,websocket,pgwire"
fi

# Check if certs exist, generate if not
if [ ! -f "./certs/cert.pem" ] || [ ! -f "./certs/key.pem" ]; then
    echo -e "${YELLOW}Generating self-signed certificates...${NC}"
    mkdir -p ./certs
    LOCAL_IP=$(ipconfig getifaddr en0 2>/dev/null || hostname -I 2>/dev/null | awk '{print $1}' || echo "127.0.0.1")
    openssl req -x509 -newkey rsa:4096 -keyout ./certs/key.pem -out ./certs/cert.pem \
        -days 365 -nodes -subj "/CN=${LOCAL_IP}" \
        -addext "subjectAltName=IP:${LOCAL_IP},IP:127.0.0.1,DNS:localhost" 2>/dev/null
    echo -e "${GREEN}Certificates generated for ${LOCAL_IP}${NC}"
fi

# Get local IP for display
LOCAL_IP=$(ipconfig getifaddr en0 2>/dev/null || hostname -I 2>/dev/null | awk '{print $1}' || echo "localhost")

echo -e "${GREEN}Starting RaisinDB with HTTPS...${NC}"
echo ""
echo -e "RaisinDB: http://localhost:8081"
echo -e "HTTPS:    https://${LOCAL_IP}:8443"
echo -e "WSS:      wss://${LOCAL_IP}:8443"
echo -e "Admin:    https://${LOCAL_IP}:8443/admin"
echo ""
echo -e "${YELLOW}Press Ctrl+C to stop both services${NC}"
echo ""

# Cleanup function to kill both processes
cleanup() {
    echo ""
    echo -e "${YELLOW}Shutting down...${NC}"
    trap - SIGINT SIGTERM  # Prevent re-entry
    kill $SERVER_PID 2>/dev/null || true
    kill $CADDY_PID 2>/dev/null || true
    # Kill any remaining child processes
    pkill -P $$ 2>/dev/null || true
    wait 2>/dev/null
    exit 0
}

trap cleanup SIGINT SIGTERM

# Start Caddy in background
echo -e "${GREEN}Starting Caddy reverse proxy (:8443 → :8081)...${NC}"
caddy run --config Caddyfile > >(sed 's/^/[caddy] /') 2>&1 &
CADDY_PID=$!

# Give Caddy a moment to start
sleep 1

# Start RaisinDB server (foreground, logs to output.log and stdout)
echo -e "${GREEN}Starting RaisinDB server...${NC}"
echo ""
RUST_LOG=info $SERVER_BIN --config examples/cluster/node1.toml --pgwire-enabled true --bind-address 0.0.0.0 --dev-mode > >(tee output.log) 2>&1 &
SERVER_PID=$!

# Wait for either process to exit
wait $SERVER_PID $CADDY_PID
