/**
 * Run Configuration Bar
 *
 * VS Code-style inline toolbar for configuring and executing functions/files.
 * Includes handler selection, input type toggle (JSON/Node), and run controls.
 */

import { useCallback, useEffect } from 'react'
import { Play, ChevronDown, Database, FileJson, X, Loader2 } from 'lucide-react'

export interface RunConfig {
  handler: string
  inputType: 'json' | 'node'
  inputJson: string
  inputNodeId: string | null
  inputNodePath: string | null
  inputWorkspace: string
}

interface RunConfigBarProps {
  /** Whether to show handler input (only for standalone files) */
  showHandler?: boolean
  /** Available handlers parsed from the open file */
  handlerOptions?: string[]
  /** Current configuration */
  config: RunConfig
  /** Called when config changes */
  onConfigChange: (config: RunConfig) => void
  /** Called when handler changes (overrides onConfigChange for handler field) */
  onHandlerChange?: (handler: string) => void
  /** Called when Run is clicked */
  onRun: () => void
  /** Whether execution is in progress */
  isRunning: boolean
  /** Called to open node picker */
  onOpenNodePicker?: () => void
  /** Whether run is disabled */
  disabled?: boolean
}

// Load saved config from localStorage
function loadSavedConfig(): Partial<RunConfig> {
  try {
    const saved = localStorage.getItem('raisindb.functions.runConfig')
    return saved ? JSON.parse(saved) : {}
  } catch {
    return {}
  }
}

// Save config to localStorage
function saveConfig(config: RunConfig) {
  try {
    localStorage.setItem('raisindb.functions.runConfig', JSON.stringify({
      handler: config.handler,
      inputType: config.inputType,
      inputWorkspace: config.inputWorkspace,
    }))
  } catch {
    // Ignore storage errors
  }
}

