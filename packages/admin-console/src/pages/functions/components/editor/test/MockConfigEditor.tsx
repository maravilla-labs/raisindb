/**
 * Mock Configuration Editor
 *
 * Allows users to configure optional function and AI agent mocking for
 * test runs. Each can be set to:
 * - 'real': Execute the actual function/agent
 * - 'passthrough': Return input as output (no execution)
 * - 'mock_output': Return a predefined mock value
 *
 * Functions are keyed by function path (mock_functions); AI steps (agent
 * steps, AI containers, chat) are keyed by agent path (mock_agents), so
 * AI flows can be tested without a provider.
 */

import { useCallback } from 'react'
import { ChevronDown, ChevronRight, Wand2, ArrowRight, Code2, Bot } from 'lucide-react'
import { useState } from 'react'
import type { FlowDefinition, FlowNode, RaisinReference } from '@raisindb/flow-designer'
import { isFlowStep, isFlowContainer } from '@raisindb/flow-designer'

export type MockBehavior = 'real' | 'passthrough' | 'mock_output'

export interface FunctionMock {
  behavior: MockBehavior
  mock_output?: unknown
  mock_delay_ms?: number
}

export interface MockConfig {
  [path: string]: FunctionMock
}

export interface MockConfigEditorProps {
  /** The workflow definition to extract function/agent steps from */
  workflow: FlowDefinition
  /** Current function mock configuration (keyed by function path) */
  mockConfig: MockConfig
  /** Called when function mock configuration changes */
  onChange: (config: MockConfig) => void
  /** Current agent mock configuration (keyed by agent path) */
  agentMockConfig?: MockConfig
  /** Called when agent mock configuration changes */
  onAgentChange?: (config: MockConfig) => void
}

interface MockTarget {
  stepId: string
  path: string
  name: string
  kind: 'function' | 'agent'
}

function refToPath(ref: RaisinReference | string | undefined): string {
  if (!ref) return ''
  if (typeof ref === 'string') return ref
  return ref['raisin:path'] || ref['raisin:ref'] || ''
}

// Extract all mockable function/agent paths from workflow nodes
function extractMockTargets(nodes: FlowNode[]): MockTarget[] {
  const targets: MockTarget[] = []

  for (const node of nodes) {
    if (isFlowStep(node)) {
      const name = node.properties.action || node.id
      const functionPath = refToPath(node.properties.function_ref)
      if (functionPath) {
        targets.push({ stepId: node.id, path: functionPath, name, kind: 'function' })
      }
      const agentPath =
        refToPath(node.properties.agent_ref) || refToPath(node.properties.chat_config?.agent_ref)
      if (agentPath) {
        targets.push({ stepId: node.id, path: agentPath, name, kind: 'agent' })
      }
    }
    if (isFlowContainer(node)) {
      const aiAgentPath = refToPath(node.ai_config?.agent_ref)
      if (aiAgentPath) {
        targets.push({ stepId: node.id, path: aiAgentPath, name: node.id, kind: 'agent' })
      }
      if (node.children) {
        targets.push(...extractMockTargets(node.children))
      }
    }
  }

  return targets
}

const BEHAVIOR_OPTIONS: Array<{ value: MockBehavior; label: string; icon: React.ReactNode; description: string }> = [
  {
    value: 'real',
    label: 'Real',
    icon: <Wand2 className="w-4 h-4" />,
    description: 'Execute the actual function/agent',
  },
  {
    value: 'passthrough',
    label: 'Passthrough',
    icon: <ArrowRight className="w-4 h-4" />,
    description: 'Return input as output',
  },
  {
    value: 'mock_output',
    label: 'Mock Output',
    icon: <Code2 className="w-4 h-4" />,
    description: 'Return custom mock value',
  },
]

