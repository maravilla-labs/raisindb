"""SQL query examples for RaisinDB Python client."""

import asyncio
import logging

from raisindb import RaisinClient

logging.basicConfig(level=logging.INFO)


async def main() -> None:
    """SQL query examples."""
    async with RaisinClient("raisin://localhost:8080/sys/default") as client:
        await client.authenticate("admin", "admin")

        db = client.database("demo")
        workspace = db.workspace("content")

        # Create some test nodes
        print("Creating test nodes...")
        for i in range(5):
            await workspace.nodes().create(
                node_type="Article",
                path=f"/articles/article-{i}",
                properties={
                    "title": f"Article {i}",
                    "published": i % 2 == 0,
                    "views": i * 100,
                },
            )

        # Example 1: Simple SELECT
        print("\n1. Simple SELECT:")
        result = await db.sql("SELECT * FROM nodes LIMIT 3")
        print(f"   Columns: {result.columns}")
        print(f"   Rows: {result.row_count}")

        # Example 2: Parameterized query (safe from SQL injection)
        print("\n2. Parameterized query:")
        result = await db.sql(
            "SELECT * FROM nodes WHERE node_type = ? AND published = ?", "Article", True
        )
        print(f"   Found {result.row_count} published articles")

        # Example 3: Aggregation
        print("\n3. Aggregation:")
        result = await db.sql("SELECT node_type, COUNT(*) as count FROM nodes GROUP BY node_type")
        for row in result.rows:
            print(f"   {row[0]}: {row[1]} nodes")

        # Example 4: WHERE with multiple conditions
        print("\n4. Complex WHERE:")
        result = await db.sql(
            """
            SELECT * FROM nodes
            WHERE node_type = ?
            AND path LIKE ?
            ORDER BY created_at DESC
            """,
            "Article",
            "/articles/%",
        )
        print(f"   Found {result.row_count} articles in /articles/")

        # Example 5: Property filtering (JSON queries depend on backend)
        print("\n5. Property filtering:")
        result = await db.sql(
            "SELECT * FROM nodes WHERE node_type = ? AND published = ?",
            "Article",
            True
        )
        for row in result.rows:
            print(f"   {row}")

        print("\nDone!")


if __name__ == "__main__":
    asyncio.run(main())
