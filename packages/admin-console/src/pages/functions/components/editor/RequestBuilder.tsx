/**
 * Request Builder Component
 *
 * Builds and executes function invocation requests.
 * Allows configuring input parameters, sync/async mode, and timeout.
 */

import { useState, useCallback, useEffect } from 'react'
import { Play, ChevronDown, ChevronRight, Loader2, Database, FileJson, X } from 'lucide-react'
import { QuickPick } from './QuickPick'

export interface FunctionRunConfig {
  inputType: 'json' | 'node'
  inputJson: string
  inputNodeId: string | null
  inputNodePath: string | null
  inputWorkspace: string
  sync: boolean
  timeout_ms: number
}

export interface PreparedRun {
  inputType: 'json' | 'node'
  input: Record<string, unknown> | null
  inputNodeId: string | null
  inputNodePath: string | null
  inputWorkspace: string
  sync: boolean
  timeout_ms: number
}

export interface RequestBuilderProps {
  functionName: string
  config: FunctionRunConfig
  onConfigChange: (config: FunctionRunConfig) => void
  onRun: (run: PreparedRun) => void
  isRunning: boolean
  disabled?: boolean
}

export function RequestBuilder({
  functionName,
  config,
  onConfigChange,
  onRun,
  isRunning,
  disabled = false,
}: RequestBuilderProps) {
  const [isExpanded, setIsExpanded] = useState(true)
  const [jsonError, setJsonError] = useState<string | null>(null)
  const [showNodePicker, setShowNodePicker] = useState(false)

  useEffect(() => {
    if (config.inputType === 'json' && config.inputJson.trim()) {
      try {
        const parsed = JSON.parse(config.inputJson)
        if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) {
          setJsonError('Input must be a JSON object')
        } else {
          setJsonError(null)
        }
      } catch {
        setJsonError('Invalid JSON')
      }
    } else {
      setJsonError(null)
    }
  }, [config.inputJson, config.inputType])

  const validateAndParseJson = useCallback((value: string): Record<string, unknown> | null => {
    try {
      const parsed = JSON.parse(value)
      if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) {
        setJsonError('Input must be a JSON object')
        return null
      }
      setJsonError(null)
      return parsed
    } catch {
      setJsonError('Invalid JSON')
      return null
    }
  }, [])

  const handleInputChange = (value: string) => {
    onConfigChange({ ...config, inputJson: value })
    if (value.trim()) {
      validateAndParseJson(value)
    } else {
      setJsonError(null)
    }
  }

  const handleInputTypeChange = (type: 'json' | 'node') => {
    onConfigChange({ ...config, inputType: type })
  }

  const handleTimeoutChange = (value: string) => {
    const parsed = parseInt(value, 10)
    const safeValue = Number.isFinite(parsed) ? parsed : config.timeout_ms
    onConfigChange({ ...config, timeout_ms: safeValue })
  }

  const handleNodeSelect = (nodeId: string, nodePath: string, workspace: string) => {
    onConfigChange({
      ...config,
      inputType: 'node',
      inputNodeId: nodeId,
      inputNodePath: nodePath,
      inputWorkspace: workspace,
    })
    setShowNodePicker(false)
  }

  const handleClearNode = () => {
    onConfigChange({
      ...config,
      inputNodeId: null,
      inputNodePath: null,
    })
  }

  const handleRun = () => {
    if (config.inputType === 'json') {
      const input = config.inputJson.trim() ? validateAndParseJson(config.inputJson) : {}
      if (input === null) return

      onRun({
        inputType: 'json',
        input,
        inputNodeId: null,
        inputNodePath: null,
        inputWorkspace: config.inputWorkspace,
        sync: config.sync,
        timeout_ms: config.timeout_ms,
      })
      return
    }

    // Node input
    if (!config.inputNodeId) {
      setJsonError('Select a node to use as input')
      return
    }

    onRun({
      inputType: 'node',
      input: null,
      inputNodeId: config.inputNodeId,
      inputNodePath: config.inputNodePath,
      inputWorkspace: config.inputWorkspace,
      sync: config.sync,
      timeout_ms: config.timeout_ms,
    })
  }

  const canRun = !disabled && !isRunning && !jsonError && (config.inputType === 'json' || !!config.inputNodeId)

  return (
    <div className="border-t border-white/10">
      {/* Header */}
      <button
        onClick={() => setIsExpanded(!isExpanded)}
        className="w-full flex items-center gap-2 px-4 py-2 hover:bg-white/5 text-white"
      >
        {isExpanded ? (
          <ChevronDown className="w-4 h-4" />
        ) : (
          <ChevronRight className="w-4 h-4" />
        )}
        <Play className="w-4 h-4 text-green-400" />
        <span className="text-sm font-medium">Run Function</span>
      </button>

      {/* Content */}
      {isExpanded && (
        <div className="px-4 pb-4 space-y-4">
          {/* Input Type */}
          <div className="flex items-center gap-3">
            <label className="text-xs text-gray-400">Input Type:</label>
            <div className="flex rounded overflow-hidden border border-white/10">
              <button
                onClick={() => handleInputTypeChange('json')}
                className={`px-2 py-1 text-xs flex items-center gap-1 transition-colors ${
                  config.inputType === 'json'
                    ? 'bg-primary-500/30 text-primary-300'
                    : 'bg-white/5 text-gray-400 hover:bg-white/10'
                }`}
                disabled={disabled || isRunning}
              >
                <FileJson className="w-3 h-3" />
                JSON
              </button>
              <button
                onClick={() => handleInputTypeChange('node')}
                className={`px-2 py-1 text-xs flex items-center gap-1 transition-colors border-l border-white/10 ${
                  config.inputType === 'node'
                    ? 'bg-primary-500/30 text-primary-300'
                    : 'bg-white/5 text-gray-400 hover:bg-white/10'
                }`}
                disabled={disabled || isRunning}
              >
                <Database className="w-3 h-3" />
                Node
              </button>
            </div>
          </div>

          {/* Input JSON */}
          {config.inputType === 'json' ? (
            <div>
              <label className="block text-xs text-gray-400 mb-1">Input (JSON)</label>
              <textarea
                value={config.inputJson}
                onChange={(e) => handleInputChange(e.target.value)}
                placeholder='{"key": "value"}'
                rows={4}
                disabled={disabled || isRunning}
                className={`w-full px-2 py-1.5 bg-white/5 border rounded text-sm text-white font-mono
                  placeholder-gray-500 resize-none
                  focus:outline-none focus:ring-1 focus:ring-primary-500
                  disabled:opacity-50 disabled:cursor-not-allowed
                  ${jsonError ? 'border-red-500' : 'border-white/10'}
                `}
              />
              {jsonError && (
                <p className="text-xs text-red-400 mt-1">{jsonError}</p>
              )}
            </div>
          ) : (
            <div className="flex items-center gap-2">
              <label className="text-xs text-gray-400">Input Node:</label>
              <button
                onClick={() => setShowNodePicker(true)}
                disabled={disabled || isRunning}
                className="flex-1 px-2 py-1 text-sm bg-white/5 border border-white/10 rounded text-left text-gray-300 hover:bg-white/10 flex items-center justify-between transition-colors disabled:opacity-50"
              >
                <span className="truncate">
                  {config.inputNodePath || 'Select a node...'}
                </span>
                <ChevronDown className="w-4 h-4 text-gray-500 flex-shrink-0" />
              </button>
              {config.inputNodeId && (
                <button
                  onClick={handleClearNode}
                  className="p-1 text-gray-400 hover:text-white transition-colors"
                  title="Clear selection"
                  disabled={disabled || isRunning}
                >
                  <X className="w-4 h-4" />
                </button>
              )}
            </div>
          )}

          {/* Options Row */}
          <div className="flex items-center gap-4">
            {/* Sync/Async Toggle */}
            <div className="flex items-center gap-2">
              <label className="text-xs text-gray-400">Mode:</label>
              <div className="flex rounded overflow-hidden border border-white/10">
                <button
                  onClick={() => onConfigChange({ ...config, sync: true })}
                  disabled={disabled || isRunning}
                  className={`px-2 py-1 text-xs transition-colors
                    ${config.sync
                      ? 'bg-primary-500/30 text-primary-300'
                      : 'bg-white/5 text-gray-400 hover:bg-white/10'
                    }
                    disabled:opacity-50 disabled:cursor-not-allowed
                  `}
                >
                  Sync
                </button>
                <button
                  onClick={() => onConfigChange({ ...config, sync: false })}
                  disabled={disabled || isRunning}
                  className={`px-2 py-1 text-xs transition-colors border-l border-white/10
                    ${!config.sync
                      ? 'bg-primary-500/30 text-primary-300'
                      : 'bg-white/5 text-gray-400 hover:bg-white/10'
                    }
                    disabled:opacity-50 disabled:cursor-not-allowed
                  `}
                >
                  Async
                </button>
              </div>
            </div>

            {/* Timeout */}
            <div className="flex items-center gap-2">
              <label className="text-xs text-gray-400">Timeout:</label>
              <input
                type="number"
                value={config.timeout_ms}
                onChange={(e) => handleTimeoutChange(e.target.value)}
                disabled={disabled || isRunning}
                className="w-20 px-2 py-1 bg-white/5 border border-white/10 rounded text-xs text-white
                  focus:outline-none focus:ring-1 focus:ring-primary-500
                  disabled:opacity-50 disabled:cursor-not-allowed"
              />
              <span className="text-xs text-gray-500">ms</span>
            </div>
          </div>

          {/* Run Button */}
          <button
            onClick={handleRun}
            disabled={!canRun}
            className={`w-full flex items-center justify-center gap-2 px-4 py-2 rounded text-sm font-medium
              transition-colors
              ${canRun
                ? 'bg-green-500/20 text-green-300 hover:bg-green-500/30'
                : 'bg-gray-500/20 text-gray-500 cursor-not-allowed'
              }
            `}
          >
            {isRunning ? (
              <>
                <Loader2 className="w-4 h-4 animate-spin" />
                Running {functionName}...
              </>
            ) : (
              <>
                <Play className="w-4 h-4" />
                Run {functionName}
              </>
            )}
          </button>

          {showNodePicker && (
            <QuickPick
              onSelect={handleNodeSelect}
              onClose={() => setShowNodePicker(false)}
              initialWorkspace={config.inputWorkspace || 'content'}
            />
          )}
        </div>
      )}
    </div>
  )
}
