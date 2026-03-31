"""Tag management operations."""

from typing import TYPE_CHECKING, Any, Dict, List, Optional

from .protocol import RequestType, RequestContext

if TYPE_CHECKING:
    from .database import Database


class Tags:
    """Tag management operations."""

    def __init__(self, database: "Database"):
        """
        Initialize Tags manager.

        Args:
            database: Parent Database instance
        """
        self.database = database

    async def create(
        self, name: str, revision: str, message: Optional[str] = None
    ) -> Any:
        """
        Create a new tag.

        Args:
            name: Tag name
            revision: Revision to tag
            message: Optional tag message

        Returns:
            Created tag
        """
        context = self.database.get_context()
        payload: Dict[str, Any] = {"name": name, "revision": revision}
        if message is not None:
            payload["message"] = message
        return await self.database.client.request_tracker.send_request(
            RequestType.TAG_CREATE, context, payload
        )

    async def get(self, name: str) -> Any:
        """
        Get a tag by name.

        Args:
            name: Tag name

        Returns:
            Tag information
        """
        context = self.database.get_context()
        payload = {"name": name}
        return await self.database.client.request_tracker.send_request(
            RequestType.TAG_GET, context, payload
        )

    async def list(self) -> List[Any]:
        """
        List all tags.

        Returns:
            List of tags
        """
        context = self.database.get_context()
        result = await self.database.client.request_tracker.send_request(
            RequestType.TAG_LIST, context, {}
        )
        return result if isinstance(result, list) else []

    async def delete(self, name: str) -> Any:
        """
        Delete a tag.

        Args:
            name: Tag name

        Returns:
            Deletion result
        """
        context = self.database.get_context()
        payload = {"name": name}
        return await self.database.client.request_tracker.send_request(
            RequestType.TAG_DELETE, context, payload
        )
