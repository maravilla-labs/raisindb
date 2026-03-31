"""Protocol definitions for RaisinDB WebSocket communication."""

from dataclasses import dataclass, field, asdict
from datetime import datetime
from enum import Enum
from typing import Any, Dict, List, Optional, Union
import msgpack


class RequestType(str, Enum):
    """Types of requests supported by the WebSocket protocol."""

    # Authentication
    AUTHENTICATE = "authenticate"
    REFRESH_TOKEN = "refresh_token"

    # Node operations
    NODE_CREATE = "node_create"
    NODE_UPDATE = "node_update"
    NODE_DELETE = "node_delete"
    NODE_GET = "node_get"
    NODE_QUERY = "node_query"
    NODE_QUERY_BY_PATH = "node_query_by_path"
    NODE_QUERY_BY_PROPERTY = "node_query_by_property"

    # Tree operations
    NODE_LIST_CHILDREN = "node_list_children"
    NODE_GET_TREE = "node_get_tree"
    NODE_GET_TREE_FLAT = "node_get_tree_flat"

    # Node manipulation operations
    NODE_MOVE = "node_move"
    NODE_RENAME = "node_rename"
    NODE_COPY = "node_copy"
    NODE_COPY_TREE = "node_copy_tree"
    NODE_REORDER = "node_reorder"
    NODE_MOVE_CHILD_BEFORE = "node_move_child_before"
    NODE_MOVE_CHILD_AFTER = "node_move_child_after"

    # Property path operations
    PROPERTY_GET = "property_get"
    PROPERTY_UPDATE = "property_update"

    # Relationship operations
    RELATION_ADD = "relation_add"
    RELATION_REMOVE = "relation_remove"
    RELATIONS_GET = "relations_get"

    # SQL queries
    SQL_QUERY = "sql_query"

    # Workspace operations
    WORKSPACE_CREATE = "workspace_create"
    WORKSPACE_GET = "workspace_get"
    WORKSPACE_LIST = "workspace_list"
    WORKSPACE_DELETE = "workspace_delete"
    WORKSPACE_UPDATE = "workspace_update"

    # NodeType operations
    NODE_TYPE_CREATE = "node_type_create"
    NODE_TYPE_GET = "node_type_get"
    NODE_TYPE_LIST = "node_type_list"
    NODE_TYPE_UPDATE = "node_type_update"
    NODE_TYPE_DELETE = "node_type_delete"
    NODE_TYPE_PUBLISH = "node_type_publish"
    NODE_TYPE_UNPUBLISH = "node_type_unpublish"
    NODE_TYPE_VALIDATE = "node_type_validate"
    NODE_TYPE_GET_RESOLVED = "node_type_get_resolved"

    # Archetype operations
    ARCHETYPE_CREATE = "archetype_create"
    ARCHETYPE_GET = "archetype_get"
    ARCHETYPE_LIST = "archetype_list"
    ARCHETYPE_UPDATE = "archetype_update"
    ARCHETYPE_DELETE = "archetype_delete"
    ARCHETYPE_PUBLISH = "archetype_publish"
    ARCHETYPE_UNPUBLISH = "archetype_unpublish"

    # ElementType operations
    ELEMENT_TYPE_CREATE = "element_type_create"
    ELEMENT_TYPE_GET = "element_type_get"
    ELEMENT_TYPE_LIST = "element_type_list"
    ELEMENT_TYPE_UPDATE = "element_type_update"
    ELEMENT_TYPE_DELETE = "element_type_delete"
    ELEMENT_TYPE_PUBLISH = "element_type_publish"
    ELEMENT_TYPE_UNPUBLISH = "element_type_unpublish"

    # Branch operations
    BRANCH_CREATE = "branch_create"
    BRANCH_GET = "branch_get"
    BRANCH_LIST = "branch_list"
    BRANCH_DELETE = "branch_delete"
    BRANCH_GET_HEAD = "branch_get_head"
    BRANCH_UPDATE_HEAD = "branch_update_head"
    BRANCH_MERGE = "branch_merge"
    BRANCH_COMPARE = "branch_compare"

    # Tag operations
    TAG_CREATE = "tag_create"
    TAG_GET = "tag_get"
    TAG_LIST = "tag_list"
    TAG_DELETE = "tag_delete"

    # Event subscriptions
    SUBSCRIBE = "subscribe"
    UNSUBSCRIBE = "unsubscribe"

    # Repository management
    REPOSITORY_CREATE = "repository_create"
    REPOSITORY_GET = "repository_get"
    REPOSITORY_LIST = "repository_list"
    REPOSITORY_UPDATE = "repository_update"
    REPOSITORY_DELETE = "repository_delete"


class ResponseStatus(str, Enum):
    """Response status codes."""

    SUCCESS = "success"
    ERROR = "error"
    STREAMING = "streaming"
    COMPLETE = "complete"
    ACKNOWLEDGED = "acknowledged"


