/**
 * Trigger Dialog Component
 *
 * Dialog for creating and editing function triggers.
 * Supports NodeEvent, Schedule, and HTTP trigger types.
 */

import { useState } from 'react'
import { createPortal } from 'react-dom'
import { Zap, X, AlertCircle, Plus, Clock, Globe, FileCode } from 'lucide-react'
import type { TriggerCondition, TriggerType, EventKind } from '../../types'

interface TriggerDialogProps {
  trigger?: TriggerCondition | null
  onSave: (trigger: TriggerCondition) => void
  onClose: () => void
}

const EVENT_KINDS: EventKind[] = ['Created', 'Updated', 'Deleted', 'Published']

const COMMON_NODE_TYPES = [
  'raisin:Article',
  'raisin:Page',
  'raisin:User',
  'raisin:Asset',
  'raisin:Folder',
  'raisin:Product',
  'raisin:Order',
  'raisin:Comment',
]

const TRIGGER_TYPE_INFO: Record<TriggerType, { icon: React.ReactNode; label: string; description: string }> = {
  node_event: {
    icon: <FileCode className="w-5 h-5" />,
    label: 'Node Event',
    description: 'Triggered when nodes are created, updated, deleted, or published',
  },
  schedule: {
    icon: <Clock className="w-5 h-5" />,
    label: 'Schedule',
    description: 'Triggered on a recurring schedule using cron expressions',
  },
  http: {
    icon: <Globe className="w-5 h-5" />,
    label: 'HTTP',
    description: 'Triggered via HTTP requests to the function endpoint',
  },
}

// Common cron presets for quick selection
const CRON_PRESETS = [
  { label: 'Every minute', value: '* * * * *' },
  { label: 'Every 5 minutes', value: '*/5 * * * *' },
  { label: 'Every hour', value: '0 * * * *' },
  { label: 'Every day at midnight', value: '0 0 * * *' },
  { label: 'Every day at noon', value: '0 12 * * *' },
  { label: 'Every Monday at 9am', value: '0 9 * * 1' },
  { label: 'First of month at midnight', value: '0 0 1 * *' },
]

