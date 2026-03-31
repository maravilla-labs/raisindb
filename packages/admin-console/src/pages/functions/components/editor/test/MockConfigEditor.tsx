/**
 * Mock Configuration Editor
 *
 * Allows users to configure optional function mocking for test runs.
 * Functions can be set to:
 * - 'real': Execute the actual function
 * - 'passthrough': Return input as output (no execution)
 * - 'mock_output': Return a predefined mock value
 *
 * AI agents always run with real behavior and cannot be mocked.
 */

import { useCallback } from 'react'
import { ChevronDown, ChevronRight, Wand2, ArrowRight, Code2 } from 'lucide-react'
import { useState } from 'react'
import type { FlowDefinition, FlowNode } from '@raisindb/flow-designer'
import { isFlowStep, isFlowContainer } from '@raisindb/flow-designer'

export type MockBehavior = 'real' | 'passthrough' | 'mock_output'

export interface FunctionMock {
  behavior: MockBehavior
  mock_output?: unknown
  mock_delay_ms?: number
}

export interface MockConfig {
  [functionPath: string]: FunctionMock
}

export interface MockConfigEditorProps {
  /** The workflow definition to extract function steps from */
  workflow: FlowDefinition
  /** Current mock configuration */
  mockConfig: MockConfig
  /** Called when mock configuration changes */
  onChange: (config: MockConfig) => void
}

// Extract all function paths from workflow nodes
function extractFunctionPaths(nodes: FlowNode[]): Array<{ stepId: string; path: string; name: string }> {
  const paths: Array<{ stepId: string; path: string; name: string }> = []

  for (const node of nodes) {
    if (isFlowStep(node) && node.properties.function_ref) {
      const ref = node.properties.function_ref
      const path = typeof ref === 'string' ? ref : ref['raisin:path'] || ref['raisin:ref'] || ''
      if (path) {
        paths.push({
          stepId: node.id,
          path,
          name: node.properties.action || node.id,
        })
      }
    }
    if (isFlowContainer(node) && node.children) {
      paths.push(...extractFunctionPaths(node.children))
    }
  }

  return paths
}

const BEHAVIOR_OPTIONS: Array<{ value: MockBehavior; label: string; icon: React.ReactNode; description: string }> = [
  {
    value: 'real',
    label: 'Real',
    icon: <Wand2 className="w-4 h-4" />,
    description: 'Execute the actual function',
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

export function MockConfigEditor({ workflow, mockConfig, onChange }: MockConfigEditorProps) {
  const [expanded, setExpanded] = useState(false)

  // Extract function paths from workflow
  const functionPaths = extractFunctionPaths(workflow.nodes)

  // Count mocked functions
  const mockCount = Object.values(mockConfig).filter((m) => m.behavior !== 'real').length

  // Update mock behavior
  const updateBehavior = useCallback(
    (path: string, behavior: MockBehavior) => {
      onChange({
        ...mockConfig,
        [path]: {
          ...mockConfig[path],
          behavior,
          mock_output: behavior === 'mock_output' ? mockConfig[path]?.mock_output ?? {} : undefined,
        },
      })
    },
    [mockConfig, onChange]
  )

  // Update mock output
  const updateMockOutput = useCallback(
    (path: string, outputStr: string) => {
      try {
        const output = JSON.parse(outputStr)
        onChange({
          ...mockConfig,
          [path]: {
            ...mockConfig[path],
            mock_output: output,
          },
        })
      } catch {
        // Invalid JSON, ignore
      }
    },
    [mockConfig, onChange]
  )

  if (functionPaths.length === 0) {
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
          <span className="text-sm font-medium text-white">Function Mocking</span>
          {mockCount > 0 && (
            <span className="px-2 py-0.5 text-xs bg-amber-500/20 text-amber-400 rounded-full">
              {mockCount} mocked
            </span>
          )}
        </div>
        <span className="text-xs text-gray-500">{functionPaths.length} functions</span>
      </button>

      {/* Content */}
      {expanded && (
        <div className="p-4 space-y-3 bg-black/20">
          <p className="text-xs text-gray-500">
            Configure how functions behave during test runs. AI agents always run with real behavior.
          </p>

          {functionPaths.map(({ stepId, path, name }) => {
            const mock = mockConfig[path]
            const isMocked = mock && mock.behavior !== 'real'

            return (
              <div
                key={stepId}
                className={`border rounded-lg transition-colors ${
                  isMocked ? 'border-amber-500/30 bg-amber-500/5' : 'border-white/10 bg-white/5'
                }`}
              >
                {/* Function header */}
                <div className="flex items-center justify-between px-3 py-2">
                  <div className="flex-1 min-w-0">
                    <p className="text-sm font-medium text-white truncate">{name}</p>
                    <p className="text-xs text-gray-500 truncate">{path}</p>
                  </div>
                  <div className="flex items-center gap-2">
                    {/* Behavior selector */}
                    <select
                      value={mock?.behavior || 'real'}
                      onChange={(e) => {
                        const behavior = e.target.value as MockBehavior
                        if (behavior === 'real') {
                          const newConfig = { ...mockConfig }
                          delete newConfig[path]
                          onChange(newConfig)
                        } else {
                          updateBehavior(path, behavior)
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
                      onChange={(e) => updateMockOutput(path, e.target.value)}
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
