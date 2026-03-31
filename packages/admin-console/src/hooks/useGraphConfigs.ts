import { useState, useEffect, useCallback } from 'react'
import { nodesApi } from '../api/nodes'
import { getAuthHeaders } from '../api/client'

const MANAGEMENT_API_BASE = '/management'
const GRAPH_CONFIG_WORKSPACE = 'raisin:access_control'
const GRAPH_CONFIG_PATH_PREFIX = '/graph-config'
const GRAPH_CONFIG_NODE_TYPE = 'raisin:GraphAlgorithmConfig'

export interface GraphTarget {
  mode: 'branch' | 'all_branches' | 'revision' | 'branch_pattern'
  branches?: string[]
  revisions?: string[]
  branch_pattern?: string
}

export interface GraphScope {
  paths?: string[]
  node_types?: string[]
  workspaces?: string[]
  relation_types?: string[]
}

export interface RefreshConfig {
  ttl_seconds: number
  on_branch_change: boolean
  on_relation_change: boolean
  cron?: string
}

export interface GraphAlgorithmConfig {
  id: string
  algorithm: string
  enabled: boolean
  target: GraphTarget
  scope: GraphScope
  algorithm_config: Record<string, unknown>
  refresh: RefreshConfig
}

export interface ConfigStatus {
  id: string
  algorithm: string
  enabled: boolean
  status: 'ready' | 'computing' | 'stale' | 'pending' | 'error'
  last_computed_at?: number
  next_scheduled_at?: number
  node_count?: number
  error?: string
  config: GraphAlgorithmConfig
}

export interface ConfigStatusResponse {
  configs: ConfigStatus[]
  next_tick_at: number
  tick_interval_seconds: number
}

interface ApiResponse<T> {
  success: boolean
  data?: T
  error?: string
}

/**
 * Hook to fetch and manage graph algorithm configurations
 */
