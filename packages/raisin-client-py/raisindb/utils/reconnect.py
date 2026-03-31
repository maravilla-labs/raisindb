"""Auto-reconnection logic with exponential backoff."""

import asyncio
from typing import Optional


class ReconnectManager:
    """Manages reconnection attempts with exponential backoff."""

    def __init__(
        self,
        initial_delay: float = 1.0,
        max_delay: float = 30.0,
        max_attempts: Optional[int] = None,
    ):
        """
        Initialize the reconnect manager.

        Args:
            initial_delay: Initial delay between reconnect attempts in seconds
            max_delay: Maximum delay between reconnect attempts in seconds
            max_attempts: Maximum number of reconnect attempts (None for unlimited)
        """
        self.initial_delay = initial_delay
        self.max_delay = max_delay
        self.max_attempts = max_attempts
        self.current_delay = initial_delay
        self.attempt = 0
        self.is_reconnecting = False

    def reset(self) -> None:
        """Reset reconnection state after successful connection."""
        self.current_delay = self.initial_delay
        self.attempt = 0
        self.is_reconnecting = False

    def should_reconnect(self) -> bool:
        """Check if another reconnection attempt should be made."""
        if self.max_attempts is None:
            return True
        return self.attempt < self.max_attempts

    def next_delay(self) -> float:
        """Get the next delay duration and increment attempt counter."""
        delay = self.current_delay
        self.current_delay = min(self.current_delay * 2, self.max_delay)
        self.attempt += 1
        return delay

    async def wait_before_reconnect(self) -> None:
        """Wait for the appropriate delay before attempting reconnection."""
        delay = self.next_delay()
        await asyncio.sleep(delay)
