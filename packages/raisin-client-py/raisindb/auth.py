"""JWT authentication management."""

import asyncio
import logging
from typing import Any, Dict, Optional

from .protocol import RequestType, RequestContext, AuthCredentials
from .utils.request_tracker import RequestTracker
from .exceptions import AuthenticationError

logger = logging.getLogger(__name__)


class TokenStorage:
    """Abstract base for token storage."""

    async def get_tokens(self) -> Optional[tuple[str, str]]:
        """Get stored access and refresh tokens."""
        raise NotImplementedError

    async def set_tokens(self, access_token: str, refresh_token: str) -> None:
        """Store access and refresh tokens."""
        raise NotImplementedError

    async def clear_tokens(self) -> None:
        """Clear stored tokens."""
        raise NotImplementedError


class MemoryTokenStorage(TokenStorage):
    """In-memory token storage (default)."""

    def __init__(self) -> None:
        """Initialize memory storage."""
        self.access_token: Optional[str] = None
        self.refresh_token: Optional[str] = None

    async def get_tokens(self) -> Optional[tuple[str, str]]:
        """Get stored tokens."""
        if self.access_token and self.refresh_token:
            return (self.access_token, self.refresh_token)
        return None

    async def set_tokens(self, access_token: str, refresh_token: str) -> None:
        """Store tokens."""
        self.access_token = access_token
        self.refresh_token = refresh_token

    async def clear_tokens(self) -> None:
        """Clear tokens."""
        self.access_token = None
        self.refresh_token = None


class AuthManager:
    """Manages authentication and token lifecycle."""

    def __init__(
        self,
        request_tracker: RequestTracker,
        tenant_id: str,
        token_storage: Optional[TokenStorage] = None,
    ):
        """
        Initialize the auth manager.

        Args:
            request_tracker: Request tracker for sending auth requests
            tenant_id: Tenant ID for authentication
            token_storage: Token storage implementation (defaults to memory)
        """
        self.request_tracker = request_tracker
        self.tenant_id = tenant_id
        self.token_storage = token_storage or MemoryTokenStorage()
        self._is_authenticated = False
        self._refresh_task: Optional[asyncio.Task[None]] = None

    async def authenticate(self, credentials: AuthCredentials) -> None:
        """
        Authenticate with username and password.

        Args:
            credentials: Username and password

        Raises:
            AuthenticationError: If authentication fails
        """
        try:
            context = RequestContext(tenant_id=self.tenant_id)
            response = await self.request_tracker.send_request(
                RequestType.AUTHENTICATE,
                context,
                {"username": credentials.username, "password": credentials.password},
            )

            if not response or "access_token" not in response:
                raise AuthenticationError("Invalid authentication response")

            await self.token_storage.set_tokens(
                response["access_token"], response["refresh_token"]
            )

            self._is_authenticated = True

            # Start automatic token refresh
            self._start_token_refresh(response.get("expires_in", 3600))

            logger.info(f"Authenticated as {credentials.username}")

        except Exception as e:
            logger.error(f"Authentication failed: {e}")
            raise AuthenticationError(f"Authentication failed: {e}")

    async def refresh_access_token(self) -> None:
        """
        Refresh the access token using the refresh token.

        Raises:
            AuthenticationError: If refresh fails
        """
        tokens = await self.token_storage.get_tokens()
        if not tokens:
            raise AuthenticationError("No refresh token available")

        _, refresh_token = tokens

        try:
            context = RequestContext(tenant_id=self.tenant_id)
            response = await self.request_tracker.send_request(
                RequestType.REFRESH_TOKEN, context, {"refresh_token": refresh_token}
            )

            if not response or "access_token" not in response:
                raise AuthenticationError("Invalid refresh response")

            await self.token_storage.set_tokens(
                response["access_token"], response["refresh_token"]
            )

            logger.info("Access token refreshed")

        except Exception as e:
            logger.error(f"Token refresh failed: {e}")
            self._is_authenticated = False
            raise AuthenticationError(f"Token refresh failed: {e}")

    def _start_token_refresh(self, expires_in: int) -> None:
        """Start automatic token refresh."""
        if self._refresh_task:
            self._refresh_task.cancel()

        # Refresh token 1 minute before expiry
        refresh_delay = max(expires_in - 60, 60)

        async def refresh_loop() -> None:
            while True:
                await asyncio.sleep(refresh_delay)
                try:
                    await self.refresh_access_token()
                except Exception as e:
                    logger.error(f"Automatic token refresh failed: {e}")
                    break

        self._refresh_task = asyncio.create_task(refresh_loop())

    async def logout(self) -> None:
        """Logout and clear tokens."""
        if self._refresh_task:
            self._refresh_task.cancel()
            self._refresh_task = None

        await self.token_storage.clear_tokens()
        self._is_authenticated = False
        logger.info("Logged out")

    async def get_access_token(self) -> Optional[str]:
        """Get the current access token."""
        tokens = await self.token_storage.get_tokens()
        if tokens:
            return tokens[0]
        return None

    @property
    def is_authenticated(self) -> bool:
        """Check if currently authenticated."""
        return self._is_authenticated
