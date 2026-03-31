/**
 * Step Configuration Panel
 *
 * Side panel for configuring step-level settings like retry, timeout,
 * error handling, and AI container options.
 */

import { useMemo, useState } from 'react'
import {
  RefreshCw,
  Clock,
  AlertTriangle,
  Sparkles,
  ChevronDown,
  Info,
  Bot,
  Link2,
  Unlink,
  UserCheck,
  User,
  Plus,
  Trash2,
  MessageSquare,
} from 'lucide-react'
import { AgentPicker } from './AgentPicker'
import type {
  FlowStep,
  FlowContainer,
  FlowStepProperties,
} from '@raisindb/flow-designer'
import { isFlowStep, isFlowContainer } from '@raisindb/flow-designer'

// Retry strategy definitions
const RETRY_STRATEGIES = {
  none: { max_retries: 0, base_delay_ms: 0, max_delay_ms: 0 },
  quick: { max_retries: 3, base_delay_ms: 1000, max_delay_ms: 10000 },
  standard: { max_retries: 5, base_delay_ms: 2000, max_delay_ms: 60000 },
  aggressive: { max_retries: 10, base_delay_ms: 5000, max_delay_ms: 120000 },
  llm: { max_retries: 5, base_delay_ms: 10000, max_delay_ms: 120000 },
} as const

type RetryStrategy = keyof typeof RETRY_STRATEGIES

const RETRY_STRATEGY_LABELS: Record<RetryStrategy, string> = {
  none: 'No Retries',
  quick: 'Quick (3 retries)',
  standard: 'Standard (5 retries)',
  aggressive: 'Aggressive (10 retries)',
  llm: 'LLM Optimized',
}

const RETRY_STRATEGY_DESCRIPTIONS: Record<RetryStrategy, string> = {
  none: 'Fail immediately on error',
  quick: '1s base delay, good for transient failures',
  standard: '2s base delay, suitable for most operations',
  aggressive: '5s base delay, for critical operations',
  llm: '10s base delay, optimized for LLM rate limits',
}

// Error behavior options
const ERROR_BEHAVIORS = {
  stop: { label: 'Stop Flow', description: 'Stop the entire flow on error' },
  skip: { label: 'Skip Step', description: 'Skip this step and continue' },
  continue: { label: 'Continue', description: 'Continue to next step, ignore error' },
  rollback: { label: 'Rollback', description: 'Trigger compensation/rollback' },
} as const

type ErrorBehavior = keyof typeof ERROR_BEHAVIORS

// Execution identity modes (FR-028)
const EXECUTION_IDENTITY_MODES = {
  agent: { label: 'Agent Identity', description: "Use AI agent's service account identity" },
  caller: { label: 'Caller Identity', description: "Use triggering user's identity for attribution" },
  function: { label: 'Function Identity', description: 'Use elevated function service account (delegation)' },
} as const

// AI tool modes
const AI_TOOL_MODES = {
  auto: { label: 'Auto', description: 'Agent handles tool calls internally' },
  explicit: { label: 'Explicit', description: 'Tool calls appear as child steps' },
  hybrid: { label: 'Hybrid', description: 'Some tools internal, others explicit' },
} as const

type AiToolMode = keyof typeof AI_TOOL_MODES

// Human task types
const HUMAN_TASK_TYPES = {
  approval: { label: 'Approval', description: 'User approves or rejects' },
  input: { label: 'Input', description: 'User provides form data' },
  review: { label: 'Review', description: 'User reviews content' },
  action: { label: 'Action', description: 'User takes an action' },
} as const

type HumanTaskType = keyof typeof HUMAN_TASK_TYPES

// Human task priorities
const TASK_PRIORITIES = [
  { value: 1, label: 'Low', color: 'text-gray-400' },
  { value: 2, label: 'Normal', color: 'text-blue-400' },
  { value: 3, label: 'Medium', color: 'text-yellow-400' },
  { value: 4, label: 'High', color: 'text-orange-400' },
  { value: 5, label: 'Critical', color: 'text-red-400' },
] as const

// Task option for approval type
interface TaskOption {
  value: string
  label: string
  style?: 'default' | 'success' | 'danger' | 'warning'
}

const OPTION_STYLES = [
  { value: 'default', label: 'Default', className: 'bg-gray-500/20' },
  { value: 'success', label: 'Success', className: 'bg-green-500/20' },
  { value: 'danger', label: 'Danger', className: 'bg-red-500/20' },
  { value: 'warning', label: 'Warning', className: 'bg-yellow-500/20' },
] as const

interface StepConfigPanelProps {
  /** The selected node (step or container) */
  node: FlowStep | FlowContainer
  /** Callback when step properties change */
  onUpdateStep: (updates: Partial<FlowStepProperties>) => void
  /** Callback when container properties change */
  onUpdateContainer?: (updates: {
    ai_config?: AiContainerConfig
    timeout_ms?: number
    properties?: Record<string, unknown>
  }) => void
  /** Mark tab as dirty */
  onDirty: () => void
}

