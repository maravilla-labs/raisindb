/**
 * Properties Panel Component
 *
 * Shows basic node information for any selected node.
 * For functions, also displays triggers in read-only mode.
 */

import { useState } from 'react'
import { Info, Zap, ChevronDown, ChevronRight, Folder, SquareFunction, File } from 'lucide-react'
import { useFunctionsContext } from '../../hooks'
import type { TriggerCondition, FunctionNode } from '../../types'

interface SectionProps {
  title: string
  icon: React.ReactNode
  children: React.ReactNode
  defaultOpen?: boolean
}

function Section({ title, icon, children, defaultOpen = true }: SectionProps) {
  const [isOpen, setIsOpen] = useState(defaultOpen)

  return (
    <div className="border-b border-white/10">
      <button
        onClick={() => setIsOpen(!isOpen)}
        className="w-full flex items-center gap-2 px-3 py-2 hover:bg-white/5 text-white"
      >
        {isOpen ? <ChevronDown className="w-4 h-4" /> : <ChevronRight className="w-4 h-4" />}
        {icon}
        <span className="text-sm font-medium">{title}</span>
      </button>
      {isOpen && (
        <div className="px-3 pb-3">
          {children}
        </div>
      )}
    </div>
  )
}

interface FieldProps {
  label: string
  children: React.ReactNode
}

function Field({ label, children }: FieldProps) {
  return (
    <div className="mb-3">
      <label className="block text-xs text-gray-400 mb-1">{label}</label>
      <div className="text-sm text-white break-all">{children}</div>
    </div>
  )
}

/** Format a date string for display */
function formatDate(dateString?: string): string {
  if (!dateString) return '-'
  try {
    const date = new Date(dateString)
    return date.toLocaleString()
  } catch {
    return dateString
  }
}

/** Get display name for node type */
function getNodeTypeDisplay(nodeType: string): { label: string; icon: React.ReactNode } {
  switch (nodeType) {
    case 'raisin:Function':
      return { label: 'Function', icon: <SquareFunction className="w-4 h-4 text-violet-400" /> }
    case 'raisin:Folder':
      return { label: 'Folder', icon: <Folder className="w-4 h-4 text-yellow-400" /> }
    case 'raisin:Asset':
      return { label: 'Asset', icon: <File className="w-4 h-4 text-gray-400" /> }
    default:
      return { label: nodeType, icon: <File className="w-4 h-4 text-gray-400" /> }
  }
}

export function PropertiesPanel() {
  const { selectedNode } = useFunctionsContext()

  if (!selectedNode) {
    return (
      <div className="h-full flex flex-col">
        <div className="p-3 border-b border-white/10">
          <h2 className="text-sm font-semibold text-white">Properties</h2>
        </div>
        <div className="flex-1 flex items-center justify-center text-gray-400 text-sm">
          Select a node to view properties
        </div>
      </div>
    )
  }

  const isFunction = selectedNode.node_type === 'raisin:Function'
  const props = selectedNode.properties || {}
  const triggers = isFunction ? ((props as FunctionNode['properties']).triggers || []) : []
  const nodeTypeInfo = getNodeTypeDisplay(selectedNode.node_type)

  return (
    <div className="h-full flex flex-col">
      {/* Header */}
      <div className="p-3 border-b border-white/10">
        <div className="flex items-center gap-2 mb-1">
          {nodeTypeInfo.icon}
          <h2 className="text-sm font-semibold text-white truncate" title={selectedNode.name}>
            {selectedNode.name}
          </h2>
        </div>
        <p className="text-xs text-gray-400 truncate">{selectedNode.path}</p>
      </div>

      {/* Sections */}
      <div className="flex-1 overflow-auto">
        {/* Node Info */}
        <Section title="Node Info" icon={<Info className="w-4 h-4 text-gray-400" />}>
          <Field label="Path">{selectedNode.path}</Field>
          <Field label="Type">
            <span className="flex items-center gap-2">
              {nodeTypeInfo.icon}
              {nodeTypeInfo.label}
            </span>
          </Field>
          <Field label="ID">
            <span className="font-mono text-xs">{selectedNode.id}</span>
          </Field>
          <Field label="Created">{formatDate(selectedNode.created_at)}</Field>
          <Field label="Updated">{formatDate(selectedNode.updated_at)}</Field>
        </Section>

        {/* Triggers (read-only, only for functions) */}
        {isFunction && triggers.length > 0 && (
          <Section title="Triggers" icon={<Zap className="w-4 h-4 text-yellow-400" />} defaultOpen={false}>
            <p className="text-xs text-gray-500 mb-2">
              Edit triggers in the function editor tab
            </p>
            <div className="space-y-2">
              {triggers.map((trigger: TriggerCondition, idx: number) => (
                <div
                  key={idx}
                  className="p-2 bg-white/5 border border-white/10 rounded text-sm"
                >
                  <div className="flex items-center justify-between">
                    <span className="text-white">{trigger.name}</span>
                    <span className={`text-xs ${trigger.enabled ? 'text-green-400' : 'text-red-400'}`}>
                      {trigger.enabled ? 'Active' : 'Inactive'}
                    </span>
                  </div>
                  <div className="text-xs text-gray-400 mt-1">
                    {trigger.trigger_type === 'node_event' && (
                      <span>Events: {trigger.event_kinds?.join(', ')}</span>
                    )}
                    {trigger.trigger_type === 'schedule' && (
                      <span>Cron: {trigger.cron_expression}</span>
                    )}
                    {trigger.trigger_type === 'http' && (
                      <span>HTTP endpoint</span>
                    )}
                  </div>
                </div>
              ))}
            </div>
          </Section>
        )}
      </div>
    </div>
  )
}
