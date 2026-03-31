import { nodesApi, type Node, type CreateNodeRequest, type UpdateNodeRequest } from './nodes'

// Supported AI providers
export type AIProviderType = 'openai' | 'anthropic' | 'google' | 'azure_openai' | 'ollama' | 'groq' | 'openrouter' | 'bedrock' | 'local' | 'custom'

export type ExecutionMode = 'automatic' | 'step_by_step' | 'manual'


// Agent interface matching raisin:AIAgent node type
export interface Agent {
  id: string
  name: string
  path: string
  properties: {
    system_prompt: string
    provider: AIProviderType
    model: string
    temperature: number
    max_tokens: number
    thinking_enabled: boolean
    task_creation_enabled: boolean
    execution_mode: ExecutionMode
    tools?: string[]
    rules?: string[]
    compaction_enabled: boolean
    compaction_token_threshold: number
    compaction_keep_recent: number
    compaction_provider?: string
    compaction_model?: string
    compaction_prompt?: string
  }
  created_at?: string
  updated_at?: string
}

// Request to create an agent
export interface CreateAgentRequest {
  name: string
  system_prompt: string
  provider: AIProviderType
  model: string
  temperature?: number
  max_tokens?: number
  thinking_enabled?: boolean
  task_creation_enabled?: boolean
  execution_mode?: ExecutionMode
  tools?: string[]
  rules?: string[]
  compaction_enabled?: boolean
  compaction_token_threshold?: number
  compaction_keep_recent?: number
  compaction_provider?: string
  compaction_model?: string
  compaction_prompt?: string
}

// Request to update an agent
export interface UpdateAgentRequest {
  system_prompt?: string
  provider?: AIProviderType
  model?: string
  temperature?: number
  max_tokens?: number
  thinking_enabled?: boolean
  task_creation_enabled?: boolean
  execution_mode?: ExecutionMode
  tools?: string[]
  rules?: string[]
  compaction_enabled?: boolean
  compaction_token_threshold?: number
  compaction_keep_recent?: number
  compaction_provider?: string
  compaction_model?: string
  compaction_prompt?: string
}

/** Parse tools from DB — extracts path strings from any format */
function parseToolPaths(raw: unknown): string[] {
  if (!Array.isArray(raw)) return []
  return raw
    .map((t): string | null => {
      if (typeof t === 'string') return t
      if (t && typeof t === 'object' && 'raisin:path' in t) return (t as Record<string, string>)['raisin:path']
      return null
    })
    .filter((t): t is string => !!t)
}

const WORKSPACE = 'functions'
const AGENTS_PATH = '/agents'

/**
 * Convert a Node to an Agent
 */
function nodeToAgent(node: Node): Agent {
  return {
    id: node.id,
    name: node.name,
    path: node.path,
    properties: {
      system_prompt: (node.properties?.system_prompt as string) || '',
      provider: (node.properties?.provider as AIProviderType) || 'openai',
      model: (node.properties?.model as string) || '',
      temperature: (node.properties?.temperature as number) ?? 0.7,
      max_tokens: (node.properties?.max_tokens as number) ?? 4096,
      thinking_enabled: (node.properties?.thinking_enabled as boolean) ?? false,
      task_creation_enabled: (node.properties?.task_creation_enabled as boolean) ?? false,
      execution_mode: (node.properties?.execution_mode as ExecutionMode) || 'automatic',
      tools: parseToolPaths(node.properties?.tools),
      rules: (node.properties?.rules as string[]) || [],
      compaction_enabled: (node.properties?.compaction_enabled as boolean) ?? true,
      compaction_token_threshold: (node.properties?.compaction_token_threshold as number) ?? 8000,
      compaction_keep_recent: (node.properties?.compaction_keep_recent as number) ?? 10,
      compaction_provider: node.properties?.compaction_provider as string | undefined,
      compaction_model: node.properties?.compaction_model as string | undefined,
      compaction_prompt: node.properties?.compaction_prompt as string | undefined,
    },
    created_at: node.created_at,
    updated_at: node.updated_at,
  }
}

