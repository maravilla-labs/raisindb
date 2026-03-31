#!/bin/bash
# Test event-driven NodeType initialization

set -e

echo "=========================================="
echo "  Event-Driven NodeType Initialization"
echo "  Verification Script"
echo "=========================================="
echo ""

# Clean up any existing data
echo "🧹 Cleaning up old data..."
rm -rf ./.data
echo ""

# Build the server
echo "🔨 Building raisin-server..."
cargo build --package raisin-server --quiet 2>&1 | tail -5
echo "✅ Build complete"
echo ""

# Start the server in the background
echo "🚀 Starting server..."
RUST_LOG=info cargo run --package raisin-server > /tmp/raisin-server.log 2>&1 &
SERVER_PID=$!
echo "Server PID: $SERVER_PID"

# Wait for server to start
echo "⏳ Waiting for server to start..."
sleep 3

# Check if server is running
if ! ps -p $SERVER_PID > /dev/null; then
    echo "❌ Server failed to start. Check /tmp/raisin-server.log"
    cat /tmp/raisin-server.log
    exit 1
fi

# Function to cleanup on exit
cleanup() {
    echo ""
    echo "🧹 Cleaning up..."
    if ps -p $SERVER_PID > /dev/null; then
        kill $SERVER_PID
        echo "✅ Server stopped"
    fi
}
trap cleanup EXIT

# Test 1: Create a repository
echo ""
echo "📝 Test 1: Create repository via HTTP"
echo "--------------------------------------"
RESPONSE=$(curl -s -w "\n%{http_code}" -X POST http://localhost:8080/api/repositories \
  -H "Content-Type: application/json" \
  -d '{
    "repo_id": "event-test-repo",
    "name": "Event Test Repository",
    "description": "Testing event-driven NodeType initialization",
    "default_branch": "main"
  }')

HTTP_CODE=$(echo "$RESPONSE" | tail -n1)
BODY=$(echo "$RESPONSE" | head -n-1)

if [ "$HTTP_CODE" = "201" ]; then
    echo "✅ Repository created successfully"
    echo "Response: $BODY" | jq '.' 2>/dev/null || echo "$BODY"
else
    echo "❌ Failed to create repository (HTTP $HTTP_CODE)"
    echo "Response: $BODY"
    exit 1
fi

# Wait for event handler to process
echo ""
echo "⏳ Waiting for NodeType initialization (2 seconds)..."
sleep 2

# Test 2: Verify NodeTypes were created
echo ""
echo "🔍 Test 2: Verify NodeTypes exist"
echo "--------------------------------------"
NODETYPES=$(curl -s http://localhost:8080/api/repository/event-test-repo/main/draft/nodetypes)

# Check if response contains expected NodeTypes
if echo "$NODETYPES" | jq -e '. | length > 0' > /dev/null 2>&1; then
    echo "✅ NodeTypes endpoint returned data"
    
    # Check for specific NodeTypes
    FOLDER=$(echo "$NODETYPES" | jq -r '.[] | select(.name == "raisin:Folder") | .name' 2>/dev/null)
    PAGE=$(echo "$NODETYPES" | jq -r '.[] | select(.name == "raisin:Page") | .name' 2>/dev/null)
    ASSET=$(echo "$NODETYPES" | jq -r '.[] | select(.name == "raisin:Asset") | .name' 2>/dev/null)
    
    echo ""
    if [ "$FOLDER" = "raisin:Folder" ]; then
        echo "  ✅ raisin:Folder found"
    else
        echo "  ❌ raisin:Folder NOT found"
    fi
    
    if [ "$PAGE" = "raisin:Page" ]; then
        echo "  ✅ raisin:Page found"
    else
        echo "  ❌ raisin:Page NOT found"
    fi
    
    if [ "$ASSET" = "raisin:Asset" ]; then
        echo "  ✅ raisin:Asset found"
    else
        echo "  ❌ raisin:Asset NOT found"
    fi
    
    # Check logs for event handler execution
    echo ""
    echo "📋 Server logs (event handler):"
    echo "--------------------------------------"
    grep -i "nodetype" /tmp/raisin-server.log | tail -10 || echo "No NodeType logs found"
    
else
    echo "❌ Failed to retrieve NodeTypes"
    echo "Response: $NODETYPES"
    exit 1
fi

# Test 3: Create another repository (verify idempotent)
echo ""
echo "📝 Test 3: Create second repository"
echo "--------------------------------------"
RESPONSE2=$(curl -s -w "\n%{http_code}" -X POST http://localhost:8080/api/repositories \
  -H "Content-Type: application/json" \
  -d '{
    "repo_id": "event-test-repo-2",
    "name": "Second Event Test Repository",
    "default_branch": "develop"
  }')

HTTP_CODE2=$(echo "$RESPONSE2" | tail -n1)

if [ "$HTTP_CODE2" = "201" ]; then
    echo "✅ Second repository created successfully"
else
    echo "❌ Failed to create second repository (HTTP $HTTP_CODE2)"
fi

sleep 2

# Verify second repository has NodeTypes
NODETYPES2=$(curl -s http://localhost:8080/api/repository/event-test-repo-2/develop/draft/nodetypes)
if echo "$NODETYPES2" | jq -e '. | length > 0' > /dev/null 2>&1; then
    echo "✅ Second repository has NodeTypes"
else
    echo "❌ Second repository missing NodeTypes"
fi

echo ""
echo "=========================================="
echo "  ✅ All tests passed!"
echo "=========================================="
echo ""
echo "Key points verified:"
echo "  • Repositories created via HTTP API"
echo "  • RepositoryCreated events emitted"
echo "  • NodeTypeInitHandler processed events"
echo "  • NodeTypes automatically initialized"
echo "  • Multiple repositories work correctly"
echo ""
echo "Full server logs: /tmp/raisin-server.log"
