"""Request tracking and promise resolution."""

import asyncio
from typing import Any, Callable, Dict, Optional
from uuid import uuid4

from ..protocol import (
    RequestEnvelope,
    ResponseEnvelope,
    ResponseStatus,
    RequestType,
    RequestContext,
)
from ..exceptions import RequestError, TimeoutError


class PendingRequest:
    """Represents a pending request awaiting response."""

    def __init__(self, request_id: str, timeout: float):
        """Initialize a pending request."""
        self.request_id = request_id
        self.future: asyncio.Future[Any] = asyncio.Future()
        self.timeout = timeout
        self.timeout_handle: Optional[asyncio.TimerHandle] = None

    def set_result(self, result: Any) -> None:
        """Set the successful result."""
        if self.timeout_handle:
            self.timeout_handle.cancel()
        if not self.future.done():
            self.future.set_result(result)

    def set_error(self, error: Exception) -> None:
        """Set an error result."""
        if self.timeout_handle:
            self.timeout_handle.cancel()
        if not self.future.done():
            self.future.set_exception(error)

    def set_timeout(self) -> None:
        """Mark the request as timed out."""
        if not self.future.done():
            self.future.set_exception(TimeoutError(f"Request {self.request_id} timed out"))


class RequestTracker:
    """Tracks pending requests and resolves them when responses arrive."""

    def __init__(
        self,
        send_func: Callable[[bytes], None],
        default_timeout: float = 30.0,
    ):
        """
        Initialize the request tracker.

        Args:
            send_func: Function to send encoded request bytes
            default_timeout: Default timeout for requests in seconds
        """
        self.send_func = send_func
        self.default_timeout = default_timeout
        self.pending_requests: Dict[str, PendingRequest] = {}
        self._lock = asyncio.Lock()

    async def send_request(
        self,
        request_type: RequestType,
        context: RequestContext,
        payload: Any,
        timeout: Optional[float] = None,
    ) -> Any:
        """
        Send a request and wait for the response.

        Args:
            request_type: Type of request
            context: Request context
            payload: Request payload
            timeout: Timeout in seconds (uses default if None)

        Returns:
            The response result

        Raises:
            RequestError: If the request fails
            TimeoutError: If the request times out
        """
        request_id = str(uuid4())
        timeout = timeout or self.default_timeout

        # Create pending request
        pending = PendingRequest(request_id, timeout)

        async with self._lock:
            self.pending_requests[request_id] = pending

        # Set timeout
        loop = asyncio.get_event_loop()
        pending.timeout_handle = loop.call_later(
            timeout, lambda: self._handle_timeout(request_id)
        )

        try:
            # Create and send request envelope
            from ..protocol import encode_request

            envelope = RequestEnvelope(
                request_id=request_id,
                request_type=request_type,
                context=context,
                payload=payload,
            )

            self.send_func(encode_request(envelope))

            # Wait for response
            result = await pending.future
            return result

        finally:
            # Clean up
            async with self._lock:
                self.pending_requests.pop(request_id, None)

    async def handle_response(self, response: ResponseEnvelope) -> None:
        """
        Handle an incoming response.

        Args:
            response: The response envelope
        """
        async with self._lock:
            pending = self.pending_requests.get(response.request_id)

        if not pending:
            # Response for unknown request (might have timed out)
            return

        if response.status == ResponseStatus.ERROR:
            if response.error:
                error = RequestError(
                    response.error.message, response.error.code, response.error.details
                )
                pending.set_error(error)
            else:
                pending.set_error(RequestError("Unknown error", "UNKNOWN"))
        elif response.status in (ResponseStatus.SUCCESS, ResponseStatus.COMPLETE):
            pending.set_result(response.result)
        elif response.status == ResponseStatus.STREAMING:
            # For now, we'll just return the first chunk
            # TODO: Implement proper streaming support
            pending.set_result(response.result)

    def _handle_timeout(self, request_id: str) -> None:
        """Handle request timeout."""

        async def _do_timeout() -> None:
            async with self._lock:
                pending = self.pending_requests.get(request_id)
                if pending:
                    pending.set_timeout()

        asyncio.create_task(_do_timeout())
