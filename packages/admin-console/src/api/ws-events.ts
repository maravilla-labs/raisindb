// SPDX-License-Identifier: BSL-1.1

/**
 * WebSocket client for the RaisinDB node-event stream.
 *
 * Why this exists at all: everything else realtime in this console is SSE over
 * `/management/*`, and `/management/*` is **404'd at the Caddy edge for tenant
 * subdomains**. A tenant therefore cannot watch anything live. `/ws/{repo}` is
 * not under `/management/`, passes the tenant edge, and its fan-out is
 * RLS-filtered server-side, so it is the only realtime surface a tenant can
 * actually use.
 *
 * Protocol notes that are easy to get wrong (see `raisin-transport-ws`):
 *  - **Frames are MessagePack binary, not JSON.** Text frames are logged and
 *    dropped without a response. See {@link msgpackEncode}.
 *  - **Auth is in-band.** The upgrade accepts `Authorization: Bearer` or a
 *    subprotocol, and a browser can set neither (the server never echoes a
 *    selected subprotocol, so offering one fails the handshake browser-side).
 *    The first frame we send is therefore `authenticate_jwt`.
 *  - **The connection does not pin a branch and subscriptions cannot filter by
 *    one.** `SubscriptionFilters` has no branch field; the server uses the
 *    branch only to build the RLS scope. Events arrive for every branch of the
 *    repo, so callers that care must compare `payload.branch` themselves — see
 *    {@link NodeEventPayload.branch}.
 *  - `context.repository` must match the repo in the URL or the request is
 *    rejected with `REPOSITORY_SCOPE_MISMATCH`.
 *  - `path` filters are **globs**, not prefixes: `*` is one segment, `**` is
 *    recursive, and `/x/**` excludes `/x` itself.
 */

import { msgpackDecode, msgpackEncode } from './msgpack'
import { getCurrentAuthToken, getCurrentTenantId } from './bootstrap'

// ============================================================
// Wire types
// ============================================================

/** Filters for a `subscribe` request. Every field is AND-ed; absent matches all. */
export interface SubscriptionFilters {
  workspace?: string
  /** Glob, not a prefix. `*` = one segment, `**` = recursive. */
  path?: string
  event_types?: string[]
  node_type?: string
  /** Ship the full node object on each event. Defaults to false server-side. */
  include_node?: boolean
}

/** Payload of a `node:*` event. */
export interface NodeEventPayload {
  tenant_id?: string
  repository_id?: string
  /**
   * Not filterable server-side — subscriptions have no branch field, so a
   * subscription receives events from every branch of the repo. Compare this
   * client-side when the branch matters.
   */
  branch?: string
  workspace_id?: string
  node_id?: string
  node_type?: string
  /** HLC, serialized as `"{timestamp_ms}-{counter}"`. */
  revision?: string
  path?: string
  /**
   * Rust `Debug` formatting of the event kind, so structured variants read as
   * e.g. `PropertyChanged { property: "title" }`. Parse the sibling fields
   * instead of this string.
   */
  kind?: string
  /**
   * Producer-supplied. `HashMap<String, JsonValue>` on the server — values are
   * arbitrary JSON, not strings, and the map may be absent entirely. The
   * transport guarantees no particular key.
   */
  metadata?: Record<string, unknown> | null
  /** Present only when the subscription set `include_node`. */
  node?: Record<string, unknown>
  [key: string]: unknown
}

export interface NodeEvent {
  event_id: string
  subscription_id: string
  /** `node:created` | `node:updated` | `node:deleted` | … */
  event_type: string
  timestamp: string
  payload: NodeEventPayload
}

interface ResponseEnvelope {
  request_id: string
  status: 'success' | 'error' | 'streaming' | 'complete' | 'acknowledged'
  result?: unknown
  error?: { code?: string; message?: string }
}

export type WsStatus = 'connecting' | 'live' | 'offline'

type EventHandler = (event: NodeEvent) => void
type StatusHandler = (status: WsStatus) => void

interface Registration {
  filters: SubscriptionFilters
  handler: EventHandler
  /** Server id, once subscribed. Cleared on disconnect so reconnect re-subscribes. */
  serverId?: string
}

// ============================================================
// Client
// ============================================================

