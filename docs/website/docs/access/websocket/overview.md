---
sidebar_position: 1
---

# WebSocket & Client SDK

RaisinDB ships with a MessagePack WebSocket gateway and an official TypeScript client (`packages/raisin-client-js`). The protocol mirrors the `RequestEnvelope`/`ResponseEnvelope` definitions in `packages/raisin-client-js/src/protocol.ts` and the handlers inside `crates/raisin-transport-ws`.

## Connection Lifecycle

The `Connection` class (`src/connection.ts`) abstracts browser vs Node environments, auto-reconnect, and heartbeat logic:

```ts
import { Connection } from '@raisindb/client';

const conn = new Connection('ws://localhost:8080/ws', {
  heartbeatInterval: 30000,
});

await conn.connect();
conn.on('message', handleMessage);
```

- **Heartbeat** – 30s ping/pong keeps sessions alive.
- **Reconnects** – exponential backoff via `ReconnectManager`.
- **Binary mode** – every frame is MessagePack encoded (set via `ws.binaryType = 'arraybuffer'`).

## Envelope Format

```ts
interface RequestEnvelope {
  request_id: string;
  type: RequestType;
  context: {
    tenant_id: string;
    repository?: string;
    branch?: string;
    workspace?: string;
    revision?: string;
    transaction_id?: string;
  };
  payload: unknown;
}
```

The `ResponseEnvelope` echoes `request_id`, sets `status` (`success` or `error`), and carries `result` or an `error` structure.

## Implemented Request Types

`RequestType` enumerates every server capability (see `protocol.ts`). Highlights:

- **Nodes** – `NodeCreate`, `NodeUpdate`, `NodeDelete`, `NodeGet`, `NodeQuery`, `NodeCopyTree`, `NodeReorder`.
- **Tree helpers** – `NodeListChildren`, `NodeGetTree`, `NodeMoveChildBefore/After`.
- **Property ops** – `PropertyGet`, `PropertyUpdate`.
- **Relations** – `RelationAdd`, `RelationRemove`, `RelationsGet`.
- **Workspaces/Branches/Tags** – CRUD endpoints mirroring the REST routes.
- **NodeTypes/Archetypes/ElementTypes** – schema management commands.
- **Search** – `FullTextSearch`, `VectorSearch`.
- **SQL** – `SqlQuery` executes the same planner as `/api/sql/{repo}`.
- **Events** – `Subscribe`/`Unsubscribe` register live update feeds.

Keep the list in sync with the enum before documenting new behavior.

## Sending a Request

```ts
import { encode } from '@msgpack/msgpack';
import { randomUUID } from 'crypto';
import { RequestType } from '@raisindb/client/protocol';

const env = {
  request_id: randomUUID(),
  type: RequestType.NodeGet,
  context: {
    tenant_id: 'default',
    repository: 'cms',
    branch: 'main',
    workspace: 'content',
  },
  payload: { path: '/articles/hello-world' },
};

connection.send(encode(env));
```

## Handling Responses

Responses arrive as MessagePack-encoded buffers:

```ts
connection.on('message', (buf: ArrayBuffer) => {
  const msg = decode(new Uint8Array(buf));
  if (msg.status === 'error') {
    console.error(msg.error?.code, msg.error?.message);
    return;
  }
  console.log('result', msg.result);
});
```

Errors use the same shape as the HTTP API (`code`, `message`, `details`, `timestamp`), so clients can share parsing logic.

## Authentication

When running with RocksDB and auth enabled, obtain a token via `/api/raisindb/sys/{tenant}/auth` and include it in the WebSocket `Authorization` header. The Axum middleware enforces this before handing control to `raisin-transport-ws`.

## Examples

- **`packages/raisin-client-js/examples/social-feed`** – browser app that streams live feed updates.
- **`packages/raisin-client-js/examples/social-feed-ssr`** – SSR example using the same SDK in Node.js.

Use these samples as templates for your own WebSocket integrations.
