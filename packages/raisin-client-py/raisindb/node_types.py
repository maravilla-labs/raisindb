"""NodeType management operations."""

from typing import TYPE_CHECKING, Any, Dict, List

from .protocol import RequestType, RequestContext

if TYPE_CHECKING:
    from .database import Database


class NodeTypes:
    """NodeType management operations."""

    def __init__(self, database: "Database"):
        """
        Initialize NodeTypes manager.

        Args:
            database: Parent Database instance
        """
        self.database = database

    async def create(self, name: str, node_type: Dict[str, Any]) -> Any:
        """
        Create a new NodeType.

        Args:
            name: NodeType name
            node_type: NodeType definition

        Returns:
            Created NodeType
        """
        context = self.database.get_context()
        payload = {"name": name, "node_type": node_type}
        return await self.database.client.request_tracker.send_request(
            RequestType.NODE_TYPE_CREATE, context, payload
        )

    async def get(self, name: str) -> Any:
        """
        Get a NodeType by name.

        Args:
            name: NodeType name

        Returns:
            NodeType definition
        """
        context = self.database.get_context()
        payload = {"name": name}
        return await self.database.client.request_tracker.send_request(
            RequestType.NODE_TYPE_GET, context, payload
        )

    async def list(self, published_only: bool = False) -> List[Any]:
        """
        List all NodeTypes.

        Args:
            published_only: Only return published NodeTypes

        Returns:
            List of NodeTypes
        """
        context = self.database.get_context()
        payload = {"published_only": published_only}
        result = await self.database.client.request_tracker.send_request(
            RequestType.NODE_TYPE_LIST, context, payload
        )
        return result if isinstance(result, list) else []

    async def update(self, name: str, node_type: Dict[str, Any]) -> Any:
        """
        Update a NodeType.

        Args:
            name: NodeType name
            node_type: Updated NodeType definition

        Returns:
            Updated NodeType
        """
        context = self.database.get_context()
        payload = {"name": name, "node_type": node_type}
        return await self.database.client.request_tracker.send_request(
            RequestType.NODE_TYPE_UPDATE, context, payload
        )

    async def delete(self, name: str) -> Any:
        """
        Delete a NodeType.

        Args:
            name: NodeType name

        Returns:
            Deletion result
        """
        context = self.database.get_context()
        payload = {"name": name}
        return await self.database.client.request_tracker.send_request(
            RequestType.NODE_TYPE_DELETE, context, payload
        )

    async def publish(self, name: str) -> Any:
        """
        Publish a NodeType.

        Args:
            name: NodeType name

        Returns:
            Publish result
        """
        context = self.database.get_context()
        payload = {"name": name}
        return await self.database.client.request_tracker.send_request(
            RequestType.NODE_TYPE_PUBLISH, context, payload
        )

    async def unpublish(self, name: str) -> Any:
        """
        Unpublish a NodeType.

        Args:
            name: NodeType name

        Returns:
            Unpublish result
        """
        context = self.database.get_context()
        payload = {"name": name}
        return await self.database.client.request_tracker.send_request(
            RequestType.NODE_TYPE_UNPUBLISH, context, payload
        )

    async def validate(self, node: Dict[str, Any]) -> Any:
        """
        Validate a node against its NodeType.

        Args:
            node: Node data to validate

        Returns:
            Validation result
        """
        context = self.database.get_context()
        payload = {"node": node}
        return await self.database.client.request_tracker.send_request(
            RequestType.NODE_TYPE_VALIDATE, context, payload
        )

    async def get_resolved(self, name: str) -> Any:
        """
        Get a resolved NodeType with inherited properties.

        Args:
            name: NodeType name

        Returns:
            Resolved NodeType definition
        """
        context = self.database.get_context()
        payload = {"name": name}
        return await self.database.client.request_tracker.send_request(
            RequestType.NODE_TYPE_GET_RESOLVED, context, payload
        )