export function RunConfigBar({
  showHandler = false,
  config,
  onConfigChange,
  onHandlerChange,
  onRun,
  isRunning,
  onOpenNodePicker,
  disabled = false,
  handlerOptions = [],
}: RunConfigBarProps) {
  // Load saved preferences on mount
  useEffect(() => {
    const saved = loadSavedConfig()
    if (saved.handler || saved.inputType) {
      onConfigChange({
        ...config,
        handler: saved.handler || config.handler,
        inputType: saved.inputType || config.inputType,
        inputWorkspace: saved.inputWorkspace || config.inputWorkspace,
      })
    }
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  // Save config changes
  useEffect(() => {
    saveConfig(config)
  }, [config])

  const handleHandlerChange = useCallback((value: string) => {
    if (onHandlerChange) {
      onHandlerChange(value)
    } else {
      onConfigChange({ ...config, handler: value })
    }
  }, [config, onConfigChange, onHandlerChange])

  const handleHandlerSelect = useCallback((e: React.ChangeEvent<HTMLSelectElement>) => {
    handleHandlerChange(e.target.value)
  }, [handleHandlerChange])

  const handleHandlerInputChange = useCallback((e: React.ChangeEvent<HTMLInputElement>) => {
    handleHandlerChange(e.target.value)
  }, [handleHandlerChange])

  const handleInputTypeChange = useCallback((type: 'json' | 'node') => {
    onConfigChange({ ...config, inputType: type })
  }, [config, onConfigChange])

  const handleInputJsonChange = useCallback((e: React.ChangeEvent<HTMLInputElement>) => {
    onConfigChange({ ...config, inputJson: e.target.value })
  }, [config, onConfigChange])

  const handleClearNode = useCallback(() => {
    onConfigChange({ ...config, inputNodeId: null, inputNodePath: null })
  }, [config, onConfigChange])

  const hasHandlerOptions = handlerOptions.length > 0

  return (
    <div className="flex items-center gap-3 px-3 py-1.5 bg-black/10 border-b border-white/10">
      {/* Run Button */}
      <button
        onClick={onRun}
        disabled={disabled || isRunning}
        className={`
          flex items-center gap-1.5 px-3 py-1 rounded text-sm font-medium
          ${isRunning
            ? 'bg-yellow-500/20 text-yellow-300'
            : 'bg-green-500/20 text-green-300 hover:bg-green-500/30'
          }
          ${(disabled || isRunning) ? 'cursor-not-allowed opacity-60' : ''}
        `}
        title="Run (Ctrl+Enter)"
      >
        {isRunning ? (
          <Loader2 className="w-4 h-4 animate-spin" />
        ) : (
          <Play className="w-4 h-4" />
        )}
        {isRunning ? 'Running...' : 'Run'}
      </button>

      {/* Handler selector when functions are detected */}
      {hasHandlerOptions && (
        <div className="flex items-center gap-2">
          <label className="text-xs text-gray-400">Function:</label>
          <select
            value={config.handler}
            onChange={handleHandlerSelect}
            className="px-2 py-1 text-sm bg-black/30 border border-white/10 rounded text-white"
          >
            {handlerOptions.map((option) => (
              <option key={option} value={option}>
                {option}
              </option>
            ))}
            {!handlerOptions.includes(config.handler) && (
              <option value={config.handler || 'handler'}>
                {config.handler ? `Custom (${config.handler})` : 'Custom'}
              </option>
            )}
          </select>
        </div>
      )}

      {/* Handler Input (fallback) */}
      {!hasHandlerOptions && showHandler && (
        <div className="flex items-center gap-2">
          <label className="text-xs text-gray-400">Handler:</label>
          <input
            type="text"
            value={config.handler}
            onChange={handleHandlerInputChange}
            className="w-24 px-2 py-1 text-sm bg-black/30 border border-white/10 rounded text-white placeholder-gray-500"
            placeholder="handler"
          />
        </div>
      )}

      {/* Input Type Toggle */}
      <div className="flex items-center gap-2">
        <label className="text-xs text-gray-400">Input:</label>
        <div className="flex rounded overflow-hidden border border-white/10">
          <button
            onClick={() => handleInputTypeChange('json')}
            className={`px-2 py-1 text-xs flex items-center gap-1 transition-colors ${
              config.inputType === 'json'
                ? 'bg-primary-500/30 text-primary-300'
                : 'bg-black/30 text-gray-400 hover:bg-black/50'
            }`}
            title="JSON input"
          >
            <FileJson className="w-3 h-3" />
            JSON
          </button>
          <button
            onClick={() => handleInputTypeChange('node')}
            className={`px-2 py-1 text-xs flex items-center gap-1 transition-colors ${
              config.inputType === 'node'
                ? 'bg-primary-500/30 text-primary-300'
                : 'bg-black/30 text-gray-400 hover:bg-black/50'
            }`}
            title="Node input"
          >
            <Database className="w-3 h-3" />
            Node
          </button>
        </div>
      </div>

      {/* Input Value */}
      {config.inputType === 'json' ? (
        <input
          type="text"
          value={config.inputJson}
          onChange={handleInputJsonChange}
          className="flex-1 min-w-[200px] max-w-[400px] px-2 py-1 text-sm bg-black/30 border border-white/10 rounded text-white font-mono placeholder-gray-500"
          placeholder='{"key": "value"}'
        />
      ) : (
        <div className="flex items-center gap-2 flex-1 min-w-[200px] max-w-[400px]">
          <button
            onClick={onOpenNodePicker}
            className="flex-1 px-2 py-1 text-sm bg-black/30 border border-white/10 rounded text-left text-gray-300 hover:bg-black/50 flex items-center justify-between transition-colors"
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
            >
              <X className="w-4 h-4" />
            </button>
          )}
        </div>
      )}
    </div>
  )
}

// Default config factory
export function createDefaultRunConfig(): RunConfig {
  const saved = loadSavedConfig()
  return {
    handler: saved.handler || 'handler',
    inputType: saved.inputType || 'json',
    inputJson: '{}',
    inputNodeId: null,
    inputNodePath: null,
    inputWorkspace: saved.inputWorkspace || 'content',
  }
}
