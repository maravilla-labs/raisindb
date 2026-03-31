"""Event subscription examples for RaisinDB Python client."""

import asyncio
import logging

from raisindb import RaisinClient
from raisindb.protocol import EventMessage

logging.basicConfig(level=logging.INFO)


async def main() -> None:
    """Event subscription examples."""
    client = RaisinClient("raisin://localhost:8080/sys/default")

    try:
        await client.connect()
        await client.authenticate("admin", "admin")

        db = client.database("demo")
        workspace = db.workspace("content")

        # Example 1: Subscribe to all events on a path
        print("Example 1: Subscribe to path /blog/*")

        async def on_blog_event(event: EventMessage):
            print(f"  📢 Blog event: {event.event_type} at {event.payload.get('path')}")

        sub1 = await workspace.events().subscribe_to_path("/blog/*", on_blog_event)

        # Create some blog nodes
        for i in range(3):
            await workspace.nodes().create(
                node_type="Post", path=f"/blog/post-{i}", properties={"title": f"Post {i}"}
            )

        await asyncio.sleep(0.5)
        await sub1.unsubscribe()

        # Example 2: Subscribe to specific event type
        print("\nExample 2: Subscribe to 'node:created' events")

        async def on_create(event: EventMessage):
            print(f"  📢 New node created: {event.payload}")

        sub2 = await workspace.events().subscribe_to_type("node:created", on_create)

        await workspace.nodes().create(
            node_type="Page", path="/new-page", properties={"title": "New Page"}
        )

        await asyncio.sleep(0.5)
        await sub2.unsubscribe()

        # Example 3: Subscribe to specific node type
        print("\nExample 3: Subscribe to 'Article' node type")

        async def on_article(event: EventMessage):
            print(f"  📢 Article event: {event.event_type}")

        sub3 = await workspace.events().subscribe_to_node_type("Article", on_article)

        await workspace.nodes().create(
            node_type="Article", path="/articles/new", properties={"title": "Breaking News"}
        )

        await asyncio.sleep(0.5)
        await sub3.unsubscribe()

        # Example 4: Multiple filters
        print("\nExample 4: Multiple filters (path + event types)")

        async def on_filtered(event: EventMessage):
            print(f"  📢 Filtered event: {event.event_type} at {event.payload.get('path')}")

        sub4 = await workspace.events().subscribe(
            callback=on_filtered,
            path="/content/*",
            event_types=["node:created", "node:updated"],
        )

        # These should trigger events
        node = await workspace.nodes().create(
            node_type="Page", path="/content/page1", properties={"title": "Page 1"}
        )

        await workspace.nodes().update(node_id=node.node_id, properties={"title": "Updated Page 1"})

        await asyncio.sleep(0.5)
        await sub4.unsubscribe()

        print("\nAll examples completed!")

    finally:
        await client.close()


if __name__ == "__main__":
    asyncio.run(main())
