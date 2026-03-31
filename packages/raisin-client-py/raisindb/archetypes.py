"""Archetype management operations."""

from typing import TYPE_CHECKING, Any, Dict, List

from .protocol import RequestType, RequestContext

if TYPE_CHECKING:
    from .database import Database


class Archetypes:
    """Archetype management operations."""

    def __init__(self, database: "Database"):
        """
        Initialize Archetypes manager.

        Args:
            database: Parent Database instance
        """
        self.database = database

    async def create(self, name: str, archetype: Dict[str, Any]) -> Any:
        """
        Create a new Archetype.

        Args:
            name: Archetype name
            archetype: Archetype definition

        Returns:
            Created Archetype
        """
        context = self.database.get_context()
        payload = {"name": name, "archetype": archetype}
        return await self.database.client.request_tracker.send_request(
            RequestType.ARCHETYPE_CREATE, context, payload
        )

    async def get(self, name: str) -> Any:
        """
        Get an Archetype by name.

        Args:
            name: Archetype name

        Returns:
            Archetype definition
        """
        context = self.database.get_context()
        payload = {"name": name}
        return await self.database.client.request_tracker.send_request(
            RequestType.ARCHETYPE_GET, context, payload
        )

    async def list(self, published_only: bool = False) -> List[Any]:
        """
        List all Archetypes.

        Args:
            published_only: Only return published Archetypes

        Returns:
            List of Archetypes
        """
        context = self.database.get_context()
        payload = {"published_only": published_only}
        result = await self.database.client.request_tracker.send_request(
            RequestType.ARCHETYPE_LIST, context, payload
        )
        return result if isinstance(result, list) else []

    async def update(self, name: str, archetype: Dict[str, Any]) -> Any:
        """
        Update an Archetype.

        Args:
            name: Archetype name
            archetype: Updated Archetype definition

        Returns:
            Updated Archetype
        """
        context = self.database.get_context()
        payload = {"name": name, "archetype": archetype}
        return await self.database.client.request_tracker.send_request(
            RequestType.ARCHETYPE_UPDATE, context, payload
        )

    async def delete(self, name: str) -> Any:
        """
        Delete an Archetype.

        Args:
            name: Archetype name

        Returns:
            Deletion result
        """
        context = self.database.get_context()
        payload = {"name": name}
        return await self.database.client.request_tracker.send_request(
            RequestType.ARCHETYPE_DELETE, context, payload
        )

    async def publish(self, name: str) -> Any:
        """
        Publish an Archetype.

        Args:
            name: Archetype name

        Returns:
            Publish result
        """
        context = self.database.get_context()
        payload = {"name": name}
        return await self.database.client.request_tracker.send_request(
            RequestType.ARCHETYPE_PUBLISH, context, payload
        )

    async def unpublish(self, name: str) -> Any:
        """
        Unpublish an Archetype.

        Args:
            name: Archetype name

        Returns:
            Unpublish result
        """
        context = self.database.get_context()
        payload = {"name": name}
        return await self.database.client.request_tracker.send_request(
            RequestType.ARCHETYPE_UNPUBLISH, context, payload
        )