export const agentsApi = {
  /**
   * List all agents in the functions workspace
   */
  list: async (repo: string, branch = 'main'): Promise<Agent[]> => {
    try {
      const nodes = await nodesApi.listChildrenAtHead(repo, branch, WORKSPACE, AGENTS_PATH)
      return nodes
        .filter(node => node.node_type === 'raisin:AIAgent')
        .map(nodeToAgent)
    } catch (error) {
      console.error('Failed to list agents:', error)
      return []
    }
  },

  /**
   * Get a single agent by name
   */
  get: async (repo: string, agentName: string, branch = 'main'): Promise<Agent> => {
    const path = `${AGENTS_PATH}/${agentName}`
    const node = await nodesApi.getAtHead(repo, branch, WORKSPACE, path)
    return nodeToAgent(node)
  },

  /**
   * Create a new agent
   */
  create: async (
    repo: string,
    agent: CreateAgentRequest,
    branch = 'main'
  ): Promise<Agent> => {
    const request: CreateNodeRequest = {
      name: agent.name,
      node_type: 'raisin:AIAgent',
      properties: {
        system_prompt: agent.system_prompt,
        provider: agent.provider,
        model: agent.model,
        temperature: agent.temperature ?? 0.7,
        max_tokens: agent.max_tokens ?? 4096,
        thinking_enabled: agent.thinking_enabled ?? false,
        task_creation_enabled: agent.task_creation_enabled ?? false,
        execution_mode: agent.execution_mode ?? 'automatic',
        tools: agent.tools || [],
        rules: agent.rules || [],
        compaction_enabled: agent.compaction_enabled ?? true,
        compaction_token_threshold: agent.compaction_token_threshold ?? 8000,
        compaction_keep_recent: agent.compaction_keep_recent ?? 10,
        ...(agent.compaction_provider && { compaction_provider: agent.compaction_provider }),
        ...(agent.compaction_model && { compaction_model: agent.compaction_model }),
        ...(agent.compaction_prompt && { compaction_prompt: agent.compaction_prompt }),
      },
      commit: {
        message: `Create agent: ${agent.name}`,
        actor: 'admin',
      },
    }

    const node = await nodesApi.create(repo, branch, WORKSPACE, AGENTS_PATH, request)
    return nodeToAgent(node)
  },

  /**
   * Update an existing agent
   */
  update: async (
    repo: string,
    agentName: string,
    agent: UpdateAgentRequest,
    branch = 'main'
  ): Promise<Agent> => {
    const path = `${AGENTS_PATH}/${agentName}`

    const request: UpdateNodeRequest = {
      properties: {
        ...(agent.system_prompt !== undefined && { system_prompt: agent.system_prompt }),
        ...(agent.provider !== undefined && { provider: agent.provider }),
        ...(agent.model !== undefined && { model: agent.model }),
        ...(agent.temperature !== undefined && { temperature: agent.temperature }),
        ...(agent.max_tokens !== undefined && { max_tokens: agent.max_tokens }),
        ...(agent.thinking_enabled !== undefined && { thinking_enabled: agent.thinking_enabled }),
        ...(agent.task_creation_enabled !== undefined && { task_creation_enabled: agent.task_creation_enabled }),
        ...(agent.execution_mode !== undefined && { execution_mode: agent.execution_mode }),
        ...(agent.tools !== undefined && { tools: agent.tools }),
        ...(agent.rules !== undefined && { rules: agent.rules }),
        ...(agent.compaction_enabled !== undefined && { compaction_enabled: agent.compaction_enabled }),
        ...(agent.compaction_token_threshold !== undefined && { compaction_token_threshold: agent.compaction_token_threshold }),
        ...(agent.compaction_keep_recent !== undefined && { compaction_keep_recent: agent.compaction_keep_recent }),
        ...(agent.compaction_provider !== undefined && { compaction_provider: agent.compaction_provider }),
        ...(agent.compaction_model !== undefined && { compaction_model: agent.compaction_model }),
        ...(agent.compaction_prompt !== undefined && { compaction_prompt: agent.compaction_prompt }),
      },
      commit: {
        message: `Update agent: ${agentName}`,
        actor: 'admin',
      },
    }

    const node = await nodesApi.update(repo, branch, WORKSPACE, path, request)
    return nodeToAgent(node)
  },

  /**
   * Delete an agent
   */
  delete: async (repo: string, agentName: string, branch = 'main'): Promise<void> => {
    const path = `${AGENTS_PATH}/${agentName}`
    await nodesApi.delete(repo, branch, WORKSPACE, path, {
      commit: {
        message: `Delete agent: ${agentName}`,
        actor: 'admin',
      },
    })
  },
}
