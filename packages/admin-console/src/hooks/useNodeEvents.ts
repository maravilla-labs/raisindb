// SPDX-License-Identifier: BSL-1.1

/**
 * React bindings for the node-event WebSocket ({@link getWsClient}).
 *
 * Kept deliberately thin: the socket, reconnection and re-subscription all live
 * in the client, so a component only declares *what* it wants to watch.
 */

import { useEffect, useRef, useState } from 'react'
import {
  getWsClient,
  type NodeEvent,
  type SubscriptionFilters,
  type WsStatus,
} from '../api/ws-events'

/**
 * Subscribe to node events for `repo` while the component is mounted.
 *
 * `filters` is read through a ref, so an inline object literal does not tear
 * down and rebuild the subscription on every render. Pass `enabled: false` (or
 * an undefined repo) to stay disconnected.
 *
 * The returned status is the *socket's*, not this subscription's: 'live' means
 * frames can arrive, not that this filter has matched anything yet. Callers
 * should keep their polling fallback running whenever it is not 'live'.
 */
export function useNodeEvents(
  repo: string | undefined,
  filters: SubscriptionFilters,
  onEvent: (event: NodeEvent) => void,
  enabled = true,
): WsStatus {
  const [status, setStatus] = useState<WsStatus>('offline')

  // Latest-callback refs: the effect below must not re-run (and so must not
  // re-subscribe) merely because the parent re-rendered with a new closure.
  const onEventRef = useRef(onEvent)
  onEventRef.current = onEvent
  const filtersRef = useRef(filters)
  filtersRef.current = filters

  // Serialized filters ARE the dependency — a change of what we watch must
  // resubscribe, a change of object identity must not.
  const filterKey = JSON.stringify(filters)

  useEffect(() => {
    if (!repo || !enabled) {
      setStatus('offline')
      return
    }
    const client = getWsClient(repo)
    const offStatus = client.onStatus(setStatus)
    const off = client.subscribe(filtersRef.current, (e) => onEventRef.current(e))
    return () => {
      off()
      offStatus()
    }
  }, [repo, filterKey, enabled])

  return status
}