@dataclass
class RequestContext:
    """Context for a request (tenant, repository, workspace, branch, revision)."""

    tenant_id: str
    repository: Optional[str] = None
    branch: Optional[str] = None
    workspace: Optional[str] = None
    revision: Optional[str] = None

    def to_dict(self) -> Dict[str, Any]:
        """Convert to dictionary for serialization."""
        result = {"tenant_id": self.tenant_id}
        if self.repository is not None:
            result["repository"] = self.repository
        if self.branch is not None:
            result["branch"] = self.branch
        if self.workspace is not None:
            result["workspace"] = self.workspace
        if self.revision is not None:
            result["revision"] = self.revision
        return result


@dataclass
class RequestEnvelope:
    """Request envelope sent to server."""

    request_id: str
    request_type: RequestType
    context: RequestContext
    payload: Any

    def to_dict(self) -> Dict[str, Any]:
        """Convert to dictionary for MessagePack serialization."""
        return {
            "request_id": self.request_id,
            "type": self.request_type.value,
            "context": self.context.to_dict(),
            "payload": self.payload,
        }


@dataclass
class ErrorInfo:
    """Error information in response."""

    code: str
    message: str
    details: Optional[Any] = None


@dataclass
class ResponseMetadata:
    """Metadata for streaming and pagination."""

    chunk: Optional[int] = None
    total_chunks: Optional[int] = None
    total: Optional[int] = None
    has_more: Optional[bool] = None
    credits_consumed: Optional[int] = None


@dataclass
class ResponseEnvelope:
    """Response envelope received from server."""

    request_id: str
    status: ResponseStatus
    result: Optional[Any] = None
    error: Optional[ErrorInfo] = None
    metadata: Optional[ResponseMetadata] = None

    @classmethod
    def from_dict(cls, data: Dict[str, Any]) -> "ResponseEnvelope":
        """Create from dictionary received via MessagePack."""
        error = None
        if data.get("error"):
            error = ErrorInfo(
                code=data["error"]["code"],
                message=data["error"]["message"],
                details=data["error"].get("details"),
            )

        metadata = None
        if data.get("metadata"):
            metadata = ResponseMetadata(
                chunk=data["metadata"].get("chunk"),
                total_chunks=data["metadata"].get("total_chunks"),
                total=data["metadata"].get("total"),
                has_more=data["metadata"].get("has_more"),
                credits_consumed=data["metadata"].get("credits_consumed"),
            )

        return cls(
            request_id=data["request_id"],
            status=ResponseStatus(data["status"]),
            result=data.get("result"),
            error=error,
            metadata=metadata,
        )


@dataclass
class EventMessage:
    """Server-initiated event message."""

    event_id: str
    subscription_id: str
    event_type: str
    payload: Any
    timestamp: datetime

    @classmethod
    def from_dict(cls, data: Dict[str, Any]) -> "EventMessage":
        """Create from dictionary received via MessagePack."""
        return cls(
            event_id=data["event_id"],
            subscription_id=data["subscription_id"],
            event_type=data["event_type"],
            payload=data["payload"],
            timestamp=datetime.fromisoformat(data["timestamp"].replace("Z", "+00:00")),
        )


@dataclass
class AuthCredentials:
    """Authentication credentials."""

    username: str
    password: str


@dataclass
class RelationRef:
    """A reference to a related node in the graph database."""

    target: str
    workspace: str
    target_node_type: str
    relation_type: str
    weight: Optional[float] = None


