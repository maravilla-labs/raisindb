import { useState, useEffect, useCallback } from 'react'
import { fetchEventSource } from '@microsoft/fetch-event-source'
import { getAuthHeaders } from '../api/client'

const MANAGEMENT_API_BASE = '/management'

export interface TickCountdownEvent {
  type: 'tick_countdown'
  next_tick_at: number
  seconds_remaining: number
}

export interface ComputationStartedEvent {
  type: 'computation_started'
  config_id: string
  algorithm: string
  node_count?: number
}

export interface ComputationProgressEvent {
  type: 'computation_progress'
  config_id: string
  progress_pct: number
  current_step: string
}

export interface ComputationCompletedEvent {
  type: 'computation_completed'
  config_id: string
  duration_ms: number
  node_count: number
}

export interface ComputationFailedEvent {
  type: 'computation_failed'
  config_id: string
  error: string
}

export interface StatusChangedEvent {
  type: 'status_changed'
  config_id: string
  old_status: string
  new_status: string
}

export type GraphCacheEvent =
  | TickCountdownEvent
  | ComputationStartedEvent
  | ComputationProgressEvent
  | ComputationCompletedEvent
  | ComputationFailedEvent
  | StatusChangedEvent

export interface ComputingConfig {
  configId: string
  progress: number
  currentStep: string
}

export type ConnectionStatus = 'connected' | 'connecting' | 'disconnected'

/**
 * Hook for subscribing to real-time graph cache updates via SSE
 */
export function useGraphCacheSSE(repo: string | undefined) {
  const [connectionStatus, setConnectionStatus] = useState<ConnectionStatus>('disconnected')
  const [secondsUntilTick, setSecondsUntilTick] = useState<number>(60)
  const [computingConfigs, setComputingConfigs] = useState<Map<string, ComputingConfig>>(new Map())
  const [lastEvent, setLastEvent] = useState<GraphCacheEvent | null>(null)

  useEffect(() => {
    if (!repo) {
      setConnectionStatus('disconnected')
      return
    }

    setConnectionStatus('connecting')

    const controller = new AbortController()
    const url = `${MANAGEMENT_API_BASE}/graph-cache/${repo}/stream`

    // Get auth headers (without impersonation for SSE)
    const authHeaders = getAuthHeaders()
    delete authHeaders['X-Raisin-Impersonate']

    fetchEventSource(url, {
      headers: authHeaders,
      signal: controller.signal,
      openWhenHidden: true,

      onopen: async () => {
        setConnectionStatus('connected')
      },

      onerror: () => {
        setConnectionStatus('disconnected')
      },

      onmessage: (event) => {
        try {
          const data = JSON.parse(event.data)

          switch (event.event) {
            case 'tick_countdown': {
              const tickData = data as TickCountdownEvent
              setSecondsUntilTick(tickData.seconds_remaining)
              setLastEvent(tickData)
              break
            }
            case 'computation_started': {
              const startedData = data as ComputationStartedEvent
              setComputingConfigs(prev => {
                const next = new Map(prev)
                next.set(startedData.config_id, {
                  configId: startedData.config_id,
                  progress: 0,
                  currentStep: 'Starting...',
                })
                return next
              })
              setLastEvent(startedData)
              break
            }
            case 'computation_progress': {
              const progressData = data as ComputationProgressEvent
              setComputingConfigs(prev => {
                const next = new Map(prev)
                next.set(progressData.config_id, {
                  configId: progressData.config_id,
                  progress: progressData.progress_pct,
                  currentStep: progressData.current_step,
                })
                return next
              })
              setLastEvent(progressData)
              break
            }
            case 'computation_completed': {
              const completedData = data as ComputationCompletedEvent
              setComputingConfigs(prev => {
                const next = new Map(prev)
                next.delete(completedData.config_id)
                return next
              })
              setLastEvent(completedData)
              break
            }
            case 'computation_failed': {
              const failedData = data as ComputationFailedEvent
              setComputingConfigs(prev => {
                const next = new Map(prev)
                next.delete(failedData.config_id)
                return next
              })
              setLastEvent(failedData)
              break
            }
            case 'status_changed': {
              const statusData = data as StatusChangedEvent
              setLastEvent(statusData)
              break
            }
          }
        } catch (e) {
          console.error('Failed to parse graph cache event:', e)
        }
      },
    })

    return () => {
      controller.abort()
      setConnectionStatus('disconnected')
    }
  }, [repo])

  /**
   * Format seconds remaining as a human-readable string
   */
  const formatTimeRemaining = useCallback((seconds: number): string => {
    if (seconds < 60) {
      return `${seconds}s`
    }
    const minutes = Math.floor(seconds / 60)
    const remainingSeconds = seconds % 60
    if (remainingSeconds === 0) {
      return `${minutes}m`
    }
    return `${minutes}m ${remainingSeconds}s`
  }, [])

  return {
    connectionStatus,
    secondsUntilTick,
    formattedTimeRemaining: formatTimeRemaining(secondsUntilTick),
    computingConfigs,
    lastEvent,
    isComputing: (configId: string) => computingConfigs.has(configId),
    getComputingProgress: (configId: string) => computingConfigs.get(configId),
  }
}