export function MockConfigEditor({
  workflow,
  mockConfig,
  onChange,
  agentMockConfig = {},
  onAgentChange,
}: MockConfigEditorProps) {
  const [expanded, setExpanded] = useState(false)

  // Extract function/agent paths from workflow
  const targets = extractMockTargets(workflow.nodes)
  const functionTargets = targets.filter((t) => t.kind === 'function')
  const agentTargets = targets.filter((t) => t.kind === 'agent' && onAgentChange)

  // Count mocked entries
  const mockCount =
    Object.values(mockConfig).filter((m) => m.behavior !== 'real').length +
    Object.values(agentMockConfig).filter((m) => m.behavior !== 'real').length

  const configFor = useCallback(
    (kind: MockTarget['kind']) => (kind === 'function' ? mockConfig : agentMockConfig),
    [mockConfig, agentMockConfig]
  )

  const emitFor = useCallback(
    (kind: MockTarget['kind']) => (kind === 'function' ? onChange : onAgentChange!),
    [onChange, onAgentChange]
  )

  // Update mock behavior
  const updateBehavior = useCallback(
    (kind: MockTarget['kind'], path: string, behavior: MockBehavior) => {
      const config = configFor(kind)
      emitFor(kind)({
        ...config,
        [path]: {
          ...config[path],
          behavior,
          mock_output: behavior === 'mock_output' ? config[path]?.mock_output ?? {} : undefined,
        },
      })
    },
    [configFor, emitFor]
  )

  // Update mock output
  const updateMockOutput = useCallback(
    (kind: MockTarget['kind'], path: string, outputStr: string) => {
      try {
        const output = JSON.parse(outputStr)
        const config = configFor(kind)
        emitFor(kind)({
          ...config,
          [path]: {
            ...config[path],
            mock_output: output,
          },
        })
      } catch {
        // Invalid JSON, ignore
      }
    },
    [configFor, emitFor]
  )

  const allTargets = [...functionTargets, ...agentTargets]
  if (allTargets.length === 0) {
    return null
  }

  return (
    <div className="border border-white/10 rounded-lg overflow-hidden">
      {/* Header */}
      <button
        onClick={() => setExpanded(!expanded)}
        className="w-full flex items-center justify-between px-4 py-3 bg-white/5 hover:bg-white/10 transition-colors"
      >
        <div className="flex items-center gap-2">
          {expanded ? (
            <ChevronDown className="w-4 h-4 text-gray-400" />
          ) : (
            <ChevronRight className="w-4 h-4 text-gray-400" />
          )}
          <span className="text-sm font-medium text-white">Function & AI Mocking</span>
          {mockCount > 0 && (
            <span className="px-2 py-0.5 text-xs bg-amber-500/20 text-amber-400 rounded-full">
              {mockCount} mocked
            </span>
          )}
        </div>
        <span className="text-xs text-gray-500">
          {functionTargets.length} functions
          {agentTargets.length > 0 ? `, ${agentTargets.length} agents` : ''}
        </span>
      </button>

      {/* Content */}
      {expanded && (
        <div className="p-4 space-y-3 bg-black/20">
          <p className="text-xs text-gray-500">
            Configure how functions and AI agents behave during test runs. Mocked AI steps return
            their mock value without calling a provider.
          </p>

          {allTargets.map(({ stepId, path, name, kind }) => {
            const mock = configFor(kind)[path]
            const isMocked = mock && mock.behavior !== 'real'

            return (
              <div
                key={`${kind}:${stepId}:${path}`}
                className={`border rounded-lg transition-colors ${
                  isMocked ? 'border-amber-500/30 bg-amber-500/5' : 'border-white/10 bg-white/5'
                }`}
              >
                {/* Target header */}
                <div className="flex items-center justify-between px-3 py-2">
                  <div className="flex-1 min-w-0">
                    <p className="text-sm font-medium text-white truncate flex items-center gap-1.5">
                      {kind === 'agent' && <Bot className="w-3.5 h-3.5 text-purple-400" />}
                      {name}
                    </p>
                    <p className="text-xs text-gray-500 truncate">{path}</p>
                  </div>
                  <div className="flex items-center gap-2">
                    {/* Behavior selector */}
                    <select
                      value={mock?.behavior || 'real'}
                      onChange={(e) => {
                        const behavior = e.target.value as MockBehavior
                        if (behavior === 'real') {
                          const newConfig = { ...configFor(kind) }
                          delete newConfig[path]
                          emitFor(kind)(newConfig)
                        } else {
                          updateBehavior(kind, path, behavior)
                        }
                      }}
                      className="px-2 py-1 text-xs bg-black/30 border border-white/10 rounded text-white focus:outline-none focus:ring-1 focus:ring-blue-500"
                    >
                      {BEHAVIOR_OPTIONS.map((opt) => (
                        <option key={opt.value} value={opt.value}>
                          {opt.label}
                        </option>
                      ))}
                    </select>
                  </div>
                </div>

                {/* Mock output editor */}
                {mock?.behavior === 'mock_output' && (
                  <div className="px-3 pb-3">
                    <label className="block text-xs text-gray-400 mb-1">Mock Output (JSON)</label>
                    <textarea
                      value={JSON.stringify(mock.mock_output ?? {}, null, 2)}
                      onChange={(e) => updateMockOutput(kind, path, e.target.value)}
                      rows={3}
                      className="w-full px-2 py-1.5 text-xs font-mono bg-black/40 border border-white/10 rounded text-white placeholder-gray-600 focus:outline-none focus:ring-1 focus:ring-blue-500 resize-none"
                      placeholder='{"result": "mocked"}'
                    />
                  </div>
                )}
              </div>
            )
          })}
        </div>
      )}
    </div>
  )
}

export default MockConfigEditor