export function useGraphConfigs(repo: string | undefined) {
  const [configs, setConfigs] = useState<ConfigStatus[]>([])
  const [nextTickAt, setNextTickAt] = useState<number>(0)
  const [tickInterval, setTickInterval] = useState<number>(60)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const fetchConfigs = useCallback(async () => {
    if (!repo) {
      setConfigs([])
      setLoading(false)
      return
    }

    setLoading(true)
    setError(null)

    try {
      const response = await fetch(`${MANAGEMENT_API_BASE}/graph-cache/${repo}/status`, {
        headers: getAuthHeaders(),
      })
      const data: ApiResponse<ConfigStatusResponse> = await response.json()

      if (data.success && data.data) {
        setConfigs(data.data.configs)
        setNextTickAt(data.data.next_tick_at)
        setTickInterval(data.data.tick_interval_seconds)
      } else {
        setError(data.error || 'Failed to fetch graph configs')
        setConfigs([])
      }
    } catch (err) {
      setError((err as Error).message)
      setConfigs([])
    } finally {
      setLoading(false)
    }
  }, [repo])

  useEffect(() => {
    fetchConfigs()
  }, [fetchConfigs])

  /**
   * Trigger immediate recomputation for a config
   */
  const triggerRecompute = useCallback(async (configId: string): Promise<{ success: boolean; error?: string }> => {
    if (!repo) return { success: false, error: 'No repository selected' }

    try {
      const response = await fetch(`${MANAGEMENT_API_BASE}/graph-cache/${repo}/${configId}/recompute`, {
        method: 'POST',
        headers: getAuthHeaders(),
      })
      const data: ApiResponse<string> = await response.json()

      if (data.success) {
        // Refresh the configs after triggering recompute
        setTimeout(() => fetchConfigs(), 1000)
        return { success: true }
      } else {
        return { success: false, error: data.error }
      }
    } catch (err) {
      return { success: false, error: (err as Error).message }
    }
  }, [repo, fetchConfigs])

  /**
   * Mark config as stale to be picked up at next tick
   */
  const markStale = useCallback(async (configId: string): Promise<{ success: boolean; error?: string }> => {
    if (!repo) return { success: false, error: 'No repository selected' }

    try {
      const response = await fetch(`${MANAGEMENT_API_BASE}/graph-cache/${repo}/${configId}/mark-stale`, {
        method: 'POST',
        headers: getAuthHeaders(),
      })
      const data: ApiResponse<string> = await response.json()

      if (data.success) {
        // Refresh the configs after marking stale
        setTimeout(() => fetchConfigs(), 500)
        return { success: true }
      } else {
        return { success: false, error: data.error }
      }
    } catch (err) {
      return { success: false, error: (err as Error).message }
    }
  }, [repo, fetchConfigs])

  /**
   * Create a new graph algorithm configuration using nodesApi
   */
  const createConfig = useCallback(async (config: GraphAlgorithmConfig): Promise<{ success: boolean; error?: string }> => {
    if (!repo) return { success: false, error: 'No repository selected' }

    try {
      // Build node properties from config
      const properties = configToNodeProperties(config)

      // Create the node using nodesApi
      await nodesApi.create(repo, 'main', GRAPH_CONFIG_WORKSPACE, GRAPH_CONFIG_PATH_PREFIX, {
        name: config.id,
        node_type: GRAPH_CONFIG_NODE_TYPE,
        properties,
        commit: {
          message: `Created graph algorithm config: ${config.id}`,
        },
      })

      // Refresh the configs after creating
      await fetchConfigs()
      return { success: true }
    } catch (err) {
      return { success: false, error: (err as Error).message }
    }
  }, [repo, fetchConfigs])

  /**
   * Update an existing graph algorithm configuration using nodesApi
   */
  const updateConfig = useCallback(async (configId: string, config: GraphAlgorithmConfig): Promise<{ success: boolean; error?: string }> => {
    if (!repo) return { success: false, error: 'No repository selected' }

    try {
      // Build node properties from config
      const properties = configToNodeProperties(config)

      // Update the node using nodesApi
      const nodePath = `${GRAPH_CONFIG_PATH_PREFIX}/${configId}`
      await nodesApi.update(repo, 'main', GRAPH_CONFIG_WORKSPACE, nodePath, {
        properties,
        commit: {
          message: `Updated graph algorithm config: ${configId}`,
        },
      })

      // Refresh the configs after updating
      await fetchConfigs()
      return { success: true }
    } catch (err) {
      return { success: false, error: (err as Error).message }
    }
  }, [repo, fetchConfigs])

  /**
   * Delete a graph algorithm configuration using nodesApi
   */
  const deleteConfig = useCallback(async (configId: string): Promise<{ success: boolean; error?: string }> => {
    if (!repo) return { success: false, error: 'No repository selected' }

    try {
      // Delete the node using nodesApi
      const nodePath = `${GRAPH_CONFIG_PATH_PREFIX}/${configId}`
      await nodesApi.delete(repo, 'main', GRAPH_CONFIG_WORKSPACE, nodePath, {
        commit: {
          message: `Deleted graph algorithm config: ${configId}`,
        },
      })

      // Refresh the configs after deleting
      await fetchConfigs()
      return { success: true }
    } catch (err) {
      return { success: false, error: (err as Error).message }
    }
  }, [repo, fetchConfigs])

  return {
    configs,
    nextTickAt,
    tickInterval,
    loading,
    error,
    refresh: fetchConfigs,
    triggerRecompute,
    markStale,
    createConfig,
    updateConfig,
    deleteConfig,
  }
}

/**
 * Convert a GraphAlgorithmConfig to node properties format
 */
function configToNodeProperties(config: GraphAlgorithmConfig): Record<string, unknown> {
  return {
    algorithm: config.algorithm,
    enabled: config.enabled,
    target: {
      mode: config.target.mode,
      ...(config.target.branches?.length && { branches: config.target.branches }),
      ...(config.target.revisions?.length && { revisions: config.target.revisions }),
      ...(config.target.branch_pattern && { branch_pattern: config.target.branch_pattern }),
    },
    scope: {
      ...(config.scope.paths?.length && { paths: config.scope.paths }),
      ...(config.scope.node_types?.length && { node_types: config.scope.node_types }),
      ...(config.scope.workspaces?.length && { workspaces: config.scope.workspaces }),
      ...(config.scope.relation_types?.length && { relation_types: config.scope.relation_types }),
    },
    ...(Object.keys(config.algorithm_config).length > 0 && { config: config.algorithm_config }),
    refresh: {
      ttl_seconds: config.refresh.ttl_seconds,
      on_branch_change: config.refresh.on_branch_change,
      on_relation_change: config.refresh.on_relation_change,
      ...(config.refresh.cron && { cron: config.refresh.cron }),
    },
  }
}
