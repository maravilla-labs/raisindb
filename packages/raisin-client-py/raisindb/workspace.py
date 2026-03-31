"""Workspace operations."""

from typing import TYPE_CHECKING, Any, Optional

from .protocol import RequestType, WorkspaceInfo
from .nodes import NodeOperations
from .events import EventSubscriptions

if TYPE_CHECKING:
    from .database import Database


class Workspace:
    """Represents a workspace within a database."""

    def __init__(
        self,
        database: "Database",
        name: str,
        branch: str = "main",
        revision: str | None = None,
    ):
        """
        Initialize a workspace interface.

        Args:
            database: Parent Database instance
            name: Workspace name
            branch: Branch name (defaults to "main")
            revision: Optional revision/commit ID for time-travel
        """
        self.database = database
        self.name = name
        self.branch = branch
        self.revision = revision
        self._nodes: Optional[NodeOperations] = None
        self._events: Optional[EventSubscriptions] = None

    def nodes(self) -> NodeOperations:
        """
        Get the node operations interface.

        Returns:
            NodeOperations instance
        """
        if not self._nodes:
            self._nodes = NodeOperations(self)
        return self._nodes

    def events(self) -> EventSubscriptions:
        """
        Get the event subscriptions interface.

        Returns:
            EventSubscriptions instance
        """
        if not self._events:
            self._events = EventSubscriptions(self)
        return self._events

    async def get_info(self) -> WorkspaceInfo:
        """
        Get workspace information.

        Returns:
            WorkspaceInfo object
        """
        context = self.get_context()
        result = await self.database.client.request_tracker.send_request(
            RequestType.WORKSPACE_GET, context, {"name": self.name}
        )
        return WorkspaceInfo.from_dict(result)

    async def update(
        self,
        description: Optional[str] = None,
        allowed_node_types: Optional[list[str]] = None,
        allowed_root_node_types: Optional[list[str]] = None,
    ) -> WorkspaceInfo:
        """
        Update workspace configuration.

        Args:
            description: New description
            allowed_node_types: List of allowed node types
            allowed_root_node_types: List of allowed root node types

        Returns:
            Updated WorkspaceInfo
        """
        context = self.get_context()
        payload: dict[str, Any] = {"name": self.name}

        if description is not None:
            payload["description"] = description
        if allowed_node_types is not None:
            payload["allowed_node_types"] = allowed_node_types
        if allowed_root_node_types is not None:
            payload["allowed_root_node_types"] = allowed_root_node_types

        result = await self.database.client.request_tracker.send_request(
            RequestType.WORKSPACE_UPDATE, context, payload
        )
        return WorkspaceInfo.from_dict(result)

    async def delete(self) -> None:
        """
        Delete this workspace.

        Note: This may not be supported by all storage backends.
        """
        context = self.get_context()
        await self.database.client.request_tracker.send_request(
            RequestType.WORKSPACE_DELETE, context, {"name": self.name}
        )

    def get_context(self):
        """Get request context for this workspace."""
        context = self.database.get_context(workspace=self.name, branch_override=self.branch)
        # Override with workspace-specific revision if set
        if self.revision is not None:
            context.revision = self.revision
        return context

    def on_branch(self, branch: str) -> "Workspace":
        """
        Create a new workspace instance on a different branch.

        Args:
            branch: Branch name

        Returns:
            New Workspace instance
        """
        return Workspace(self.database, self.name, branch, self.revision)

    def at_revision(self, revision: str) -> "Workspace":
        """
        Create a new workspace instance scoped to a specific revision/commit.

        Args:
            revision: Revision/commit ID

        Returns:
            New Workspace instance with revision context
        """
        return Workspace(self.database, self.name, self.branch, revision)
