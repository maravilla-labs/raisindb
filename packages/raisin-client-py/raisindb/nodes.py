"""Node CRUD operations."""

from typing import TYPE_CHECKING, Any, Dict, List, Optional

from .protocol import RequestType, Node, RelationRef

if TYPE_CHECKING:
    from .workspace import Workspace


class NodeOperations:
    """Node CRUD operations within a workspace."""

    def __init__(self, workspace: "Workspace"):
        """
        Initialize node operations.

        Args:
            workspace: Parent Workspace instance
        """
        self.workspace = workspace

    async def create(
        self,
        node_type: str,
        path: str,
        properties: Optional[Dict[str, Any]] = None,
        content: Optional[Any] = None,
    ) -> Node:
        """
        Create a new node.

        Args:
            node_type: Type of node to create
            path: Path for the node
            properties: Node properties
            content: Node content

        Returns:
            Created Node instance
        """
        context = self.workspace.get_context()
        payload = {
            "node_type": node_type,
            "path": path,
            "properties": properties or {},
        }
        if content is not None:
            payload["content"] = content

        result = await self.workspace.database.client.request_tracker.send_request(
            RequestType.NODE_CREATE, context, payload
        )
        return Node.from_dict(result)

    async def get(self, node_id: str) -> Node:
        """
        Get a node by ID.

        Args:
            node_id: Node ID

        Returns:
            Node instance
        """
        context = self.workspace.get_context()
        result = await self.workspace.database.client.request_tracker.send_request(
            RequestType.NODE_GET, context, {"node_id": node_id}
        )
        return Node.from_dict(result)

    async def update(
        self,
        node_id: str,
        properties: Optional[Dict[str, Any]] = None,
        content: Optional[Any] = None,
    ) -> Node:
        """
        Update an existing node.

        Args:
            node_id: Node ID
            properties: Properties to update
            content: Content to update

        Returns:
            Updated Node instance
        """
        context = self.workspace.get_context()
        payload = {"node_id": node_id, "properties": properties or {}}
        if content is not None:
            payload["content"] = content

        result = await self.workspace.database.client.request_tracker.send_request(
            RequestType.NODE_UPDATE, context, payload
        )
        return Node.from_dict(result)

    async def delete(self, node_id: str) -> None:
        """
        Delete a node.

        Args:
            node_id: Node ID
        """
        context = self.workspace.get_context()
        await self.workspace.database.client.request_tracker.send_request(
            RequestType.NODE_DELETE, context, {"node_id": node_id}
        )

    async def query(
        self, query: Dict[str, Any], limit: Optional[int] = None, offset: Optional[int] = None
    ) -> List[Node]:
        """
        Query nodes.

        Args:
            query: Query object
            limit: Maximum number of results
            offset: Offset for pagination

        Returns:
            List of Node instances
        """
        context = self.workspace.get_context()
        payload: Dict[str, Any] = {"query": query}
        if limit is not None:
            payload["limit"] = limit
        if offset is not None:
            payload["offset"] = offset

        result = await self.workspace.database.client.request_tracker.send_request(
            RequestType.NODE_QUERY, context, payload
        )
        return [Node.from_dict(node) for node in result]

    async def query_by_path(self, path: str) -> List[Node]:
        """
        Query nodes by path pattern.

        Args:
            path: Path pattern (supports wildcards)

        Returns:
            List of Node instances
        """
        context = self.workspace.get_context()
        result = await self.workspace.database.client.request_tracker.send_request(
            RequestType.NODE_QUERY_BY_PATH, context, {"path": path}
        )
        return [Node.from_dict(node) for node in result]

    async def query_by_property(
        self, property_name: str, property_value: Any
    ) -> List[Node]:
        """
        Query nodes by a specific property.

        Args:
            property_name: Property name
            property_value: Property value

        Returns:
            List of Node instances
        """
        context = self.workspace.get_context()
        result = await self.workspace.database.client.request_tracker.send_request(
            RequestType.NODE_QUERY_BY_PROPERTY,
            context,
            {"property_name": property_name, "property_value": property_value},
        )
        return [Node.from_dict(node) for node in result]

    # ========================================================================
    # Tree Operations
    # ========================================================================

    async def list_children(self, parent_path: str) -> List[Node]:
        """
        List children of a parent node.

        Args:
            parent_path: Path of the parent node

        Returns:
            List of child Node instances
        """
        context = self.workspace.get_context()
        result = await self.workspace.database.client.request_tracker.send_request(
            RequestType.NODE_LIST_CHILDREN,
            context,
            {"parent_path": parent_path},
        )
        return [Node.from_dict(node) for node in result]

    async def get_tree(
        self, root_path: str, max_depth: Optional[int] = None
    ) -> Node:
        """
        Get a node tree starting from a root node.

        Args:
            root_path: Path of the root node
            max_depth: Maximum depth to traverse (optional)

        Returns:
            Root Node instance with nested children
        """
        context = self.workspace.get_context()
        payload: Dict[str, Any] = {"root_path": root_path}
        if max_depth is not None:
            payload["max_depth"] = max_depth

        result = await self.workspace.database.client.request_tracker.send_request(
            RequestType.NODE_GET_TREE,
            context,
            payload,
        )
        return Node.from_dict(result)

    async def get_tree_flat(
        self, root_path: str, max_depth: Optional[int] = None
    ) -> List[Node]:
        """
        Get a flattened node tree starting from a root node.

        Args:
            root_path: Path of the root node
            max_depth: Maximum depth to traverse (optional)

        Returns:
            List of Node instances in tree order
        """
        context = self.workspace.get_context()
        payload: Dict[str, Any] = {"root_path": root_path}
        if max_depth is not None:
            payload["max_depth"] = max_depth

        result = await self.workspace.database.client.request_tracker.send_request(
            RequestType.NODE_GET_TREE_FLAT,
            context,
            payload,
        )
        return [Node.from_dict(node) for node in result]

    # ========================================================================
    # Node Manipulation Operations
    # ========================================================================

    async def move(self, from_path: str, to_parent_path: str) -> Node:
        """
        Move a node to a new parent.

        Args:
            from_path: Source node path
            to_parent_path: Destination parent path

        Returns:
            Moved Node instance
        """
        context = self.workspace.get_context()
        result = await self.workspace.database.client.request_tracker.send_request(
            RequestType.NODE_MOVE,
            context,
            {"from_path": from_path, "to_parent_path": to_parent_path},
        )
        return Node.from_dict(result)

    async def rename(self, node_path: str, new_name: str) -> Node:
        """
        Rename a node.

        Args:
            node_path: Node path
            new_name: New name for the node

        Returns:
            Renamed Node instance
        """
        context = self.workspace.get_context()
        result = await self.workspace.database.client.request_tracker.send_request(
            RequestType.NODE_RENAME,
            context,
            {"node_path": node_path, "new_name": new_name},
        )
        return Node.from_dict(result)

    async def copy(
        self, from_path: str, to_parent_path: str, new_name: Optional[str] = None
    ) -> Node:
        """
        Copy a node to a new parent (shallow copy).

        Args:
            from_path: Source node path
            to_parent_path: Destination parent path
            new_name: New name for the copied node (optional)

        Returns:
            Copied Node instance
        """
        context = self.workspace.get_context()
        payload: Dict[str, Any] = {
            "from_path": from_path,
            "to_parent_path": to_parent_path,
            "deep": False,
        }
        if new_name is not None:
            payload["new_name"] = new_name

        result = await self.workspace.database.client.request_tracker.send_request(
            RequestType.NODE_COPY,
            context,
            payload,
        )
        return Node.from_dict(result)

    async def copy_tree(
        self, from_path: str, to_parent_path: str, new_name: Optional[str] = None
    ) -> Node:
        """
        Copy a node tree to a new parent (deep copy with all children).

        Args:
            from_path: Source node path
            to_parent_path: Destination parent path
            new_name: New name for the copied node (optional)

        Returns:
            Copied Node tree
        """
        context = self.workspace.get_context()
        payload: Dict[str, Any] = {
            "from_path": from_path,
            "to_parent_path": to_parent_path,
            "deep": True,
        }
        if new_name is not None:
            payload["new_name"] = new_name

        result = await self.workspace.database.client.request_tracker.send_request(
            RequestType.NODE_COPY_TREE,
            context,
            payload,
        )
        return Node.from_dict(result)

    async def reorder(self, node_path: str, order_key: str) -> Node:
        """
        Reorder a node by setting a new order key.

        Args:
            node_path: Node path
            order_key: New order key (base62-encoded fractional index)

        Returns:
            Reordered Node instance
        """
        context = self.workspace.get_context()
        result = await self.workspace.database.client.request_tracker.send_request(
            RequestType.NODE_REORDER,
            context,
            {"node_path": node_path, "order_key": order_key},
        )
        return Node.from_dict(result)

    async def move_child_before(
        self, parent_path: str, child_path: str, reference_path: str
    ) -> Node:
        """
        Move a child node before a reference sibling.

        Args:
            parent_path: Parent node path
            child_path: Child node path to move
            reference_path: Reference sibling path to position before

        Returns:
            Moved Node instance
        """
        context = self.workspace.get_context()
        result = await self.workspace.database.client.request_tracker.send_request(
            RequestType.NODE_MOVE_CHILD_BEFORE,
            context,
            {
                "parent_path": parent_path,
                "child_path": child_path,
                "reference_path": reference_path,
            },
        )
        return Node.from_dict(result)

    async def move_child_after(
        self, parent_path: str, child_path: str, reference_path: str
    ) -> Node:
        """
        Move a child node after a reference sibling.

        Args:
            parent_path: Parent node path
            child_path: Child node path to move
            reference_path: Reference sibling path to position after

        Returns:
            Moved Node instance
        """
        context = self.workspace.get_context()
        result = await self.workspace.database.client.request_tracker.send_request(
            RequestType.NODE_MOVE_CHILD_AFTER,
            context,
            {
                "parent_path": parent_path,
                "child_path": child_path,
                "reference_path": reference_path,
            },
        )
        return Node.from_dict(result)

    # ========================================================================
    # Relationship Operations
    # ========================================================================

    async def add_relation(
        self,
        node_path: str,
        relation_type: str,
        target_node_path: str,
        weight: Optional[float] = None,
    ) -> Node:
        """
        Add a relationship between two nodes.

        Args:
            node_path: Source node path
            relation_type: Type of relationship
            target_node_path: Target node path
            weight: Optional relationship weight

        Returns:
            Updated Node instance
        """
        context = self.workspace.get_context()
        payload: Dict[str, Any] = {
            "node_path": node_path,
            "relation_type": relation_type,
            "target_node_path": target_node_path,
        }
        if weight is not None:
            payload["weight"] = weight

        result = await self.workspace.database.client.request_tracker.send_request(
            RequestType.RELATION_ADD,
            context,
            payload,
        )
        return Node.from_dict(result)

    async def remove_relation(
        self, node_path: str, relation_type: str, target_node_path: str
    ) -> Node:
        """
        Remove a relationship between two nodes.

        Args:
            node_path: Source node path
            relation_type: Type of relationship
            target_node_path: Target node path

        Returns:
            Updated Node instance
        """
        context = self.workspace.get_context()
        result = await self.workspace.database.client.request_tracker.send_request(
            RequestType.RELATION_REMOVE,
            context,
            {
                "node_path": node_path,
                "relation_type": relation_type,
                "target_node_path": target_node_path,
            },
        )
        return Node.from_dict(result)

    async def get_relationships(self, node_path: str) -> List[RelationRef]:
        """
        Get all relationships for a node.

        Args:
            node_path: Node path

        Returns:
            List of RelationRef instances
        """
        context = self.workspace.get_context()
        result = await self.workspace.database.client.request_tracker.send_request(
            RequestType.RELATIONS_GET,
            context,
            {"node_path": node_path},
        )
        return [
            RelationRef(
                target=rel["target"],
                workspace=rel["workspace"],
                target_node_type=rel["target_node_type"],
                relation_type=rel["relation_type"],
                weight=rel.get("weight"),
            )
            for rel in result
        ]
