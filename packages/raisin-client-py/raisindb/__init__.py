"""RaisinDB Python Client - A Git-like document database client."""

from .client import RaisinClient
from .database import Database
from .workspace import Workspace
from .nodes import NodeOperations
from .node_types import NodeTypes
from .archetypes import Archetypes
from .element_types import ElementTypes
from .branches import Branches
from .tags import Tags
from .protocol import (
    RequestType,
    ResponseStatus,
    RequestContext,
    AuthCredentials,
    Node,
)
from .exceptions import (
    RaisinDBError,
    ConnectionError,
    AuthenticationError,
    RequestError,
    TimeoutError,
)

__version__ = "0.1.0"
__all__ = [
    "RaisinClient",
    "Database",
    "Workspace",
    "NodeOperations",
    "NodeTypes",
    "Archetypes",
    "ElementTypes",
    "Branches",
    "Tags",
    "RequestType",
    "ResponseStatus",
    "RequestContext",
    "AuthCredentials",
    "Node",
    "RaisinDBError",
    "ConnectionError",
    "AuthenticationError",
    "RequestError",
    "TimeoutError",
]
