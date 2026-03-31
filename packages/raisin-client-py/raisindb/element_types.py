"""ElementType management operations."""

from typing import TYPE_CHECKING, Any, Dict, List

from .protocol import RequestType, RequestContext

if TYPE_CHECKING:
    from .database import Database


class ElementTypes:
    """ElementType management operations."""

    def __init__(self, database: "Database"):
        """
        Initialize ElementTypes manager.

        Args:
            database: Parent Database instance
        """
        self.database = database

    async def create(self, name: str, element_type: Dict[str, Any]) -> Any:
        """
        Create a new ElementType.

        Args:
            name: ElementType name
            element_type: ElementType definition

        Returns:
            Created ElementType
        """
        context = self.database.get_context()
        payload = {"name": name, "element_type": element_type}
        return await self.database.client.request_tracker.send_request(
            RequestType.ELEMENT_TYPE_CREATE, context, payload
        )

    async def get(self, name: str) -> Any:
        """
        Get an ElementType by name.

        Args:
            name: ElementType name

        Returns:
            ElementType definition
        """
        context = self.database.get_context()
        payload = {"name": name}
        return await self.database.client.request_tracker.send_request(
            RequestType.ELEMENT_TYPE_GET, context, payload
        )

    async def list(self, published_only: bool = False) -> List[Any]:
        """
        List all ElementTypes.

        Args:
            published_only: Only return published ElementTypes

        Returns:
            List of ElementTypes
        """
        context = self.database.get_context()
        payload = {"published_only": published_only}
        result = await self.database.client.request_tracker.send_request(
            RequestType.ELEMENT_TYPE_LIST, context, payload
        )
        return result if isinstance(result, list) else []

    async def update(self, name: str, element_type: Dict[str, Any]) -> Any:
        """
        Update an ElementType.

        Args:
            name: ElementType name
            element_type: Updated ElementType definition

        Returns:
            Updated ElementType
        """
        context = self.database.get_context()
        payload = {"name": name, "element_type": element_type}
        return await self.database.client.request_tracker.send_request(
            RequestType.ELEMENT_TYPE_UPDATE, context, payload
        )

    async def delete(self, name: str) -> Any:
        """
        Delete an ElementType.

        Args:
            name: ElementType name

        Returns:
            Deletion result
        """
        context = self.database.get_context()
        payload = {"name": name}
        return await self.database.client.request_tracker.send_request(
            RequestType.ELEMENT_TYPE_DELETE, context, payload
        )

    async def publish(self, name: str) -> Any:
        """
        Publish an ElementType.

        Args:
            name: ElementType name

        Returns:
            Publish result
        """
        context = self.database.get_context()
        payload = {"name": name}
        return await self.database.client.request_tracker.send_request(
            RequestType.ELEMENT_TYPE_PUBLISH, context, payload
        )

    async def unpublish(self, name: str) -> Any:
        """
        Unpublish an ElementType.

        Args:
            name: ElementType name

        Returns:
            Unpublish result
        """
        context = self.database.get_context()
        payload = {"name": name}
        return await self.database.client.request_tracker.send_request(
            RequestType.ELEMENT_TYPE_UNPUBLISH, context, payload
        )