interface AiContainerConfig {
  agent_ref?: { 'raisin:ref': string; 'raisin:workspace': string; 'raisin:path'?: string }
  tool_mode: AiToolMode
  explicit_tools: string[]
  max_iterations: number
  thinking_enabled: boolean
  on_error: 'stop' | 'continue' | 'retry'
  timeout_ms?: number
}

export function StepConfigPanel({
  node,
  onUpdateStep,
  onUpdateContainer,
  onDirty,
}: StepConfigPanelProps) {
  // Determine if this is a step or container
  const isStep = isFlowStep(node)
  const isContainer = isFlowContainer(node)
  const isAiContainer = isContainer && (node as FlowContainer).container_type === 'ai_sequence'
  const isHumanTask = isStep && (node as FlowStep).properties?.step_type === 'human_task'
  const isChatStep = isStep && (node as FlowStep).properties?.step_type === 'chat'

  // Get current values for step
  const stepProps = isStep ? (node as FlowStep).properties : null
  const currentRetryStrategy = stepProps?.retry_strategy || 'none'
  const currentTimeout = stepProps?.timeout_ms
  const currentErrorBehavior = (node as FlowStep).on_error || 'stop'

  // Get AI config for container
  const containerNode = isContainer ? (node as FlowContainer) : null
  const aiConfig = (containerNode as any)?.ai_config as AiContainerConfig | undefined

  // Agent picker state
  const [showAgentPicker, setShowAgentPicker] = useState(false)
  const [showChatAgentPicker, setShowChatAgentPicker] = useState(false)

  // Chat step configuration
  const chatConfig = stepProps?.chat_config as {
    agent_ref?: { 'raisin:ref': string; 'raisin:workspace': string; 'raisin:path'?: string }
    system_prompt?: string
    handoff_targets: Array<{
      agent_ref: { 'raisin:ref': string; 'raisin:workspace': string; 'raisin:path'?: string }
      trigger_phrases?: string[]
      trigger_condition?: string
    }>
    session_timeout_ms?: number
    max_turns: number
    termination: {
      modes: Array<'user_request' | 'max_turns' | 'inactivity' | 'ai_decision'>
      inactivity_timeout_ms?: number
      termination_phrases?: string[]
    }
  } | undefined

  // Detect custom retry config
  const hasCustomRetry = useMemo(() => {
    if (!stepProps?.retry) return false
    const strategy = RETRY_STRATEGIES[currentRetryStrategy]
    return (
      stepProps.retry.max_retries !== strategy.max_retries ||
      stepProps.retry.base_delay_ms !== strategy.base_delay_ms ||
      stepProps.retry.max_delay_ms !== strategy.max_delay_ms
    )
  }, [stepProps?.retry, currentRetryStrategy])

  // Handlers
  const handleRetryStrategyChange = (strategy: RetryStrategy) => {
    const config = RETRY_STRATEGIES[strategy]
    onUpdateStep({
      retry_strategy: strategy,
      retry: strategy === 'none' ? undefined : config,
    })
    onDirty()
  }

  const handleCustomRetryChange = (field: keyof typeof RETRY_STRATEGIES.standard, value: number) => {
    const currentRetry = stepProps?.retry || RETRY_STRATEGIES.standard
    onUpdateStep({
      retry_strategy: undefined, // Clear preset when customizing
      retry: {
        ...currentRetry,
        [field]: value,
      },
    })
    onDirty()
  }

  const handleTimeoutChange = (timeout: number | undefined) => {
    if (isStep) {
      onUpdateStep({ timeout_ms: timeout })
    } else if (isAiContainer && onUpdateContainer) {
      onUpdateContainer({ timeout_ms: timeout })
    }
    onDirty()
  }

  const handleErrorBehaviorChange = (behavior: ErrorBehavior) => {
    // Note: on_error is at FlowStep level, not properties
    // This would need to be handled via a different mechanism
    // For now, we'll store it in properties as a workaround
    onUpdateStep({ on_error: behavior } as any)
    onDirty()
  }

  const handleAiConfigChange = (updates: Partial<AiContainerConfig>) => {
    if (!onUpdateContainer || !isAiContainer) return
    onUpdateContainer({
      ai_config: {
        ...aiConfig,
        tool_mode: aiConfig?.tool_mode || 'auto',
        explicit_tools: aiConfig?.explicit_tools || [],
        max_iterations: aiConfig?.max_iterations || 10,
        thinking_enabled: aiConfig?.thinking_enabled || false,
        on_error: aiConfig?.on_error || 'stop',
        ...updates,
      },
    })
    onDirty()
  }

  // Human task option handlers
  const taskOptions: TaskOption[] = (stepProps?.options as TaskOption[]) || []

  const handleAddOption = () => {
    const newOption: TaskOption = {
      value: `option_${taskOptions.length + 1}`,
      label: `Option ${taskOptions.length + 1}`,
      style: 'default',
    }
    onUpdateStep({ options: [...taskOptions, newOption] } as any)
    onDirty()
  }

  const handleUpdateOption = (index: number, updates: Partial<TaskOption>) => {
    const updated = [...taskOptions]
    updated[index] = { ...updated[index], ...updates }
    onUpdateStep({ options: updated } as any)
    onDirty()
  }

  const handleRemoveOption = (index: number) => {
    const updated = taskOptions.filter((_, i) => i !== index)
    onUpdateStep({ options: updated } as any)
    onDirty()
  }

  // Chat config handlers
  const handleChatConfigChange = (updates: Partial<typeof chatConfig>) => {
    const currentConfig = chatConfig || {
      handoff_targets: [],
      max_turns: 20,
      termination: { modes: ['user_request', 'max_turns'] },
    }
    onUpdateStep({
      chat_config: {
        ...currentConfig,
        ...updates,
      },
    } as any)
    onDirty()
  }

  const handleTerminationModeToggle = (mode: 'user_request' | 'max_turns' | 'inactivity' | 'ai_decision') => {
    const currentModes = chatConfig?.termination?.modes || ['user_request', 'max_turns']
    const newModes = currentModes.includes(mode)
      ? currentModes.filter(m => m !== mode)
      : [...currentModes, mode]
    handleChatConfigChange({
      termination: {
        ...chatConfig?.termination,
        modes: newModes.length > 0 ? newModes : ['user_request'], // Ensure at least one mode
      },
    })
  }

  return (
    <div className="space-y-6">
      {/* Human Task Configuration */}
      {isHumanTask && (
        <div className="space-y-4">
          <div className="flex items-center gap-2">
            <UserCheck className="w-4 h-4 text-amber-400" />
            <h4 className="text-sm font-medium text-white">Human Task Settings</h4>
          </div>

          {/* Task Type */}
          <div className="space-y-2">
            <label className="block text-xs text-gray-400">Task Type</label>
            <div className="relative">
              <select
                value={stepProps?.task_type || 'approval'}
                onChange={(e) => {
                  onUpdateStep({ task_type: e.target.value as HumanTaskType })
                  onDirty()
                }}
                className="w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white text-sm appearance-none cursor-pointer focus:outline-none focus:ring-2 focus:ring-amber-500"
              >
                {Object.entries(HUMAN_TASK_TYPES).map(([key, { label }]) => (
                  <option key={key} value={key} className="bg-gray-800">
                    {label}
                  </option>
                ))}
              </select>
              <ChevronDown className="absolute right-3 top-1/2 -translate-y-1/2 w-4 h-4 text-gray-400 pointer-events-none" />
            </div>
            <p className="text-xs text-gray-500">
              {HUMAN_TASK_TYPES[(stepProps?.task_type as HumanTaskType) || 'approval']?.description}
            </p>
          </div>

          {/* Assignee */}
          <div className="space-y-2">
            <label className="block text-xs text-gray-400">
              Assignee <span className="text-red-400">*</span>
            </label>
            <div className="flex items-center gap-2">
              <User className="w-4 h-4 text-gray-400 flex-shrink-0" />
              <input
                type="text"
                value={stepProps?.assignee || ''}
                onChange={(e) => {
                  onUpdateStep({ assignee: e.target.value })
                  onDirty()
                }}
                placeholder="users/manager"
                className="flex-1 px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white text-sm placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-amber-500"
              />
            </div>
            <p className="text-xs text-gray-500">
              Path to the user who will receive this task (e.g., users/manager)
            </p>
          </div>

          {/* Description */}
          <div className="space-y-2">
            <label className="block text-xs text-gray-400">Description</label>
            <textarea
              value={stepProps?.task_description || ''}
              onChange={(e) => {
                onUpdateStep({ task_description: e.target.value })
                onDirty()
              }}
              placeholder="Describe what the user needs to do..."
              rows={3}
              className="w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white text-sm placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-amber-500 resize-none"
            />
          </div>

          {/* Options (only for approval type) */}
          {stepProps?.task_type === 'approval' && (
            <div className="space-y-2">
              <label className="block text-xs text-gray-400">Approval Options</label>
              <div className="space-y-2">
                {taskOptions.map((option, index) => (
                  <div
                    key={index}
                    className="flex items-center gap-2 p-2 bg-white/5 border border-white/10 rounded-lg"
                  >
                    <input
                      type="text"
                      value={option.value}
                      onChange={(e) => handleUpdateOption(index, { value: e.target.value })}
                      placeholder="Value"
                      className="w-24 px-2 py-1 bg-white/5 border border-white/10 rounded text-white text-xs focus:outline-none focus:ring-1 focus:ring-amber-500"
                    />
                    <input
                      type="text"
                      value={option.label}
                      onChange={(e) => handleUpdateOption(index, { label: e.target.value })}
                      placeholder="Label"
                      className="flex-1 px-2 py-1 bg-white/5 border border-white/10 rounded text-white text-xs focus:outline-none focus:ring-1 focus:ring-amber-500"
                    />
                    <select
                      value={option.style || 'default'}
                      onChange={(e) =>
                        handleUpdateOption(index, {
                          style: e.target.value as TaskOption['style'],
                        })
                      }
                      className="px-2 py-1 bg-white/5 border border-white/10 rounded text-white text-xs appearance-none cursor-pointer focus:outline-none focus:ring-1 focus:ring-amber-500"
                    >
                      {OPTION_STYLES.map((s) => (
                        <option key={s.value} value={s.value} className="bg-gray-800">
                          {s.label}
                        </option>
                      ))}
                    </select>
                    <button
                      onClick={() => handleRemoveOption(index)}
                      className="p-1 text-gray-400 hover:text-red-400 transition-colors"
                      title="Remove option"
                    >
                      <Trash2 className="w-3.5 h-3.5" />
                    </button>
                  </div>
                ))}
                <button
                  onClick={handleAddOption}
                  className="w-full flex items-center justify-center gap-2 px-3 py-2 bg-amber-500/10 border border-amber-500/30 rounded-lg text-amber-400 hover:bg-amber-500/20 transition-colors text-sm"
                >
                  <Plus className="w-4 h-4" />
                  <span>Add Option</span>
                </button>
              </div>
              <p className="text-xs text-gray-500">
                Options presented to the user for approval (e.g., Approve/Reject)
              </p>
            </div>
          )}

          {/* Priority */}
          <div className="space-y-2">
            <label className="block text-xs text-gray-400">Priority</label>
            <div className="relative">
              <select
                value={stepProps?.priority?.toString() || '3'}
                onChange={(e) => {
                  onUpdateStep({ priority: parseInt(e.target.value) })
                  onDirty()
                }}
                className="w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white text-sm appearance-none cursor-pointer focus:outline-none focus:ring-2 focus:ring-amber-500"
              >
                {TASK_PRIORITIES.map(({ value, label }) => (
                  <option key={value} value={value} className="bg-gray-800">
                    {label}
                  </option>
                ))}
              </select>
              <ChevronDown className="absolute right-3 top-1/2 -translate-y-1/2 w-4 h-4 text-gray-400 pointer-events-none" />
            </div>
          </div>

          {/* Due Time */}
          <div className="space-y-2">
            <label className="block text-xs text-gray-400">Due in (hours)</label>
            <input
              type="number"
              min={0}
              max={720}
              step={1}
              value={stepProps?.due_in_seconds ? Math.round((stepProps.due_in_seconds as number) / 3600) : ''}
              onChange={(e) => {
                const hours = e.target.value ? parseInt(e.target.value) : undefined
                onUpdateStep({ due_in_seconds: hours ? hours * 3600 : undefined })
                onDirty()
              }}
              placeholder="No deadline"
              className="w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white text-sm placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-amber-500"
            />
            <p className="text-xs text-gray-500">
              {stepProps?.due_in_seconds
                ? `Task due in ${Math.round((stepProps.due_in_seconds as number) / 3600)} hours`
                : 'No deadline set'}
            </p>
          </div>
        </div>
      )}

      {/* Chat Step Configuration */}
      {isChatStep && (
        <div className="space-y-4">
          <div className="flex items-center gap-2">
            <MessageSquare className="w-4 h-4 text-cyan-400" />
            <h4 className="text-sm font-medium text-white">Chat Session Settings</h4>
          </div>

          {/* Agent Selection */}
          <div className="space-y-2">
            <label className="block text-xs text-gray-400">
              Chat Agent <span className="text-red-400">*</span>
            </label>
            {chatConfig?.agent_ref ? (
              <div className="flex items-center gap-2 px-3 py-2 bg-white/5 border border-white/10 rounded-lg">
                <Bot className="w-4 h-4 text-cyan-400 flex-shrink-0" />
                <span className="text-sm text-white truncate flex-1">
                  {chatConfig.agent_ref['raisin:path'] || chatConfig.agent_ref['raisin:ref']}
                </span>
                <button
                  onClick={() => setShowChatAgentPicker(true)}
                  className="text-gray-400 hover:text-white"
                  title="Change agent"
                >
                  <Link2 className="w-4 h-4" />
                </button>
                <button
                  onClick={() => handleChatConfigChange({ agent_ref: undefined })}
                  className="text-gray-400 hover:text-red-400"
                  title="Remove agent"
                >
                  <Unlink className="w-4 h-4" />
                </button>
              </div>
            ) : (
              <button
                onClick={() => setShowChatAgentPicker(true)}
                className="w-full flex items-center gap-2 px-3 py-2 bg-cyan-500/10 border border-cyan-500/30 rounded-lg text-cyan-400 hover:bg-cyan-500/20 transition-colors"
              >
                <Bot className="w-4 h-4" />
                <span className="text-sm">Select Chat Agent</span>
              </button>
            )}
            <p className="text-xs text-gray-500">
              The AI agent that handles the chat session
            </p>
          </div>

          {/* Max Turns */}
          <div className="space-y-2">
            <label className="block text-xs text-gray-400">Max Turns</label>
            <input
              type="number"
              min={1}
              max={100}
              value={chatConfig?.max_turns ?? 20}
              onChange={(e) => handleChatConfigChange({ max_turns: parseInt(e.target.value) || 20 })}
              className="w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white text-sm focus:outline-none focus:ring-2 focus:ring-cyan-500"
            />
            <p className="text-xs text-gray-500">
              Maximum conversation turns before auto-termination
            </p>
          </div>

          {/* Session Timeout */}
          <div className="space-y-2">
            <label className="block text-xs text-gray-400">Session Timeout (minutes)</label>
            <input
              type="number"
              min={1}
              max={1440}
              value={chatConfig?.session_timeout_ms ? Math.round(chatConfig.session_timeout_ms / 60000) : ''}
              onChange={(e) => {
                const minutes = e.target.value ? parseInt(e.target.value) : undefined
                handleChatConfigChange({ session_timeout_ms: minutes ? minutes * 60000 : undefined })
              }}
              placeholder="No timeout"
              className="w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white text-sm placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-cyan-500"
            />
          </div>

          {/* Termination Modes */}
          <div className="space-y-2">
            <label className="block text-xs text-gray-400">Termination Triggers</label>
            <div className="grid grid-cols-2 gap-2">
              {[
                { mode: 'user_request' as const, label: 'User Request', desc: 'User explicitly ends chat' },
                { mode: 'max_turns' as const, label: 'Max Turns', desc: 'Turn limit reached' },
                { mode: 'inactivity' as const, label: 'Inactivity', desc: 'No response timeout' },
                { mode: 'ai_decision' as const, label: 'AI Decision', desc: 'Agent decides to end' },
              ].map(({ mode, label, desc }) => {
                const isActive = chatConfig?.termination?.modes?.includes(mode) ??
                  (mode === 'user_request' || mode === 'max_turns')
                return (
                  <button
                    key={mode}
                    onClick={() => handleTerminationModeToggle(mode)}
                    className={`p-2 rounded-lg border text-left transition-colors ${
                      isActive
                        ? 'bg-cyan-500/20 border-cyan-500/50 text-cyan-400'
                        : 'bg-white/5 border-white/10 text-gray-400 hover:bg-white/10'
                    }`}
                  >
                    <div className="text-xs font-medium">{label}</div>
                    <div className="text-[10px] opacity-70 mt-0.5">{desc}</div>
                  </button>
                )
              })}
            </div>
          </div>

          {/* System Prompt Override */}
          <div className="space-y-2">
            <label className="block text-xs text-gray-400">System Prompt (optional)</label>
            <textarea
              value={chatConfig?.system_prompt || ''}
              onChange={(e) => handleChatConfigChange({ system_prompt: e.target.value || undefined })}
              placeholder="Override the agent's default system prompt..."
              rows={3}
              className="w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white text-sm placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-cyan-500 resize-none"
            />
            <p className="text-xs text-gray-500">
              Leave empty to use the agent's default prompt
            </p>
          </div>

          {/* Handoff Targets Count */}
          {chatConfig?.handoff_targets && chatConfig.handoff_targets.length > 0 && (
            <div className="p-2 bg-white/5 border border-white/10 rounded-lg">
              <p className="text-xs text-gray-400">
                <span className="text-cyan-400 font-medium">{chatConfig.handoff_targets.length}</span> handoff target{chatConfig.handoff_targets.length > 1 ? 's' : ''} configured
              </p>
              <p className="text-[10px] text-gray-500 mt-1">
                Edit handoff targets in the flow designer YAML view
              </p>
            </div>
          )}
        </div>
      )}

      {/* Step Configuration (for non-human-task and non-chat steps only) */}
      {isStep && !isHumanTask && !isChatStep && (
        <>
          {/* Retry Configuration */}
          <div className="space-y-3">
            <div className="flex items-center gap-2">
              <RefreshCw className="w-4 h-4 text-blue-400" />
              <h4 className="text-sm font-medium text-white">Retry Strategy</h4>
            </div>

            {/* Strategy selector */}
            <div className="relative">
              <select
                value={hasCustomRetry ? 'custom' : currentRetryStrategy}
                onChange={(e) => {
                  if (e.target.value !== 'custom') {
                    handleRetryStrategyChange(e.target.value as RetryStrategy)
                  }
                }}
                className="w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white text-sm appearance-none cursor-pointer focus:outline-none focus:ring-2 focus:ring-blue-500"
              >
                {Object.entries(RETRY_STRATEGY_LABELS).map(([key, label]) => (
                  <option key={key} value={key} className="bg-gray-800">
                    {label}
                  </option>
                ))}
                {hasCustomRetry && (
                  <option value="custom" className="bg-gray-800">
                    Custom
                  </option>
                )}
              </select>
              <ChevronDown className="absolute right-3 top-1/2 -translate-y-1/2 w-4 h-4 text-gray-400 pointer-events-none" />
            </div>

            {/* Strategy description */}
            {!hasCustomRetry && currentRetryStrategy !== 'none' && (
              <p className="text-xs text-gray-500 flex items-start gap-1.5">
                <Info className="w-3 h-3 mt-0.5 flex-shrink-0" />
                {RETRY_STRATEGY_DESCRIPTIONS[currentRetryStrategy]}
              </p>
            )}

            {/* Custom retry config */}
            {(currentRetryStrategy !== 'none' || hasCustomRetry) && (
              <div className="space-y-2 pl-2 border-l-2 border-white/10">
                <div className="grid grid-cols-2 gap-2">
                  <div>
                    <label className="block text-xs text-gray-500 mb-1">Max Retries</label>
                    <input
                      type="number"
                      min={0}
                      max={20}
                      value={stepProps?.retry?.max_retries ?? RETRY_STRATEGIES[currentRetryStrategy].max_retries}
                      onChange={(e) => handleCustomRetryChange('max_retries', parseInt(e.target.value) || 0)}
                      className="w-full px-2 py-1.5 bg-white/5 border border-white/10 rounded text-white text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
                    />
                  </div>
                  <div>
                    <label className="block text-xs text-gray-500 mb-1">Base Delay (ms)</label>
                    <input
                      type="number"
                      min={100}
                      max={60000}
                      step={100}
                      value={stepProps?.retry?.base_delay_ms ?? RETRY_STRATEGIES[currentRetryStrategy].base_delay_ms}
                      onChange={(e) => handleCustomRetryChange('base_delay_ms', parseInt(e.target.value) || 1000)}
                      className="w-full px-2 py-1.5 bg-white/5 border border-white/10 rounded text-white text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
                    />
                  </div>
                </div>
                <div>
                  <label className="block text-xs text-gray-500 mb-1">Max Delay (ms)</label>
                  <input
                    type="number"
                    min={1000}
                    max={300000}
                    step={1000}
                    value={stepProps?.retry?.max_delay_ms ?? RETRY_STRATEGIES[currentRetryStrategy].max_delay_ms}
                    onChange={(e) => handleCustomRetryChange('max_delay_ms', parseInt(e.target.value) || 60000)}
                    className="w-full px-2 py-1.5 bg-white/5 border border-white/10 rounded text-white text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
                  />
                </div>
              </div>
            )}
          </div>

          {/* Timeout Configuration */}
          <div className="space-y-3">
            <div className="flex items-center gap-2">
              <Clock className="w-4 h-4 text-yellow-400" />
              <h4 className="text-sm font-medium text-white">Step Timeout</h4>
            </div>
            <div className="flex items-center gap-2">
              <input
                type="number"
                min={0}
                max={600000}
                step={1000}
                value={currentTimeout || ''}
                onChange={(e) => handleTimeoutChange(e.target.value ? parseInt(e.target.value) : undefined)}
                placeholder="Default (no limit)"
                className="flex-1 px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white text-sm placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-blue-500"
              />
              <span className="text-xs text-gray-500">ms</span>
            </div>
            <p className="text-xs text-gray-500">
              {currentTimeout
                ? `Timeout: ${(currentTimeout / 1000).toFixed(1)}s`
                : 'No timeout configured (uses flow default)'}
            </p>
          </div>

          {/* Error Behavior */}
          <div className="space-y-3">
            <div className="flex items-center gap-2">
              <AlertTriangle className="w-4 h-4 text-orange-400" />
              <h4 className="text-sm font-medium text-white">On Error</h4>
            </div>
            <div className="grid grid-cols-2 gap-2">
              {Object.entries(ERROR_BEHAVIORS).map(([key, { label, description }]) => (
                <button
                  key={key}
                  onClick={() => handleErrorBehaviorChange(key as ErrorBehavior)}
                  className={`p-2 rounded-lg border text-left transition-colors ${
                    currentErrorBehavior === key
                      ? 'bg-blue-500/20 border-blue-500/50 text-blue-400'
                      : 'bg-white/5 border-white/10 text-gray-400 hover:bg-white/10'
                  }`}
                >
                  <div className="text-xs font-medium">{label}</div>
                  <div className="text-[10px] opacity-70 mt-0.5">{description}</div>
                </button>
              ))}
            </div>
          </div>

          {/* Execution Identity (FR-028) */}
          <div className="space-y-3">
            <div className="flex items-center gap-2">
              <User className="w-4 h-4 text-cyan-400" />
              <h4 className="text-sm font-medium text-white">Execution Identity</h4>
            </div>
            <div className="space-y-2">
              {Object.entries(EXECUTION_IDENTITY_MODES).map(([key, { label, description }]) => (
                <label
                  key={key}
                  className={`flex items-start gap-3 p-2 rounded-lg border cursor-pointer transition-colors ${
                    (stepProps?.execution_identity || 'agent') === key
                      ? 'bg-cyan-500/20 border-cyan-500/50'
                      : 'bg-white/5 border-white/10 hover:bg-white/10'
                  }`}
                >
                  <input
                    type="radio"
                    name="execution_identity"
                    value={key}
                    checked={(stepProps?.execution_identity || 'agent') === key}
                    onChange={() => {
                      onUpdateStep({ execution_identity: key } as any)
                      onDirty()
                    }}
                    className="mt-0.5"
                  />
                  <div>
                    <div className="text-sm text-white">{label}</div>
                    <div className="text-xs text-gray-500">{description}</div>
                  </div>
                </label>
              ))}
            </div>
            <p className="text-xs text-gray-500">
              Controls whose permissions are used when this step accesses data
            </p>
          </div>

          {/* Isolated Branch Mode (FR-033) */}
          <div className="flex items-center justify-between">
            <div>
              <label className="block text-sm text-white">Isolated Branch</label>
              <p className="text-xs text-gray-500">Execute step in git-like isolated branch for safety</p>
            </div>
            <div
              className={`w-10 h-5 rounded-full relative cursor-pointer transition-colors ${
                stepProps?.isolated_branch ? 'bg-cyan-500' : 'bg-gray-600'
              }`}
              onClick={() => {
                onUpdateStep({ isolated_branch: !stepProps?.isolated_branch } as any)
                onDirty()
              }}
            >
              <div
                className={`w-4 h-4 rounded-full bg-white absolute top-0.5 transition-all ${
                  stepProps?.isolated_branch ? 'left-5' : 'left-0.5'
                }`}
              />
            </div>
          </div>
        </>
      )}

      {/* AI Container Configuration */}
      {isAiContainer && (
        <div className="space-y-4">
          <div className="flex items-center gap-2">
            <Sparkles className="w-4 h-4 text-purple-400" />
            <h4 className="text-sm font-medium text-white">AI Container Settings</h4>
          </div>

          {/* Agent Selection */}
          <div className="space-y-2">
            <label className="block text-xs text-gray-400">AI Agent</label>
            {aiConfig?.agent_ref ? (
              <div className="flex items-center gap-2 px-3 py-2 bg-white/5 border border-white/10 rounded-lg">
                <Bot className="w-4 h-4 text-purple-400 flex-shrink-0" />
                <span className="text-sm text-white truncate flex-1">
                  {aiConfig.agent_ref['raisin:path'] || aiConfig.agent_ref['raisin:ref']}
                </span>
                <button
                  onClick={() => setShowAgentPicker(true)}
                  className="text-gray-400 hover:text-white"
                  title="Change agent"
                >
                  <Link2 className="w-4 h-4" />
                </button>
                <button
                  onClick={() => handleAiConfigChange({ agent_ref: undefined })}
                  className="text-gray-400 hover:text-red-400"
                  title="Remove agent"
                >
                  <Unlink className="w-4 h-4" />
                </button>
              </div>
            ) : (
              <button
                onClick={() => setShowAgentPicker(true)}
                className="w-full flex items-center gap-2 px-3 py-2 bg-purple-500/10 border border-purple-500/30 rounded-lg text-purple-400 hover:bg-purple-500/20 transition-colors"
              >
                <Bot className="w-4 h-4" />
                <span className="text-sm">Select Agent</span>
              </button>
            )}
            <p className="text-xs text-gray-500">
              Select the AI agent that will handle this container's execution
            </p>
          </div>

          {/* Tool Mode */}
          <div className="space-y-2">
            <label className="block text-xs text-gray-400">Tool Execution Mode</label>
            <div className="space-y-2">
              {Object.entries(AI_TOOL_MODES).map(([key, { label, description }]) => (
                <label
                  key={key}
                  className={`flex items-start gap-3 p-2 rounded-lg border cursor-pointer transition-colors ${
                    (aiConfig?.tool_mode || 'auto') === key
                      ? 'bg-purple-500/20 border-purple-500/50'
                      : 'bg-white/5 border-white/10 hover:bg-white/10'
                  }`}
                >
                  <input
                    type="radio"
                    name="tool_mode"
                    value={key}
                    checked={(aiConfig?.tool_mode || 'auto') === key}
                    onChange={() => handleAiConfigChange({ tool_mode: key as AiToolMode })}
                    className="mt-0.5"
                  />
                  <div>
                    <div className="text-sm text-white">{label}</div>
                    <div className="text-xs text-gray-500">{description}</div>
                  </div>
                </label>
              ))}
            </div>
          </div>

          {/* Max Iterations */}
          <div className="space-y-2">
            <label className="block text-xs text-gray-400">Max Iterations</label>
            <input
              type="number"
              min={1}
              max={50}
              value={aiConfig?.max_iterations ?? 10}
              onChange={(e) => handleAiConfigChange({ max_iterations: parseInt(e.target.value) || 10 })}
              className="w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white text-sm focus:outline-none focus:ring-2 focus:ring-purple-500"
            />
            <p className="text-xs text-gray-500">
              Maximum tool call iterations before stopping
            </p>
          </div>

          {/* Thinking Enabled */}
          <div className="flex items-center justify-between">
            <div>
              <label className="block text-sm text-white">Show Thinking</label>
              <p className="text-xs text-gray-500">Display AI reasoning process</p>
            </div>
            <div
              className={`w-10 h-5 rounded-full relative cursor-pointer transition-colors ${
                aiConfig?.thinking_enabled ? 'bg-purple-500' : 'bg-gray-600'
              }`}
              onClick={() => handleAiConfigChange({ thinking_enabled: !aiConfig?.thinking_enabled })}
            >
              <div
                className={`w-4 h-4 rounded-full bg-white absolute top-0.5 transition-all ${
                  aiConfig?.thinking_enabled ? 'left-5' : 'left-0.5'
                }`}
              />
            </div>
          </div>

          {/* Container Timeout */}
          <div className="space-y-2">
            <label className="block text-xs text-gray-400">Container Timeout (ms)</label>
            <input
              type="number"
              min={0}
              max={1800000}
              step={1000}
              value={aiConfig?.timeout_ms || containerNode?.timeout_ms || ''}
              onChange={(e) => handleTimeoutChange(e.target.value ? parseInt(e.target.value) : undefined)}
              placeholder="No limit"
              className="w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white text-sm placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-purple-500"
            />
          </div>

          {/* Error Handling */}
          <div className="space-y-2">
            <label className="block text-xs text-gray-400">On Error</label>
            <div className="relative">
              <select
                value={aiConfig?.on_error || 'stop'}
                onChange={(e) => handleAiConfigChange({ on_error: e.target.value as any })}
                className="w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white text-sm appearance-none cursor-pointer focus:outline-none focus:ring-2 focus:ring-purple-500"
              >
                <option value="stop" className="bg-gray-800">Stop - Halt on error</option>
                <option value="continue" className="bg-gray-800">Continue - Use last response</option>
                <option value="retry" className="bg-gray-800">Retry - Exponential backoff</option>
              </select>
              <ChevronDown className="absolute right-3 top-1/2 -translate-y-1/2 w-4 h-4 text-gray-400 pointer-events-none" />
            </div>
          </div>

          {/* Explicit Tools (for hybrid mode) */}
          {aiConfig?.tool_mode === 'hybrid' && (
            <div className="space-y-2">
              <label className="block text-xs text-gray-400">Explicit Tools</label>
              <textarea
                value={aiConfig?.explicit_tools?.join('\n') || ''}
                onChange={(e) =>
                  handleAiConfigChange({
                    explicit_tools: e.target.value.split('\n').filter(Boolean),
                  })
                }
                placeholder="Enter tool names (one per line)"
                rows={3}
                className="w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white text-sm placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-purple-500 resize-none font-mono"
              />
              <p className="text-xs text-gray-500">
                Tools listed here will appear as explicit child steps
              </p>
            </div>
          )}
        </div>
      )}

      {/* Container info for non-AI containers */}
      {isContainer && !isAiContainer && (
        <div className="space-y-3">
          <div className="flex items-center gap-2">
            <Info className="w-4 h-4 text-gray-400" />
            <h4 className="text-sm font-medium text-white">Container Settings</h4>
          </div>
          <p className="text-xs text-gray-500">
            Container type: <span className="text-white font-medium">{containerNode?.container_type.toUpperCase()}</span>
          </p>

          {/* Merge Strategy for Parallel containers */}
          {containerNode?.container_type === 'parallel' && onUpdateContainer && (
            <div>
              <label className="block text-xs text-gray-400 mb-1.5">
                Join Mode (Merge Strategy)
              </label>
              <select
                value={(containerNode as any).properties?.merge_strategy || 'merge_all'}
                onChange={(e) => {
                  // Update container properties
                  const currentProps = (containerNode as any).properties || {}
                  onUpdateContainer({
                    properties: {
                      ...currentProps,
                      merge_strategy: e.target.value,
                    },
                  })
                  onDirty()
                }}
                className="w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
              >
                <option value="merge_all">Merge All - Collect all branch outputs</option>
                <option value="first_success">First Success - Return first successful branch</option>
                <option value="all_success">All Success - Fail if any branch fails</option>
              </select>
              <p className="text-xs text-gray-500 mt-1">
                How to combine outputs when parallel branches complete
              </p>
            </div>
          )}

          <p className="text-xs text-gray-500">
            Select a step inside this container to configure retry and timeout settings.
          </p>
        </div>
      )}

      {/* Agent Picker Modal (AI Container) */}
      {showAgentPicker && (
        <AgentPicker
          currentAgentPath={aiConfig?.agent_ref?.['raisin:path']}
          onSelect={(ref) => {
            handleAiConfigChange({ agent_ref: ref })
            setShowAgentPicker(false)
          }}
          onClose={() => setShowAgentPicker(false)}
        />
      )}

      {/* Agent Picker Modal (Chat Step) */}
      {showChatAgentPicker && (
        <AgentPicker
          currentAgentPath={chatConfig?.agent_ref?.['raisin:path']}
          onSelect={(ref) => {
            handleChatConfigChange({ agent_ref: ref })
            setShowChatAgentPicker(false)
          }}
          onClose={() => setShowChatAgentPicker(false)}
        />
      )}
    </div>
  )
}

export default StepConfigPanel
