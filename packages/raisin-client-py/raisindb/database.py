"""Database (repository) operations."""

from typing import TYPE_CHECKING, Any, List

from .protocol import RequestType, RequestContext, SqlResult
from .workspace import Workspace
from .node_types import NodeTypes
from .archetypes import Archetypes
from .element_types import ElementTypes
from .branches import Branches
from .tags import Tags

if TYPE_CHECKING:
    from .client import RaisinClient


class Database:
    """Represents a database (repository) in RaisinDB."""

    def __init__(
        self,
        client: "RaisinClient",
        name: str,
        branch: str | None = None,
        revision: str | None = None,
    ):
        """
        Initialize a database interface.

        Args:
            client: Parent RaisinClient instance
            name: Repository name
            branch: Optional branch name for scoping
            revision: Optional revision/commit ID for time-travel
        """
        self.client = client
        self.name = name
        self.branch = branch
        self.revision = revision

    def workspace(self, name: str) -> Workspace:
        """
        Get a workspace interface.

        Args:
            name: Workspace name

        Returns:
            Workspace instance
        """
        return Workspace(self, name)

    async def list_workspaces(self) -> List[Any]:
        """
        List all workspaces in this database.

        Returns:
            List of workspace objects
        """
        context = RequestContext(
            tenant_id=self.client.tenant_id, repository=self.name
        )
        result = await self.client.request_tracker.send_request(
            RequestType.WORKSPACE_LIST, context, {}
        )
        return result or []

    async def create_workspace(
        self, name: str, description: str | None = None
    ) -> Any:
        """
        Create a new workspace.

        Args:
            name: Workspace name
            description: Optional description

        Returns:
            Created workspace object
        """
        context = RequestContext(
            tenant_id=self.client.tenant_id, repository=self.name
        )
        payload = {"name": name}
        if description:
            payload["description"] = description

        result = await self.client.request_tracker.send_request(
            RequestType.WORKSPACE_CREATE, context, payload
        )
        return result

    async def sql(self, query: str, *params: Any) -> SqlResult:
        """
        Execute a SQL query with parameter binding.

        Args:
            query: SQL query string (use ? for parameters)
            *params: Query parameters

        Returns:
            SqlResult with columns, rows, and row count

        Example:
            ```python
            result = await db.sql("SELECT * FROM nodes WHERE node_type = ?", "Page")
            for row in result.rows:
                print(row)
            ```
        """
        context = RequestContext(
            tenant_id=self.client.tenant_id, repository=self.name
        )
        payload = {"query": query}
        if params:
            payload["params"] = list(params)

        result = await self.client.request_tracker.send_request(
            RequestType.SQL_QUERY, context, payload
        )

        return SqlResult(
            columns=result.get("columns", []),
            rows=result.get("rows", []),
            row_count=result.get("row_count", 0),
        )

    def on_branch(self, branch: str) -> "Database":
        """
        Create a new Database instance scoped to a specific branch.

        Args:
            branch: Branch name

        Returns:
            New Database instance with branch context
        """
        return Database(self.client, self.name, branch, self.revision)

    def at_revision(self, revision: str) -> "Database":
        """
        Create a new Database instance scoped to a specific revision/commit.

        Args:
            revision: Revision/commit ID

        Returns:
            New Database instance with revision context
        """
        return Database(self.client, self.name, self.branch, revision)

    def get_context(
        self, workspace: str | None = None, branch_override: str | None = None
    ) -> RequestContext:
        """
        Create a request context for this database.

        Args:
            workspace: Optional workspace name
            branch_override: Optional branch name override

        Returns:
            RequestContext
        """
        effective_branch = branch_override if branch_override is not None else self.branch
        return RequestContext(
            tenant_id=self.client.tenant_id,
            repository=self.name,
            workspace=workspace,
            branch=effective_branch,
            revision=self.revision,
        )

    def node_types(self) -> NodeTypes:
        """
        Get NodeTypes management operations.

        Returns:
            NodeTypes instance
        """
        return NodeTypes(self)

    def archetypes(self) -> Archetypes:
        """
        Get Archetypes management operations.

        Returns:
            Archetypes instance
        """
        return Archetypes(self)

    def element_types(self) -> ElementTypes:
        """
        Get ElementTypes management operations.

        Returns:
            ElementTypes instance
        """
        return ElementTypes(self)

    def branches(self) -> Branches:
        """
        Get Branches management operations.

        Returns:
            Branches instance
        """
        return Branches(self)

    def tags(self) -> Tags:
        """
        Get Tags management operations.

        Returns:
            Tags instance
        """
        return Tags(self)
