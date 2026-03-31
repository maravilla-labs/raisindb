"""Basic usage example for RaisinDB Python client."""

import asyncio
import logging

from raisindb import RaisinClient

# Enable logging
logging.basicConfig(
    level=logging.INFO, format="%(asctime)s - %(name)s - %(levelname)s - %(message)s"
)


async def main() -> None:
    """Main example function."""
    # Create and connect client
    client = RaisinClient("raisin://localhost:8080/sys/default")

    try:
        await client.connect()
        print("✓ Connected to RaisinDB")

        # Authenticate
        await client.authenticate("admin", "admin")
        print("✓ Authenticated")

        # Get database and workspace
        db = client.database("demo")
        workspace = db.workspace("content")

        # Create a node
        print("\nCreating a node...")
        node = await workspace.nodes().create(
            node_type="Page",
            path="/welcome",
            properties={"title": "Welcome Page", "published": True, "author": "admin"},
            content={"body": "Welcome to RaisinDB!"},
        )
        print(f"✓ Created node: {node.node_id}")
        print(f"  Path: {node.path}")
        print(f"  Properties: {node.properties}")

        # Query nodes by path
        print("\nQuerying nodes...")
        nodes = await workspace.nodes().query_by_path("/welcome")
        print(f"✓ Found {len(nodes)} node(s)")
        for n in nodes:
            print(f"  - {n.path}: {n.properties.get('title')}")

        # Update the node
        print("\nUpdating node...")
        updated_node = await workspace.nodes().update(
            node_id=node.node_id,
            properties={"title": "Updated Welcome Page", "published": True},
        )
        print(f"✓ Updated node: {updated_node.node_id}")
        print(f"  New title: {updated_node.properties.get('title')}")

        # Execute SQL query
        print("\nExecuting SQL query...")
        result = await db.sql("SELECT * FROM nodes WHERE node_type = ?", "Page")
        print(f"✓ Query returned {result.row_count} row(s)")
        print(f"  Columns: {result.columns}")
        for row in result.rows:
            print(f"  Row: {row}")

        # List workspaces
        print("\nListing workspaces...")
        workspaces = await db.list_workspaces()
        print(f"✓ Found {len(workspaces)} workspace(s)")
        for ws in workspaces:
            print(f"  - {ws}")

        # Subscribe to events
        print("\nSubscribing to events...")

        event_count = 0

        async def on_event(event):
            nonlocal event_count
            event_count += 1
            print(f"\n📢 Event received: {event.event_type}")
            print(f"   Subscription: {event.subscription_id}")
            print(f"   Timestamp: {event.timestamp}")
            print(f"   Payload: {event.payload}")

        subscription = await workspace.events().subscribe(
            callback=on_event, path="/", event_types=["node:created", "node:updated"]
        )
        print(f"✓ Subscribed with ID: {subscription.subscription_id}")

        # Create another node to trigger event
        print("\nCreating another node (should trigger event)...")
        node2 = await workspace.nodes().create(
            node_type="Page",
            path="/test",
            properties={"title": "Test Page"},
        )
        print(f"✓ Created node: {node2.node_id}")

        # Wait a bit for events
        await asyncio.sleep(1)

        # Unsubscribe
        await subscription.unsubscribe()
        print(f"✓ Unsubscribed (received {event_count} event(s))")

        # Clean up - delete test nodes
        print("\nCleaning up...")
        await workspace.nodes().delete(node.node_id)
        await workspace.nodes().delete(node2.node_id)
        print("✓ Deleted test nodes")

    except Exception as e:
        print(f"✗ Error: {e}")
        raise

    finally:
        # Close connection
        await client.close()
        print("\n✓ Connection closed")


if __name__ == "__main__":
    asyncio.run(main())