export function TriggerDialog({ trigger, onSave, onClose }: TriggerDialogProps) {
  const isEditing = !!trigger

  // Form state
  const [name, setName] = useState(trigger?.name || '')
  const [triggerType, setTriggerType] = useState<TriggerType>(trigger?.trigger_type || 'node_event')
  const [enabled, setEnabled] = useState(trigger?.enabled ?? true)
  const [priority, setPriority] = useState(trigger?.priority ?? 0)

  // Node event fields
  const [eventKinds, setEventKinds] = useState<EventKind[]>(trigger?.event_kinds || ['Created'])

  // Filter fields
  const [nodeTypes, setNodeTypes] = useState<string[]>(trigger?.filters?.node_types || [])
  const [paths, setPaths] = useState<string[]>(trigger?.filters?.paths || [])
  const [workspaces, setWorkspaces] = useState<string[]>(trigger?.filters?.workspaces || [])
  const [customNodeType, setCustomNodeType] = useState('')
  const [customPath, setCustomPath] = useState('')
  const [customWorkspace, setCustomWorkspace] = useState('')

  // Schedule fields
  const [cronExpression, setCronExpression] = useState(trigger?.cron_expression || '0 * * * *')

  // Error state
  const [error, setError] = useState<string | null>(null)

  // Validate form
  const validate = (): boolean => {
    if (!name.trim()) {
      setError('Trigger name is required')
      return false
    }

    if (triggerType === 'node_event' && eventKinds.length === 0) {
      setError('At least one event kind is required for node event triggers')
      return false
    }

    if (triggerType === 'schedule' && !cronExpression.trim()) {
      setError('Cron expression is required for schedule triggers')
      return false
    }

    setError(null)
    return true
  }

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault()

    if (!validate()) return

    const newTrigger: TriggerCondition = {
      name: name.trim(),
      trigger_type: triggerType,
      enabled,
      priority,
    }

    if (triggerType === 'node_event') {
      newTrigger.event_kinds = eventKinds
      if (nodeTypes.length > 0 || paths.length > 0 || workspaces.length > 0) {
        newTrigger.filters = {}
        if (nodeTypes.length > 0) newTrigger.filters.node_types = nodeTypes
        if (paths.length > 0) newTrigger.filters.paths = paths
        if (workspaces.length > 0) newTrigger.filters.workspaces = workspaces
      }
    } else if (triggerType === 'schedule') {
      newTrigger.cron_expression = cronExpression
    }

    onSave(newTrigger)
  }

  const toggleEventKind = (kind: EventKind) => {
    setEventKinds((prev) =>
      prev.includes(kind) ? prev.filter((k) => k !== kind) : [...prev, kind]
    )
  }

  const addNodeType = () => {
    if (customNodeType.trim() && !nodeTypes.includes(customNodeType.trim())) {
      setNodeTypes([...nodeTypes, customNodeType.trim()])
      setCustomNodeType('')
    }
  }

  const removeNodeType = (type: string) => {
    setNodeTypes(nodeTypes.filter((t) => t !== type))
  }

  const addPath = () => {
    if (customPath.trim() && !paths.includes(customPath.trim())) {
      setPaths([...paths, customPath.trim()])
      setCustomPath('')
    }
  }

  const removePath = (path: string) => {
    setPaths(paths.filter((p) => p !== path))
  }

  const addWorkspace = () => {
    if (customWorkspace.trim() && !workspaces.includes(customWorkspace.trim())) {
      setWorkspaces([...workspaces, customWorkspace.trim()])
      setCustomWorkspace('')
    }
  }

  const removeWorkspace = (ws: string) => {
    setWorkspaces(workspaces.filter((w) => w !== ws))
  }

  return createPortal(
    <div className="fixed inset-0 bg-black/50 backdrop-blur-sm flex items-center justify-center z-50 p-4">
      <div className="bg-gradient-to-br from-zinc-900 to-black border border-white/20 rounded-xl shadow-2xl w-full max-w-2xl max-h-[90vh] overflow-hidden flex flex-col">
        {/* Header */}
        <div className="flex items-center justify-between p-6 border-b border-white/10 flex-shrink-0">
          <div className="flex items-center gap-3">
            <div className="p-2 bg-yellow-500/20 rounded-lg">
              <Zap className="w-5 h-5 text-yellow-400" />
            </div>
            <div>
              <h2 className="text-xl font-semibold text-white">
                {isEditing ? 'Edit Trigger' : 'Add Trigger'}
              </h2>
              <p className="text-sm text-gray-400">
                Configure when this function should execute
              </p>
            </div>
          </div>
          <button
            onClick={onClose}
            className="p-2 hover:bg-white/10 rounded-lg transition-colors"
          >
            <X className="w-5 h-5 text-gray-400" />
          </button>
        </div>

        {/* Form */}
        <form onSubmit={handleSubmit} className="flex-1 overflow-auto p-6 space-y-6">
          {/* Trigger Name */}
          <div>
            <label className="block text-sm font-medium text-gray-300 mb-2">
              Trigger Name *
            </label>
            <input
              type="text"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="e.g., on-user-created, daily-cleanup"
              className="w-full px-4 py-2 bg-black/30 border border-white/20 rounded-lg text-white placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-yellow-500"
              autoFocus
            />
          </div>

          {/* Trigger Type Selection */}
          <div>
            <label className="block text-sm font-medium text-gray-300 mb-2">
              Trigger Type
            </label>
            <div className="grid grid-cols-3 gap-3">
              {(Object.keys(TRIGGER_TYPE_INFO) as TriggerType[]).map((type) => {
                const info = TRIGGER_TYPE_INFO[type]
                const isSelected = triggerType === type
                return (
                  <button
                    key={type}
                    type="button"
                    onClick={() => setTriggerType(type)}
                    className={`p-4 rounded-lg border text-left transition-all ${
                      isSelected
                        ? 'border-yellow-500 bg-yellow-500/10'
                        : 'border-white/10 bg-white/5 hover:bg-white/10'
                    }`}
                  >
                    <div className={`mb-2 ${isSelected ? 'text-yellow-400' : 'text-gray-400'}`}>
                      {info.icon}
                    </div>
                    <div className={`font-medium ${isSelected ? 'text-white' : 'text-gray-300'}`}>
                      {info.label}
                    </div>
                    <div className="text-xs text-gray-500 mt-1">{info.description}</div>
                  </button>
                )
              })}
            </div>
          </div>

          {/* Node Event Configuration */}
          {triggerType === 'node_event' && (
            <>
              {/* Event Kinds */}
              <div>
                <label className="block text-sm font-medium text-gray-300 mb-2">
                  Event Types *
                </label>
                <div className="flex flex-wrap gap-2">
                  {EVENT_KINDS.map((kind) => {
                    const isSelected = eventKinds.includes(kind)
                    return (
                      <button
                        key={kind}
                        type="button"
                        onClick={() => toggleEventKind(kind)}
                        className={`px-3 py-1.5 rounded-lg text-sm transition-all ${
                          isSelected
                            ? 'bg-yellow-500/20 text-yellow-300 border border-yellow-500'
                            : 'bg-white/5 text-gray-400 border border-white/10 hover:bg-white/10'
                        }`}
                      >
                        {kind}
                      </button>
                    )
                  })}
                </div>
              </div>

              {/* Node Type Filter */}
              <div>
                <label className="block text-sm font-medium text-gray-300 mb-2">
                  Node Type Filter (optional)
                </label>
                <div className="flex flex-wrap gap-2 mb-2">
                  {nodeTypes.map((type) => (
                    <span
                      key={type}
                      className="inline-flex items-center gap-1 px-2 py-1 bg-blue-500/20 text-blue-300 rounded text-sm"
                    >
                      {type}
                      <button
                        type="button"
                        onClick={() => removeNodeType(type)}
                        className="hover:text-white"
                      >
                        <X className="w-3 h-3" />
                      </button>
                    </span>
                  ))}
                </div>
                <div className="flex gap-2">
                  <select
                    value=""
                    onChange={(e) => {
                      if (e.target.value && !nodeTypes.includes(e.target.value)) {
                        setNodeTypes([...nodeTypes, e.target.value])
                      }
                    }}
                    className="flex-1 px-3 py-2 bg-black/30 border border-white/20 rounded-lg text-white focus:outline-none focus:ring-2 focus:ring-yellow-500"
                  >
                    <option value="">Select common type...</option>
                    {COMMON_NODE_TYPES.filter((t) => !nodeTypes.includes(t)).map((type) => (
                      <option key={type} value={type}>
                        {type}
                      </option>
                    ))}
                  </select>
                  <input
                    type="text"
                    value={customNodeType}
                    onChange={(e) => setCustomNodeType(e.target.value)}
                    placeholder="Custom type"
                    className="w-40 px-3 py-2 bg-black/30 border border-white/20 rounded-lg text-white placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-yellow-500"
                    onKeyDown={(e) => e.key === 'Enter' && (e.preventDefault(), addNodeType())}
                  />
                  <button
                    type="button"
                    onClick={addNodeType}
                    className="px-3 py-2 bg-white/10 hover:bg-white/20 rounded-lg text-white"
                  >
                    <Plus className="w-4 h-4" />
                  </button>
                </div>
                <p className="mt-1 text-xs text-gray-500">
                  Only trigger for nodes of these types
                </p>
              </div>

              {/* Path Filter */}
              <div>
                <label className="block text-sm font-medium text-gray-300 mb-2">
                  Path Filter (optional)
                </label>
                <div className="flex flex-wrap gap-2 mb-2">
                  {paths.map((path) => (
                    <span
                      key={path}
                      className="inline-flex items-center gap-1 px-2 py-1 bg-purple-500/20 text-purple-300 rounded text-sm font-mono"
                    >
                      {path}
                      <button
                        type="button"
                        onClick={() => removePath(path)}
                        className="hover:text-white"
                      >
                        <X className="w-3 h-3" />
                      </button>
                    </span>
                  ))}
                </div>
                <div className="flex gap-2">
                  <input
                    type="text"
                    value={customPath}
                    onChange={(e) => setCustomPath(e.target.value)}
                    placeholder="/content/articles/*"
                    className="flex-1 px-3 py-2 bg-black/30 border border-white/20 rounded-lg text-white placeholder-gray-500 font-mono focus:outline-none focus:ring-2 focus:ring-yellow-500"
                    onKeyDown={(e) => e.key === 'Enter' && (e.preventDefault(), addPath())}
                  />
                  <button
                    type="button"
                    onClick={addPath}
                    className="px-3 py-2 bg-white/10 hover:bg-white/20 rounded-lg text-white"
                  >
                    <Plus className="w-4 h-4" />
                  </button>
                </div>
                <p className="mt-1 text-xs text-gray-500">
                  Glob patterns supported: * matches any segment, ** matches multiple segments
                </p>
              </div>

              {/* Workspace Filter */}
              <div>
                <label className="block text-sm font-medium text-gray-300 mb-2">
                  Workspace Filter (optional)
                </label>
                <div className="flex flex-wrap gap-2 mb-2">
                  {workspaces.map((ws) => (
                    <span
                      key={ws}
                      className="inline-flex items-center gap-1 px-2 py-1 bg-green-500/20 text-green-300 rounded text-sm"
                    >
                      {ws}
                      <button
                        type="button"
                        onClick={() => removeWorkspace(ws)}
                        className="hover:text-white"
                      >
                        <X className="w-3 h-3" />
                      </button>
                    </span>
                  ))}
                </div>
                <div className="flex gap-2">
                  <input
                    type="text"
                    value={customWorkspace}
                    onChange={(e) => setCustomWorkspace(e.target.value)}
                    placeholder="content"
                    className="flex-1 px-3 py-2 bg-black/30 border border-white/20 rounded-lg text-white placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-yellow-500"
                    onKeyDown={(e) => e.key === 'Enter' && (e.preventDefault(), addWorkspace())}
                  />
                  <button
                    type="button"
                    onClick={addWorkspace}
                    className="px-3 py-2 bg-white/10 hover:bg-white/20 rounded-lg text-white"
                  >
                    <Plus className="w-4 h-4" />
                  </button>
                </div>
                <p className="mt-1 text-xs text-gray-500">
                  Only trigger for nodes in these workspaces
                </p>
              </div>
            </>
          )}

          {/* Schedule Configuration */}
          {triggerType === 'schedule' && (
            <div>
              <label className="block text-sm font-medium text-gray-300 mb-2">
                Cron Expression *
              </label>
              <input
                type="text"
                value={cronExpression}
                onChange={(e) => setCronExpression(e.target.value)}
                placeholder="* * * * *"
                className="w-full px-4 py-2 bg-black/30 border border-white/20 rounded-lg text-white placeholder-gray-500 font-mono focus:outline-none focus:ring-2 focus:ring-yellow-500 mb-3"
              />
              <div className="flex flex-wrap gap-2">
                {CRON_PRESETS.map((preset) => (
                  <button
                    key={preset.value}
                    type="button"
                    onClick={() => setCronExpression(preset.value)}
                    className={`px-2 py-1 rounded text-xs transition-all ${
                      cronExpression === preset.value
                        ? 'bg-yellow-500/20 text-yellow-300 border border-yellow-500'
                        : 'bg-white/5 text-gray-400 border border-white/10 hover:bg-white/10'
                    }`}
                  >
                    {preset.label}
                  </button>
                ))}
              </div>
              <p className="mt-3 text-xs text-gray-500">
                Format: minute hour day-of-month month day-of-week
              </p>
            </div>
          )}

          {/* HTTP Configuration */}
          {triggerType === 'http' && (
            <div className="p-4 bg-blue-500/10 border border-blue-500/20 rounded-lg">
              <p className="text-sm text-blue-300">
                HTTP triggers are automatically enabled when you create a function.
                Your function will be accessible at:
              </p>
              <code className="block mt-2 px-3 py-2 bg-black/30 rounded text-sm text-white font-mono">
                POST /api/functions/{'{repo}'}/{'{function-name}'}/invoke
              </code>
            </div>
          )}

          {/* Priority and Enabled */}
          <div className="grid grid-cols-2 gap-4">
            <div>
              <label className="block text-sm font-medium text-gray-300 mb-2">
                Priority
              </label>
              <input
                type="number"
                value={priority}
                onChange={(e) => setPriority(parseInt(e.target.value) || 0)}
                className="w-full px-4 py-2 bg-black/30 border border-white/20 rounded-lg text-white focus:outline-none focus:ring-2 focus:ring-yellow-500"
              />
              <p className="mt-1 text-xs text-gray-500">
                Higher priority triggers execute first
              </p>
            </div>
            <div>
              <label className="block text-sm font-medium text-gray-300 mb-2">
                Status
              </label>
              <button
                type="button"
                onClick={() => setEnabled(!enabled)}
                className={`w-full px-4 py-2 rounded-lg text-sm font-medium transition-all ${
                  enabled
                    ? 'bg-green-500/20 text-green-300 border border-green-500'
                    : 'bg-red-500/20 text-red-300 border border-red-500'
                }`}
              >
                {enabled ? 'Enabled' : 'Disabled'}
              </button>
            </div>
          </div>

          {/* Error Message */}
          {error && (
            <div className="flex items-center gap-2 p-3 bg-red-500/10 border border-red-500/20 rounded-lg">
              <AlertCircle className="w-4 h-4 text-red-400 flex-shrink-0" />
              <p className="text-sm text-red-400">{error}</p>
            </div>
          )}
        </form>

        {/* Footer */}
        <div className="flex items-center justify-end gap-3 p-6 border-t border-white/10 flex-shrink-0">
          <button
            type="button"
            onClick={onClose}
            className="px-4 py-2 text-gray-300 hover:text-white transition-colors"
          >
            Cancel
          </button>
          <button
            onClick={handleSubmit}
            className="px-6 py-2 bg-yellow-500 hover:bg-yellow-600 text-black font-medium rounded-lg transition-colors flex items-center gap-2"
          >
            <Zap className="w-4 h-4" />
            {isEditing ? 'Update Trigger' : 'Add Trigger'}
          </button>
        </div>
      </div>
    </div>,
    document.body
  )
}
