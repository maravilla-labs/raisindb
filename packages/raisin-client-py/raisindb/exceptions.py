"""Exception classes for RaisinDB client."""


class RaisinDBError(Exception):
    """Base exception for all RaisinDB client errors."""

    pass


class ConnectionError(RaisinDBError):
    """Raised when connection to server fails."""

    pass


class AuthenticationError(RaisinDBError):
    """Raised when authentication fails."""

    pass


class RequestError(RaisinDBError):
    """Raised when a request fails."""

    def __init__(self, message: str, code: str, details: any = None):
        super().__init__(message)
        self.code = code
        self.details = details


class TimeoutError(RaisinDBError):
    """Raised when a request times out."""

    pass


class SubscriptionError(RaisinDBError):
    """Raised when subscription operation fails."""

    pass