/** Reconnect backoff, in ms. Capped so a long outage still recovers promptly. */
const BACKOFF_MS = [500, 1000, 2000, 5000, 10000, 15000]
const REQUEST_TIMEOUT_MS = 15000

/**
 * One socket per repository, shared by every subscriber on the page.
 *
 * Instances are cached by {@link getWsClient} so the mount detail view's two
 * subscriptions — config-node state writes and materialized item writes — share
 * a single connection rather than opening one each.
 */
export class RaisinWsClient {
  private ws: WebSocket | null = null
  private status: WsStatus = 'offline'
  private registrations = new Set<Registration>()
  private statusHandlers = new Set<StatusHandler>()
  private pending = new Map<string, { resolve: (v: unknown) => void; reject: (e: Error) => void; timer: number }>()
  private reconnectTimer: number | null = null
  private attempt = 0
  private closedByUs = false
  private seq = 0

  constructor(private readonly repo: string) {}

  // ---- lifecycle ----

  private url(): string {
    const proto = window.location.protocol === 'https:' ? 'wss:' : 'ws:'
    return `${proto}//${window.location.host}/ws/${encodeURIComponent(this.repo)}`
  }

  private setStatus(next: WsStatus) {
    if (this.status === next) return
    this.status = next
    for (const h of this.statusHandlers) h(next)
  }

  getStatus(): WsStatus {
    return this.status
  }

  onStatus(handler: StatusHandler): () => void {
    this.statusHandlers.add(handler)
    handler(this.status)
    return () => this.statusHandlers.delete(handler)
  }

  private connect() {
    if (this.ws || this.closedByUs) return
    this.setStatus('connecting')

    let socket: WebSocket
    try {
      socket = new WebSocket(this.url())
    } catch {
      // Malformed URL or blocked scheme — treat as a normal outage so the
      // caller's polling fallback takes over rather than the page breaking.
      this.scheduleReconnect()
      return
    }
    socket.binaryType = 'arraybuffer'
    this.ws = socket

    socket.onopen = () => {
      this.attempt = 0
      void this.authenticateAndSubscribe()
    }

    socket.onmessage = (ev) => {
      if (!(ev.data instanceof ArrayBuffer)) return // text frames are not part of this protocol
      let decoded: unknown
      try {
        decoded = msgpackDecode(ev.data)
      } catch (e) {
        console.warn('[ws] undecodable frame', e)
        return
      }
      this.dispatch(decoded)
    }

    socket.onerror = () => {
      // `onclose` always follows; reconnect is scheduled there so it happens once.
    }

    socket.onclose = () => {
      this.ws = null
      this.failPending(new Error('WebSocket closed'))
      for (const reg of this.registrations) reg.serverId = undefined
      this.setStatus('offline')
      if (!this.closedByUs) this.scheduleReconnect()
    }
  }

  private scheduleReconnect() {
    if (this.reconnectTimer !== null || this.closedByUs) return
    if (this.registrations.size === 0) return // nothing to come back for
    const delay = BACKOFF_MS[Math.min(this.attempt, BACKOFF_MS.length - 1)]
    this.attempt++
    this.reconnectTimer = window.setTimeout(() => {
      this.reconnectTimer = null
      this.connect()
    }, delay)
  }

  private async authenticateAndSubscribe() {
    const token = getCurrentAuthToken()
    if (token) {
      try {
        await this.request('authenticate_jwt', { token })
      } catch (e) {
        // A rejected token is terminal for this socket — retrying the same
        // credential in a tight loop would just hammer the server. Stay offline
        // and let the caller's polling fallback carry the page.
        console.warn('[ws] authentication rejected', e)
        this.setStatus('offline')
        return
      }
    }
    // Without a token the connection keeps `deny_all` unless the tenant allows
    // anonymous access; the subscribe below then simply yields no events.

    this.setStatus('live')
    for (const reg of this.registrations) void this.sendSubscribe(reg)
  }

  /** Close the socket and drop all subscriptions. */
  destroy() {
    this.closedByUs = true
    if (this.reconnectTimer !== null) window.clearTimeout(this.reconnectTimer)
    this.reconnectTimer = null
    this.registrations.clear()
    this.statusHandlers.clear()
    this.failPending(new Error('client destroyed'))
    this.ws?.close()
    this.ws = null
    this.setStatus('offline')
  }

  // ---- request/response ----

