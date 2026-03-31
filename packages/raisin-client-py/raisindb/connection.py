"""WebSocket connection manager with auto-reconnect."""

import asyncio
import logging
from typing import Any, Callable, Dict, List, Optional
from urllib.parse import urlparse

import websockets
from websockets.client import WebSocketClientProtocol

from .protocol import decode_response, ResponseEnvelope, EventMessage
from .utils.reconnect import ReconnectManager
from .exceptions import ConnectionError

logger = logging.getLogger(__name__)


class ConnectionManager:
    """Manages WebSocket connection with automatic reconnection."""

    def __init__(
        self,
        url: str,
        initial_reconnect_delay: float = 1.0,
        max_reconnect_delay: float = 30.0,
        max_reconnect_attempts: Optional[int] = None,
    ):
        """
        Initialize the connection manager.

        Args:
            url: WebSocket URL (e.g., "raisin://localhost:8080/sys/default")
            initial_reconnect_delay: Initial delay between reconnect attempts
            max_reconnect_delay: Maximum delay between reconnect attempts
            max_reconnect_attempts: Maximum reconnect attempts (None for unlimited)
        """
        self.url = self._convert_url(url)
        self.ws: Optional[WebSocketClientProtocol] = None
        self.reconnect_manager = ReconnectManager(
            initial_delay=initial_reconnect_delay,
            max_delay=max_reconnect_delay,
            max_attempts=max_reconnect_attempts,
        )

        self._connected = False
        self._closing = False
        self._message_handlers: List[Callable[[Any], None]] = []
        self._close_handlers: List[Callable[[], None]] = []
        self._error_handlers: List[Callable[[Exception], None]] = []
        self._receive_task: Optional[asyncio.Task[None]] = None

    def _convert_url(self, url: str) -> str:
        """Convert raisin:// URL to ws:// or wss:// URL."""
        if url.startswith("raisin://"):
            parsed = urlparse(url)
            scheme = "wss" if parsed.port == 443 else "ws"
            return url.replace("raisin://", f"{scheme}://") + "/ws"
        elif url.startswith("ws://") or url.startswith("wss://"):
            return url
        else:
            raise ValueError(f"Invalid URL scheme: {url}")

    def on_message(self, handler: Callable[[Any], None]) -> None:
        """Register a message handler."""
        self._message_handlers.append(handler)

    def on_close(self, handler: Callable[[], None]) -> None:
        """Register a close handler."""
        self._close_handlers.append(handler)

    def on_error(self, handler: Callable[[Exception], None]) -> None:
        """Register an error handler."""
        self._error_handlers.append(handler)

    async def connect(self) -> None:
        """
        Connect to the WebSocket server.

        Raises:
            ConnectionError: If connection fails
        """
        if self._connected:
            return

        try:
            self.ws = await websockets.connect(self.url)
            self._connected = True
            self._closing = False
            self.reconnect_manager.reset()

            # Start receiving messages
            self._receive_task = asyncio.create_task(self._receive_loop())

            logger.info(f"Connected to {self.url}")

        except Exception as e:
            logger.error(f"Failed to connect: {e}")
            raise ConnectionError(f"Failed to connect to {self.url}: {e}")

    async def close(self) -> None:
        """Close the WebSocket connection."""
        self._closing = True
        self._connected = False

        if self._receive_task:
            self._receive_task.cancel()
            try:
                await self._receive_task
            except asyncio.CancelledError:
                pass

        if self.ws:
            await self.ws.close()
            self.ws = None

        for handler in self._close_handlers:
            try:
                handler()
            except Exception as e:
                logger.error(f"Error in close handler: {e}")

    def send(self, data: bytes) -> None:
        """
        Send binary data over the WebSocket.

        Args:
            data: Binary data to send

        Raises:
            ConnectionError: If not connected
        """
        if not self._connected or not self.ws:
            raise ConnectionError("Not connected")

        asyncio.create_task(self._send_async(data))

    async def _send_async(self, data: bytes) -> None:
        """Send data asynchronously."""
        try:
            if self.ws:
                await self.ws.send(data)
        except Exception as e:
            logger.error(f"Failed to send message: {e}")
            await self._handle_disconnect()

    async def _receive_loop(self) -> None:
        """Receive and process messages."""
        while self._connected and not self._closing:
            try:
                if not self.ws:
                    break

                message = await self.ws.recv()

                # Decode MessagePack response
                if isinstance(message, bytes):
                    decoded = decode_response(message)

                    # Notify handlers
                    for handler in self._message_handlers:
                        try:
                            handler(decoded)
                        except Exception as e:
                            logger.error(f"Error in message handler: {e}")

            except websockets.exceptions.ConnectionClosed:
                logger.warning("WebSocket connection closed")
                await self._handle_disconnect()
                break

            except Exception as e:
                logger.error(f"Error receiving message: {e}")
                for handler in self._error_handlers:
                    try:
                        handler(e)
                    except Exception as err:
                        logger.error(f"Error in error handler: {err}")

    async def _handle_disconnect(self) -> None:
        """Handle disconnection and attempt reconnection."""
        if self._closing:
            return

        self._connected = False

        # Attempt reconnection
        while self.reconnect_manager.should_reconnect() and not self._closing:
            try:
                await self.reconnect_manager.wait_before_reconnect()
                logger.info(
                    f"Attempting to reconnect (attempt {self.reconnect_manager.attempt})..."
                )
                await self.connect()
                return  # Successfully reconnected

            except Exception as e:
                logger.error(f"Reconnection failed: {e}")

        # Max reconnect attempts reached or closing
        if not self._closing:
            logger.error("Max reconnection attempts reached")
            for handler in self._close_handlers:
                try:
                    handler()
                except Exception as e:
                    logger.error(f"Error in close handler: {e}")

    @property
    def is_connected(self) -> bool:
        """Check if the connection is active."""
        return self._connected and self.ws is not None