@dataclass
class Node:
    """
    A content node in RaisinDB's hierarchical structure.

    Nodes are the primary content entities, organized in a tree structure within workspaces.
    Each node has a type (NodeType) that defines its schema, allowed children, and behavior.

    Example:
        >>> node = Node(
        ...     id="node-123",
        ...     name="My Page",
        ...     path="/content/my-page",
        ...     node_type="Page",
        ...     properties={"title": "Welcome", "published": True}
        ... )
    """

    # Required fields
    id: str
    name: str
    path: str
    node_type: str

    # Properties and children
    properties: Dict[str, Any] = field(default_factory=dict)
    children: List[str] = field(default_factory=list)
    relations: List[RelationRef] = field(default_factory=list)

    # Ordering and hierarchy
    order_key: str = "a"
    parent: Optional[str] = None
    has_children: Optional[bool] = None

    # Optional metadata
    archetype: Optional[str] = None
    version: int = 1

    # Timestamps
    created_at: Optional[datetime] = None
    updated_at: Optional[datetime] = None
    published_at: Optional[datetime] = None

    # User tracking
    created_by: Optional[str] = None
    updated_by: Optional[str] = None
    published_by: Optional[str] = None

    # Multi-tenancy and workspace
    tenant_id: Optional[str] = None
    workspace: Optional[str] = None
    owner_id: Optional[str] = None

    # Translations
    translations: Optional[Dict[str, Any]] = None

    def get_property(self, key: str, default: Any = None) -> Any:
        """Get a property value by key."""
        return self.properties.get(key, default)

    def set_property(self, key: str, value: Any) -> "Node":
        """Set a property value (chainable)."""
        self.properties[key] = value
        return self

    def get_parent_path(self) -> Optional[str]:
        """Get the parent path derived from this node's path."""
        if not self.path or self.path == "/":
            return None
        last_slash = self.path.rfind("/")
        if last_slash <= 0:
            return "/"
        return self.path[:last_slash]

    def is_published(self) -> bool:
        """Check if this node is published."""
        return self.published_at is not None

    def add_relation(
        self,
        target: str,
        workspace: str,
        relation_type: str,
        target_node_type: str = "",
        weight: Optional[float] = None,
    ) -> "Node":
        """Add a relation to another node (chainable)."""
        self.relations.append(
            RelationRef(
                target=target,
                workspace=workspace,
                target_node_type=target_node_type,
                relation_type=relation_type,
                weight=weight,
            )
        )
        return self

    def get_relations_by_type(self, relation_type: str) -> List[RelationRef]:
        """Get all relations of a specific type."""
        return [r for r in self.relations if r.relation_type == relation_type]

    @classmethod
    def from_dict(cls, data: Dict[str, Any]) -> "Node":
        """Create a Node from a dictionary."""
        # Parse timestamps
        created_at = None
        if data.get("created_at"):
            created_at = datetime.fromisoformat(data["created_at"].replace("Z", "+00:00"))

        updated_at = None
        if data.get("updated_at"):
            updated_at = datetime.fromisoformat(data["updated_at"].replace("Z", "+00:00"))

        published_at = None
        if data.get("published_at"):
            published_at = datetime.fromisoformat(data["published_at"].replace("Z", "+00:00"))

        # Parse relations
        relations = []
        for rel_data in data.get("relations", []):
            relations.append(
                RelationRef(
                    target=rel_data["target"],
                    workspace=rel_data["workspace"],
                    target_node_type=rel_data.get("target_node_type", ""),
                    relation_type=rel_data.get("relation_type", ""),
                    weight=rel_data.get("weight"),
                )
            )

        return cls(
            id=data.get("id", data.get("node_id", "")),
            name=data.get("name", ""),
            path=data["path"],
            node_type=data["node_type"],
            properties=data.get("properties", {}),
            children=data.get("children", []),
            relations=relations,
            order_key=data.get("order_key", "a"),
            parent=data.get("parent"),
            has_children=data.get("has_children"),
            archetype=data.get("archetype"),
            version=data.get("version", 1),
            created_at=created_at,
            updated_at=updated_at,
            published_at=published_at,
            created_by=data.get("created_by"),
            updated_by=data.get("updated_by"),
            published_by=data.get("published_by"),
            tenant_id=data.get("tenant_id"),
            workspace=data.get("workspace"),
            owner_id=data.get("owner_id"),
            translations=data.get("translations"),
        )


@dataclass
class WorkspaceInfo:
    """Workspace information."""

    name: str
    description: Optional[str] = None
    allowed_node_types: List[str] = field(default_factory=list)
    allowed_root_node_types: List[str] = field(default_factory=list)
    created_at: Optional[datetime] = None
    updated_at: Optional[datetime] = None

    @classmethod
    def from_dict(cls, data: Dict[str, Any]) -> "WorkspaceInfo":
        """Create from dictionary."""
        created_at = None
        if data.get("created_at"):
            created_at = datetime.fromisoformat(data["created_at"].replace("Z", "+00:00"))

        updated_at = None
        if data.get("updated_at"):
            updated_at = datetime.fromisoformat(data["updated_at"].replace("Z", "+00:00"))

        return cls(
            name=data["name"],
            description=data.get("description"),
            allowed_node_types=data.get("allowed_node_types", []),
            allowed_root_node_types=data.get("allowed_root_node_types", []),
            created_at=created_at,
            updated_at=updated_at,
        )


@dataclass
class SubscriptionFilters:
    """Filters for event subscriptions."""

    workspace: Optional[str] = None
    path: Optional[str] = None
    event_types: Optional[List[str]] = None
    node_type: Optional[str] = None

    def to_dict(self) -> Dict[str, Any]:
        """Convert to dictionary for serialization."""
        result: Dict[str, Any] = {}
        if self.workspace is not None:
            result["workspace"] = self.workspace
        if self.path is not None:
            result["path"] = self.path
        if self.event_types is not None:
            result["event_types"] = self.event_types
        if self.node_type is not None:
            result["node_type"] = self.node_type
        return result


@dataclass
class SqlResult:
    """Result of a SQL query."""

    columns: List[str]
    rows: List[List[Any]]
    row_count: int


def encode_request(request: RequestEnvelope) -> bytes:
    """Encode a request to MessagePack format."""
    return msgpack.packb(request.to_dict(), use_bin_type=True)


def decode_response(data: bytes) -> Union[ResponseEnvelope, EventMessage]:
    """Decode MessagePack data to response or event."""
    decoded = msgpack.unpackb(data, raw=False)

    # Check if it's an event (has subscription_id)
    if "subscription_id" in decoded:
        return EventMessage.from_dict(decoded)

    # Otherwise it's a response
    return ResponseEnvelope.from_dict(decoded)