  private failPending(err: Error) {
    for (const [, p] of this.pending) {
      window.clearTimeout(p.timer)
      p.reject(err)
    }
    this.pending.clear()
  }

  private request(type: string, payload: unknown): Promise<unknown> {
    const socket = this.ws
    if (!socket || socket.readyState !== WebSocket.OPEN) {
      return Promise.reject(new Error('not connected'))
    }
    const requestId = `ac-${Date.now().toString(36)}-${(this.seq++).toString(36)}`

    return new Promise((resolve, reject) => {
      const timer = window.setTimeout(() => {
        this.pending.delete(requestId)
        reject(new Error(`${type} timed out`))
      }, REQUEST_TIMEOUT_MS)
      this.pending.set(requestId, { resolve, reject, timer })

      try {
        socket.send(
          msgpackEncode({
            request_id: requestId,
            type,
            // `tenant_id` is mandatory; the server clamps it to the connection's
            // tenant anyway. `repository` MUST equal the repo in the URL.
            context: { tenant_id: getCurrentTenantId(), repository: this.repo },
            payload,
          }),
        )
      } catch (e) {
        window.clearTimeout(timer)
        this.pending.delete(requestId)
        reject(e instanceof Error ? e : new Error(String(e)))
      }
    })
  }

  private dispatch(msg: unknown) {
    if (!msg || typeof msg !== 'object') return
    const m = msg as Record<string, unknown>

    // Unsolicited hello frame right after upgrade — no request_id, no status.
    if (m.type === 'connected') return

    // Events are discriminated structurally, exactly as the JS SDK does it.
    if (typeof m.subscription_id === 'string' && typeof m.event_type === 'string') {
      // Reserved out-of-band channel: permissions changed, not a subscription
      // event. Ignored here rather than routed to a handler.
      if (m.subscription_id === '__permissions__') return
      const event = m as unknown as NodeEvent
      for (const reg of this.registrations) {
        if (reg.serverId === event.subscription_id) reg.handler(event)
      }
      return
    }

    if (typeof m.request_id !== 'string') return
    const p = this.pending.get(m.request_id)
    if (!p) return
    this.pending.delete(m.request_id)
    window.clearTimeout(p.timer)

    const env = m as unknown as ResponseEnvelope
    if (env.status === 'error') {
      const err = env.error
      p.reject(new Error(err?.message || err?.code || 'request failed'))
    } else {
      p.resolve(env.result)
    }
  }

  private async sendSubscribe(reg: Registration) {
    try {
      const result = (await this.request('subscribe', { filters: reg.filters })) as
        | { subscription_id?: string }
        | undefined
      if (result?.subscription_id) reg.serverId = result.subscription_id
    } catch (e) {
      console.warn('[ws] subscribe failed', reg.filters, e)
    }
  }

  // ---- public subscription API ----

  /**
   * Subscribe to events matching `filters`. Returns an unsubscribe function.
   *
   * Safe to call before the socket is up: the registration is recorded and sent
   * as soon as the connection authenticates, and re-sent after every reconnect.
   */
  subscribe(filters: SubscriptionFilters, handler: EventHandler): () => void {
    const reg: Registration = { filters, handler }
    this.registrations.add(reg)

    if (this.ws?.readyState === WebSocket.OPEN && this.status === 'live') {
      void this.sendSubscribe(reg)
    } else {
      this.closedByUs = false
      this.connect()
    }

    return () => {
      this.registrations.delete(reg)
      if (reg.serverId && this.ws?.readyState === WebSocket.OPEN) {
        // Best-effort: the socket may be closing, and the server drops the
        // subscription with the connection anyway.
        this.request('unsubscribe', { subscription_id: reg.serverId }).catch(() => {})
      }
      if (this.registrations.size === 0) this.closeIdle()
    }
  }

  private closeIdle() {
    if (this.reconnectTimer !== null) {
      window.clearTimeout(this.reconnectTimer)
      this.reconnectTimer = null
    }
    this.ws?.close()
    this.ws = null
    this.setStatus('offline')
  }
}

// ============================================================
// Shared instances
// ============================================================

const clients = new Map<string, RaisinWsClient>()

/** The shared client for a repository, created on first use. */
export function getWsClient(repo: string): RaisinWsClient {
  let c = clients.get(repo)
  if (!c) {
    c = new RaisinWsClient(repo)
    clients.set(repo, c)
  }
  return c
}
