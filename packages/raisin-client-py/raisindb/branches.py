"""Branch management operations."""

from typing import TYPE_CHECKING, Any, Dict, List, Optional

from .protocol import RequestType, RequestContext

if TYPE_CHECKING:
    from .database import Database


class Branches:
    """Branch management operations."""

    def __init__(self, database: "Database"):
        """
        Initialize Branches manager.

        Args:
            database: Parent Database instance
        """
        self.database = database

    async def create(
        self,
        name: str,
        from_revision: Optional[str] = None,
        from_branch: Optional[str] = None,
    ) -> Any:
        """
        Create a new branch.

        Args:
            name: Branch name
            from_revision: Optional revision to branch from
            from_branch: Optional branch to branch from

        Returns:
            Created branch
        """
        context = self.database.get_context()
        payload: Dict[str, Any] = {"name": name}
        if from_revision is not None:
            payload["from_revision"] = from_revision
        if from_branch is not None:
            payload["from_branch"] = from_branch
        return await self.database.client.request_tracker.send_request(
            RequestType.BRANCH_CREATE, context, payload
        )

    async def get(self, name: str) -> Any:
        """
        Get a branch by name.

        Args:
            name: Branch name

        Returns:
            Branch information
        """
        context = self.database.get_context()
        payload = {"name": name}
        return await self.database.client.request_tracker.send_request(
            RequestType.BRANCH_GET, context, payload
        )

    async def list(self) -> List[Any]:
        """
        List all branches.

        Returns:
            List of branches
        """
        context = self.database.get_context()
        result = await self.database.client.request_tracker.send_request(
            RequestType.BRANCH_LIST, context, {}
        )
        return result if isinstance(result, list) else []

    async def delete(self, name: str) -> Any:
        """
        Delete a branch.

        Args:
            name: Branch name

        Returns:
            Deletion result
        """
        context = self.database.get_context()
        payload = {"name": name}
        return await self.database.client.request_tracker.send_request(
            RequestType.BRANCH_DELETE, context, payload
        )

    async def get_head(self, name: str) -> Any:
        """
        Get the HEAD revision of a branch.

        Args:
            name: Branch name

        Returns:
            HEAD revision
        """
        context = self.database.get_context()
        payload = {"name": name}
        return await self.database.client.request_tracker.send_request(
            RequestType.BRANCH_GET_HEAD, context, payload
        )

    async def update_head(self, name: str, revision: str) -> Any:
        """
        Update the HEAD revision of a branch.

        Args:
            name: Branch name
            revision: New HEAD revision

        Returns:
            Update result
        """
        context = self.database.get_context()
        payload = {"name": name, "revision": revision}
        return await self.database.client.request_tracker.send_request(
            RequestType.BRANCH_UPDATE_HEAD, context, payload
        )

    async def merge(
        self,
        source_branch: str,
        target_branch: str,
        strategy: Optional[str] = None,
        message: Optional[str] = None,
    ) -> Any:
        """
        Merge a source branch into a target branch.

        Args:
            source_branch: Source branch name
            target_branch: Target branch name
            strategy: Optional merge strategy
            message: Optional merge message

        Returns:
            Merge result
        """
        context = self.database.get_context()
        payload: Dict[str, Any] = {
            "source_branch": source_branch,
            "target_branch": target_branch,
        }
        if strategy is not None:
            payload["strategy"] = strategy
        if message is not None:
            payload["message"] = message
        return await self.database.client.request_tracker.send_request(
            RequestType.BRANCH_MERGE, context, payload
        )

    async def compare(self, branch: str, base_branch: str) -> Any:
        """
        Compare two branches to calculate divergence.

        Args:
            branch: Branch to compare
            base_branch: Base branch to compare against

        Returns:
            Comparison result with divergence information
        """
        context = self.database.get_context()
        payload = {"branch": branch, "base_branch": base_branch}
        return await self.database.client.request_tracker.send_request(
            RequestType.BRANCH_COMPARE, context, payload
        )
