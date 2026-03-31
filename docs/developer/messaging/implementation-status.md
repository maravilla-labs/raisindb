# Messaging System Implementation Status

Last Updated: 2025-01-18

## Overview

This document tracks the current implementation status of the messaging system.
Refer to other docs in this folder for architecture and usage guides.

---

## Implementation Status

### Node Types

| Node Type | Status | File |
|-----------|--------|------|
| raisin:Message | ✅ Implemented | `crates/raisin-core/global_nodetypes/raisin_message.yaml` |
| raisin:MessageFolder | ✅ Implemented | `crates/raisin-core/global_nodetypes/raisin_message_folder.yaml` |
| raisin:InboxTask | ✅ Implemented | `crates/raisin-core/global_nodetypes/raisin_inbox_task.yaml` |
| raisin:Notification | ✅ Implemented | `crates/raisin-core/global_nodetypes/raisin_notification.yaml` |
| raisin:Conversation | ✅ Implemented | `builtin-packages/raisin-messaging/nodetypes/conversation.yaml` |

### Message Type Handlers

| Message Type | Trigger | Handler | Status |
|--------------|---------|---------|--------|
| relationship_request | ✅ | ✅ | Complete |
| relationship_response | ✅ | ✅ | Complete |
| ward_invitation | ✅ | ✅ | Complete |
| task_assignment | ✅ | ✅ | Complete |
| system_notification | ✅ | ✅ | Complete |
| stewardship_request | ✅ | ✅ | Complete |
| chat | ✅ | ✅ | Complete |

### Function Bindings (raisin.tasks.*)

| Binding | Status | Description |
|---------|--------|-------------|
| tasks.create() | ✅ Implemented | Create InboxTask in assignee's inbox |
| tasks.update() | ✅ Implemented | Update task status/response |
| tasks.complete() | ✅ Implemented | Mark task as completed |
| tasks.query() | ✅ Implemented | Query tasks by assignee/status |

### External Notifications

| Type | Status | Notes |
|------|--------|-------|
| WebSocket | ✅ Complete | Real-time via event bus |
| Push | ⚠️ Placeholder | Logs only - needs service integration |
| Email | ⚠️ Placeholder | Logs only - needs service integration |

---

## Not Yet Implemented

1. **Push notification service** - Placeholder only (logs instead of sending)
2. **Email notification service** - Placeholder only (logs instead of sending)

---

## File Reference

### Triggers

| Trigger | Path |
|---------|------|
| on-outbox-create | `builtin-packages/raisin-messaging/content/functions/triggers/on-outbox-create/` |
| process-chat | `builtin-packages/raisin-messaging/content/functions/triggers/process-chat/` |
| process-relationship-request | `builtin-packages/raisin-stewardship/content/functions/triggers/process-relationship-request/` |
| process-relationship-response | `builtin-packages/raisin-stewardship/content/functions/triggers/process-relationship-response/` |
| process-ward-invitation | `builtin-packages/raisin-stewardship/content/functions/triggers/process-ward-invitation/` |
| process-task-assignment | `builtin-packages/raisin-messaging/content/functions/triggers/process-task-assignment/` |
| process-system-notification | `builtin-packages/raisin-messaging/content/functions/triggers/process-system-notification/` |
| process-stewardship-request | `builtin-packages/raisin-stewardship/content/functions/triggers/process-stewardship-request/` |

### Handlers

| Handler | Path |
|---------|------|
| route-message | `builtin-packages/raisin-messaging/content/functions/lib/messaging/handlers/route-message/` |
| handle-chat | `builtin-packages/raisin-messaging/content/functions/lib/messaging/handlers/handle-chat/` |
| handle-relationship-request | `builtin-packages/raisin-stewardship/content/functions/lib/stewardship/handlers/handle-relationship-request/` |
| handle-relationship-response | `builtin-packages/raisin-stewardship/content/functions/lib/stewardship/handlers/handle-relationship-response/` |
| handle-ward-invitation | `builtin-packages/raisin-stewardship/content/functions/lib/stewardship/handlers/handle-ward-invitation/` |
| handle-task-assignment | `builtin-packages/raisin-messaging/content/functions/lib/messaging/handlers/handle-task-assignment/` |
| handle-system-notification | `builtin-packages/raisin-messaging/content/functions/lib/messaging/handlers/handle-system-notification/` |
| handle-stewardship-request | `builtin-packages/raisin-stewardship/content/functions/lib/stewardship/handlers/handle-stewardship-request/` |
| send-push-notification | `builtin-packages/raisin-messaging/content/functions/lib/messaging/handlers/send-push-notification/` (placeholder) |
| send-email-notification | `builtin-packages/raisin-messaging/content/functions/lib/messaging/handlers/send-email-notification/` (placeholder) |

### Rust Function Bindings

| File | Purpose |
|------|---------|
| `crates/raisin-functions/src/api/callbacks.rs` | Callback type definitions (TaskUpdateCallback, TaskCompleteCallback, TaskQueryCallback) |
| `crates/raisin-functions/src/api/mod.rs` | RaisinDbApi trait methods (task_update, task_complete, task_query) |
| `crates/raisin-functions/src/api/raisindb.rs` | API implementations |
| `crates/raisin-functions/src/execution/callbacks/tasks.rs` | Callback factory functions |
| `crates/raisin-functions/src/execution/callbacks/mod.rs` | Callback wiring |
| `crates/raisin-functions/src/runtime/bindings/methods/tasks.rs` | JS runtime bindings |

---

## Architecture Summary

```
Message Creation (outbox)
         │
         ▼
┌─────────────────────┐
│ on-outbox-create    │  Routes to type-specific handler
│ (router trigger)    │
└─────────────────────┘
         │
         ▼
┌─────────────────────┐
│ process-*-*         │  Type-specific triggers
│ (message triggers)  │
└─────────────────────┘
         │
         ▼
┌─────────────────────┐
│ handle-*            │  Business logic handlers
│ (handlers)          │
└─────────────────────┘
         │
         ├──────────────────────┬──────────────────────┐
         ▼                      ▼                      ▼
┌─────────────────┐    ┌─────────────────┐    ┌─────────────────┐
│ Recipient Inbox │    │ WebSocket Event │    │ External Notify │
│ (Message node)  │    │ (Real-time)     │    │ (Push/Email)    │
└─────────────────┘    └─────────────────┘    └─────────────────┘
```

---

## Related Documentation

- [Architecture](./architecture.md) - System design and message flow
- [Message Types](./message-types.md) - Built-in message type specifications
- [Triggers](./triggers.md) - Trigger-based processing system
- [Notifications](./notifications.md) - Real-time delivery implementation
- [Extending](./extending.md) - Adding custom message types
