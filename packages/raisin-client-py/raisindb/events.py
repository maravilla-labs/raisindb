"""Event subscription management."""

from typing import TYPE_CHECKING, Callable, List, Optional

from .protocol import RequestType, EventMessage, SubscriptionFilters

if TYPE_CHECKING:
    from .workspace import Workspace


class Subscription:
    """Represents an active event subscription."""

    def __init__(self, subscription_id: str, manager: "EventSubscriptions"):
        """
        Initialize a subscription.

        Args:
            subscription_id: Unique subscription ID
            manager: Parent EventSubscriptions instance
        """
        self.subscription_id = subscription_id
        self.manager = manager
        self._active = True

    async def unsubscribe(self) -> None:
        """Unsubscribe from events."""
        if not self._active:
            return

        context = self.manager.workspace.get_context()
        await self.manager.workspace.database.client.request_tracker.send_request(
            RequestType.UNSUBSCRIBE,
            context,
            {"subscription_id": self.subscription_id},
        )

        # Unregister event handler
        self.manager.workspace.database.client._unregister_event_handler(
            self.subscription_id
        )
        self._active = False

    @property
    def is_active(self) -> bool:
        """Check if subscription is active."""
        return self._active


class EventSubscriptions:
    """Manages event subscriptions for a workspace."""

    def __init__(self, workspace: "Workspace"):
        """
        Initialize event subscriptions.

        Args:
            workspace: Parent Workspace instance
        """
        self.workspace = workspace

    async def subscribe(
        self,
        callback: Callable[[EventMessage], None],
        path: Optional[str] = None,
        event_types: Optional[List[str]] = None,
        node_type: Optional[str] = None,
    ) -> Subscription:
        """
        Subscribe to events with filtering.

        Args:
            callback: Function to call when events occur
            path: Path pattern filter (supports wildcards: /folder/*, /folder/**)
            event_types: List of event types to filter (e.g., ["node:created", "node:updated"])
            node_type: Node type filter

        Returns:
            Subscription instance

        Example:
            ```python
            async def on_event(event: EventMessage):
                print(f"Event: {event.event_type}")
                print(f"Payload: {event.payload}")

            # Subscribe to all node creations in /blog/
            sub = await workspace.events().subscribe(
                on_event,
                path="/blog/*",
                event_types=["node:created"]
            )

            # Later, unsubscribe
            await sub.unsubscribe()
            ```
        """
        filters = SubscriptionFilters(
            workspace=self.workspace.name,
            path=path,
            event_types=event_types,
            node_type=node_type,
        )

        context = self.workspace.get_context()
        result = await self.workspace.database.client.request_tracker.send_request(
            RequestType.SUBSCRIBE, context, {"filters": filters.to_dict()}
        )

        subscription_id = result["subscription_id"]

        # Register event handler
        self.workspace.database.client._register_event_handler(
            subscription_id, callback
        )

        return Subscription(subscription_id, self)

    async def subscribe_to_path(
        self, path: str, callback: Callable[[EventMessage], None]
    ) -> Subscription:
        """
        Subscribe to all events on a specific path.

        Args:
            path: Path pattern (supports wildcards)
            callback: Event handler function

        Returns:
            Subscription instance
        """
        return await self.subscribe(callback, path=path)

    async def subscribe_to_type(
        self, event_type: str, callback: Callable[[EventMessage], None]
    ) -> Subscription:
        """
        Subscribe to a specific event type.

        Args:
            event_type: Event type (e.g., "node:created")
            callback: Event handler function

        Returns:
            Subscription instance
        """
        return await self.subscribe(callback, event_types=[event_type])

    async def subscribe_to_node_type(
        self, node_type: str, callback: Callable[[EventMessage], None]
    ) -> Subscription:
        """
        Subscribe to events for a specific node type.

        Args:
            node_type: Node type
            callback: Event handler function

        Returns:
            Subscription instance
        """
        return await self.subscribe(callback, node_type=node_type)
