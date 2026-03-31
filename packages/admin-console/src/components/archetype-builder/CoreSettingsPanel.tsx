import { Settings } from 'lucide-react'
import { cleanObject } from './utils'
import type { ArchetypeDefinition } from './types'
import NodeTypePicker from '../shared/NodeTypePicker'
import ArchetypePicker from '../shared/ArchetypePicker'

interface CoreSettingsPanelProps {
  archetype: ArchetypeDefinition
  onChange: (archetype: ArchetypeDefinition) => void
  validationErrors: Record<string, string>
}

export default function CoreSettingsPanel({
  archetype,
  onChange,
  validationErrors,
}: CoreSettingsPanelProps) {
  const updateArchetype = (updates: Partial<ArchetypeDefinition>) => {
    onChange(cleanObject({ ...archetype, ...updates }) as ArchetypeDefinition)
  }

  return (
    <div className="h-full flex flex-col bg-black/20 border-l border-white/10">
      {/* Header */}
      <div className="px-4 py-3 border-b border-white/10 bg-black/20">
        <div className="flex items-center gap-2">
          <div className="p-1.5 rounded bg-primary-500/20 text-primary-400">
            <Settings className="w-4 h-4" />
          </div>
          <h3 className="text-sm font-semibold text-white">Archetype Settings</h3>
        </div>
      </div>

      {/* Settings */}
      <div className="flex-1 overflow-y-auto p-4 space-y-4">
        {/* Name */}
        <div>
          <label className="block text-xs text-zinc-400 mb-1">Name *</label>
          <input
            type="text"
            value={archetype.name || ''}
            onChange={(e) => updateArchetype({ name: e.target.value })}
            className={`
              w-full px-3 py-2 bg-black/30 border rounded-lg text-white text-sm focus:outline-none focus:ring-2
              ${validationErrors.name ? 'border-red-500 focus:ring-red-400' : 'border-white/10 focus:ring-primary-400'}
            `}
            placeholder="namespace:ArchetypeName"
          />
          {validationErrors.name && (
            <p className="text-xs text-red-400 mt-1">{validationErrors.name}</p>
          )}
          <p className="text-[10px] text-zinc-500 mt-1">
            Format: namespace:ArchetypeName (e.g., marketing:HeroSection)
          </p>
        </div>

        {/* Title */}
        <div>
          <label className="block text-xs text-zinc-400 mb-1">Title</label>
          <input
            type="text"
            value={archetype.title || ''}
            onChange={(e) => updateArchetype({ title: e.target.value || undefined })}
            className="w-full px-3 py-2 bg-black/30 border border-white/10 rounded-lg text-white text-sm focus:outline-none focus:ring-2 focus:ring-primary-400"
            placeholder="Human-readable title"
          />
        </div>

        {/* Extends */}
        <div>
          <label className="block text-xs text-zinc-400 mb-1">Extends</label>
          <ArchetypePicker
            mode="single"
            value={archetype.extends || ''}
            onChange={(value) => updateArchetype({ extends: (value as string) || undefined })}
            allowNone
            noneLabel="None"
            excludeNames={archetype.name ? [archetype.name] : []}
            error={validationErrors.extends}
          />
          <p className="text-[10px] text-zinc-500 mt-1">Optional parent archetype to inherit from</p>
        </div>

        {/* Base Node Type */}
        <div>
          <label className="block text-xs text-zinc-400 mb-1">Base Node Type</label>
          <NodeTypePicker
            mode="single"
            value={archetype.base_node_type || ''}
            onChange={(value) => updateArchetype({ base_node_type: (value as string) || undefined })}
            allowNone
            noneLabel="None"
            placeholder="Select base node type..."
          />
          <p className="text-[10px] text-zinc-500 mt-1">
            Underlying node type for storage (e.g., raisin:Page)
          </p>
        </div>

        {/* Strict Mode */}
        <div className="flex items-center justify-between py-2 border-t border-white/10 mt-2">
          <div>
            <label className="block text-xs text-zinc-400">Strict Mode</label>
            <p className="text-[10px] text-zinc-500">Disallow undefined properties</p>
          </div>
          <input
            type="checkbox"
            checked={archetype.strict ?? false}
            onChange={(e) => updateArchetype({ strict: e.target.checked || undefined })}
            className="w-4 h-4 rounded border-white/20 bg-black/30 text-primary-500 focus:ring-primary-400"
          />
        </div>

        {/* Icon */}
        <div>
          <label className="block text-xs text-zinc-400 mb-1">Icon</label>
          <input
            type="text"
            value={archetype.icon || ''}
            onChange={(e) => updateArchetype({ icon: e.target.value || undefined })}
            className="w-full px-3 py-2 bg-black/30 border border-white/10 rounded-lg text-white text-sm focus:outline-none focus:ring-2 focus:ring-primary-400"
            placeholder="icon-name"
          />
        </div>

        {/* Description */}
        <div>
          <label className="block text-xs text-zinc-400 mb-1">Description</label>
          <textarea
            value={archetype.description || ''}
            onChange={(e) => updateArchetype({ description: e.target.value || undefined })}
            className="w-full px-3 py-2 bg-black/30 border border-white/10 rounded-lg text-white text-sm focus:outline-none focus:ring-2 focus:ring-primary-400 min-h-[80px]"
            placeholder="Describe this archetype's purpose and usage"
          />
        </div>

        {/* Validation Summary */}
        {Object.keys(validationErrors).length > 0 && (
          <div className="pt-4 border-t border-white/10">
            <h4 className="text-xs font-semibold text-red-400 mb-2">Validation Errors</h4>
            <div className="space-y-1">
              {Object.entries(validationErrors).map(([key, message]) => (
                <p key={key} className="text-xs text-red-300">
                  • {message}
                </p>
              ))}
            </div>
          </div>
        )}
      </div>
    </div>
  )
}
