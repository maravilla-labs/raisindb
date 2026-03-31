"""Main RaisinDB client class."""

import asyncio
import logging
from typing import Optional

from .connection import ConnectionManager
from .auth import AuthManager, TokenStorage
from .database import Database
from .protocol import AuthCredentials, ResponseEnvelope, EventMessage, RequestType, RequestContext
from .utils.request_tracker import RequestTracker
from .exceptions import ConnectionError, AuthenticationError
from typing import Any, List, Dict

logger = logging.getLogger(__name__)


class RaisinClient:
    """Main client for connecting to RaisinDB."""

    def __init__(
        self,
        url: str,
        tenant_id: Optional[str] = None,
        token_storage: Optional[TokenStorage] = None,
        initial_reconnect_delay: float = 1.0,
        max_reconnect_delay: float = 30.0,
        max_reconnect_attempts: Optional[int] = None,
        request_timeout: float = 30.0,
    ):
        """
        Initialize the RaisinDB client.

        Args:
            url: WebSocket URL (e.g., "raisin://localhost:8080/sys/default")
            tenant_id: Tenant ID (extracted from URL if not provided)
            token_storage: Token storage implementation
            initial_reconnect_delay: Initial delay between reconnect attempts
            max_reconnect_delay: Maximum delay between reconnect attempts
            max_reconnect_attempts: Maximum reconnect attempts (None for unlimited)
            request_timeout: Default timeout for requests in seconds
        """
        self.url = url
        self.tenant_id = tenant_id or self._extract_tenant_from_url(url)

        # Initialize connection manager
        self.connection = ConnectionManager(
            url,
            initial_reconnect_delay=initial_reconnect_delay,
            max_reconnect_delay=max_reconnect_delay,
            max_reconnect_attempts=max_reconnect_attempts,
        )

        # Initialize request tracker
        self.request_tracker = RequestTracker(
            send_func=self.connection.send, default_timeout=request_timeout
        )

        # Initialize auth manager
        self.auth = AuthManager(
            request_tracker=self.request_tracker,
            tenant_id=self.tenant_id,
            token_storage=token_storage,
        )

        # Set up message handler
        self.connection.on_message(self._handle_message)

        # Event handlers for subscriptions
        self._event_handlers: dict[str, callable] = {}

    def _extract_tenant_from_url(self, url: str) -> str:
        """Extract tenant ID from URL path."""
        # URL format: raisin://host:port/sys/{tenant_id} or /sys/{tenant_id}/{repository}
        parts = url.split("/sys/")
        if len(parts) < 2:
            raise ValueError("URL must include /sys/{tenant_id}")

        tenant_part = parts[1].split("/")[0]
        return tenant_part

    def _handle_message(self, message: ResponseEnvelope | EventMessage) -> None:
        """Handle incoming messages from the WebSocket."""
        if isinstance(message, EventMessage):
            # Handle event message
            self._handle_event(message)
        elif isinstance(message, ResponseEnvelope):
            # Handle response
            asyncio.create_task(self.request_tracker.handle_response(message))

    def _handle_event(self, event: EventMessage) -> None:
        """Handle an event message."""
        # Look up event handler by subscription ID
        handler = self._event_handlers.get(event.subscription_id)
        if handler:
            try:
                handler(event)
            except Exception as e:
                logger.error(f"Error in event handler: {e}")

    def _register_event_handler(
        self, subscription_id: str, handler: callable
    ) -> None:
        """Register an event handler for a subscription."""
        self._event_handlers[subscription_id] = handler

    def _unregister_event_handler(self, subscription_id: str) -> None:
        """Unregister an event handler."""
        self._event_handlers.pop(subscription_id, None)

    async def connect(self) -> None:
        """
        Connect to the RaisinDB server.

        Raises:
            ConnectionError: If connection fails
        """
        await self.connection.connect()

    async def authenticate(self, username: str, password: str) -> None:
        """
        Authenticate with username and password.

        Args:
            username: Username
            password: Password

        Raises:
            AuthenticationError: If authentication fails
        """
        credentials = AuthCredentials(username=username, password=password)
        await self.auth.authenticate(credentials)

    async def close(self) -> None:
        """Close the connection and clean up resources."""
        await self.auth.logout()
        await self.connection.close()

    def database(self, name: str) -> Database:
        """
        Get a database (repository) interface.

        Args:
            name: Repository name

        Returns:
            Database instance
        """
        return Database(self, name)

    async def create_repository(
        self,
        repository_id: str,
        description: Optional[str] = None,
        config: Optional[Dict[str, Any]] = None,
    ) -> Any:
        """
        Create a new repository.

        Args:
            repository_id: Repository identifier
            description: Optional repository description
            config: Optional repository configuration

        Returns:
            Created repository
        """
        context = RequestContext(tenant_id=self.tenant_id)
        payload: Dict[str, Any] = {"repository_id": repository_id}
        if description is not None:
            payload["description"] = description
        if config is not None:
            payload["config"] = config
        return await self.request_tracker.send_request(
            RequestType.REPOSITORY_CREATE, context, payload
        )

    async def get_repository(self, repository_id: str) -> Any:
        """
        Get a repository by ID.

        Args:
            repository_id: Repository identifier

        Returns:
            Repository information
        """
        context = RequestContext(tenant_id=self.tenant_id)
        payload = {"repository_id": repository_id}
        return await self.request_tracker.send_request(
            RequestType.REPOSITORY_GET, context, payload
        )

    async def list_repositories(self) -> List[Any]:
        """
        List all repositories.

        Returns:
            List of repositories
        """
        context = RequestContext(tenant_id=self.tenant_id)
        result = await self.request_tracker.send_request(
            RequestType.REPOSITORY_LIST, context, {}
        )
        return result if isinstance(result, list) else []

    async def update_repository(
        self,
        repository_id: str,
        description: Optional[str] = None,
        config: Optional[Dict[str, Any]] = None,
    ) -> Any:
        """
        Update a repository.

        Args:
            repository_id: Repository identifier
            description: New repository description
            config: New repository configuration

        Returns:
            Updated repository
        """
        context = RequestContext(tenant_id=self.tenant_id)
        payload: Dict[str, Any] = {"repository_id": repository_id}
        if description is not None:
            payload["description"] = description
        if config is not None:
            payload["config"] = config
        return await self.request_tracker.send_request(
            RequestType.REPOSITORY_UPDATE, context, payload
        )

    async def delete_repository(self, repository_id: str) -> Any:
        """
        Delete a repository.

        Args:
            repository_id: Repository identifier

        Returns:
            Deletion result
        """
        context = RequestContext(tenant_id=self.tenant_id)
        payload = {"repository_id": repository_id}
        return await self.request_tracker.send_request(
            RequestType.REPOSITORY_DELETE, context, payload
        )

    @property
    def is_connected(self) -> bool:
        """Check if connected to the server."""
        return self.connection.is_connected

    @property
    def is_authenticated(self) -> bool:
        """Check if authenticated."""
        return self.auth.is_authenticated

    async def __aenter__(self) -> "RaisinClient":
        """Async context manager entry."""
        await self.connect()
        return self

    async def __aexit__(self, exc_type, exc_val, exc_tb) -> None:
        """Async context manager exit."""
        await self.close()
