/**
 * Field Editor Panel
 *
 * Tabbed editor for archetype/element field settings.
 * Uses icon tabs with tooltips for compact navigation.
 */

import { useState, useEffect, useRef } from 'react'
import {
  Trash2,
  Plus,
  X,
  Type,
  Eye,
  Shield,
  Sliders,
  Search,
  Check,
  ChevronDown,
  Loader2,
} from 'lucide-react'
import { FIELD_TYPE_ICONS, FIELD_TYPE_COLORS } from './constants'
import { cleanObject } from './utils'
import { elementTypesApi, type ElementType as ApiElementType } from '../../api/elementtypes'
import type { FieldSchema } from './types'

// Simplified element type info for dropdown selection
interface ElementTypeOption {
  name: string
  description?: string
}

interface FieldEditorPanelProps {
  field: FieldSchema
  onChange: (field: FieldSchema) => void
  onDelete: () => void
  /** Repository name for API calls */
  repo?: string
  /** Branch name for API calls */
  branch?: string
  /** Current element type name to exclude from selection (prevents self-reference) */
  currentElementTypeName?: string
}

type TabId = 'basic' | 'display' | 'validation' | 'config'

interface TabConfig {
  id: TabId
  icon: typeof Type
  tooltip: string
}

const TABS: TabConfig[] = [
  { id: 'basic', icon: Type, tooltip: 'Name & core settings' },
  { id: 'display', icon: Eye, tooltip: 'Labels & presentation' },
  { id: 'validation', icon: Shield, tooltip: 'Constraints & rules' },
  { id: 'config', icon: Sliders, tooltip: 'Type-specific options' },
]

export default function FieldEditorPanel({
  field,
  onChange,
  onDelete,
  repo,
  branch,
  currentElementTypeName,
}: FieldEditorPanelProps) {
  const [activeTab, setActiveTab] = useState<TabId>('basic')
  const [elementTypes, setElementTypes] = useState<ElementTypeOption[]>([])
  const [loadingElementTypes, setLoadingElementTypes] = useState(false)

  // Load element types from API for ElementField/SectionField/CompositeField
  useEffect(() => {
    if (!repo || !branch) return

    // Only load if this field type needs element types
    const needsElementTypes = ['ElementField', 'SectionField', 'CompositeField'].includes(field.$type)
    if (!needsElementTypes) return

    setLoadingElementTypes(true)
    elementTypesApi
      .list(repo, branch)
      .then((types: ApiElementType[]) => {
        // Filter out the current element type being edited to prevent self-reference
        const filtered = types
          .filter((t) => t.name !== currentElementTypeName)
          .map((t) => ({
            name: t.name,
            description: t.description,
          }))
        setElementTypes(filtered)
      })
      .catch((err) => {
        console.error('Failed to load element types:', err)
        setElementTypes([])
      })
      .finally(() => {
        setLoadingElementTypes(false)
      })
  }, [repo, branch, field.$type, currentElementTypeName])

  const Icon = FIELD_TYPE_ICONS[field.$type]
  const colorClass = FIELD_TYPE_COLORS[field.$type]

  const updateField = (updates: Partial<FieldSchema>) => {
    onChange(cleanObject({ ...field, ...updates }) as FieldSchema)
  }

  const updateConfig = (updates: any) => {
    const currentConfig = (field as any).config || {}
    updateField({ config: cleanObject({ ...currentConfig, ...updates }) } as any)
  }

  return (
    <div className="h-full flex flex-col bg-black/20 border-l border-white/10">
      {/* Header */}
      <div className="px-3 py-2 border-b border-white/10 bg-black/20">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2">
            <div className={`p-1.5 rounded ${colorClass}`}>
              <Icon className="w-4 h-4" />
            </div>
            <h3 className="text-sm font-semibold text-white">{field.$type}</h3>
          </div>
          <button
            onClick={onDelete}
            className="p-1.5 rounded text-zinc-400 hover:text-red-400 hover:bg-red-500/20 transition-colors"
          >
            <Trash2 className="w-4 h-4" />
          </button>
        </div>
      </div>

      {/* Icon Tab Bar */}
      <div className="flex items-center gap-1 px-2 py-1.5 border-b border-white/10 bg-black/30">
        {TABS.map((tab) => {
          const TabIcon = tab.icon
          const isActive = activeTab === tab.id
          return (
            <button
              key={tab.id}
              onClick={() => setActiveTab(tab.id)}
              className={`
                relative group p-2 rounded transition-colors
                ${isActive ? 'bg-primary-500/20 text-primary-400' : 'text-zinc-500 hover:text-zinc-300 hover:bg-white/5'}
              `}
              title={tab.tooltip}
            >
              <TabIcon className="w-4 h-4" />
              {/* Tooltip */}
              <div className="absolute bottom-full left-1/2 -translate-x-1/2 mb-2 px-2 py-1 bg-zinc-900 border border-white/20 rounded text-xs text-white whitespace-nowrap opacity-0 group-hover:opacity-100 transition-opacity pointer-events-none z-50">
                {tab.tooltip}
              </div>
            </button>
          )
        })}
      </div>

      {/* Tab Content */}
      <div className="flex-1 overflow-y-auto p-3 space-y-3">
        {activeTab === 'basic' && (
          <BasicTab field={field} updateField={updateField} />
        )}

        {activeTab === 'display' && (
          <DisplayTab field={field} updateField={updateField} />
        )}

        {activeTab === 'validation' && (
          <ValidationTab field={field} updateField={updateField} updateConfig={updateConfig} />
        )}

        {activeTab === 'config' && (
          <ConfigTab
            field={field}
            updateField={updateField}
            updateConfig={updateConfig}
            elementTypes={elementTypes}
            loadingElementTypes={loadingElementTypes}
          />
        )}
      </div>
    </div>
  )
}

// ============ Tab Components ============

interface TabProps {
  field: FieldSchema
  updateField: (updates: Partial<FieldSchema>) => void
}

function BasicTab({ field, updateField }: TabProps) {
  return (
    <>
      <div>
        <label className="block text-xs text-zinc-400 mb-1">Field Name *</label>
        <input
          type="text"
          value={field.name || ''}
          onChange={(e) => updateField({ name: e.target.value })}
          className="w-full px-3 py-2 bg-black/30 border border-white/10 rounded-lg text-white text-sm focus:outline-none focus:ring-2 focus:ring-primary-400"
          placeholder="field_name"
        />
        <p className="text-[10px] text-zinc-500 mt-1">Use lowercase with underscores</p>
      </div>

      <div className="space-y-2">
        <label className="flex items-center gap-2 cursor-pointer">
          <input
            type="checkbox"
            checked={field.required || false}
            onChange={(e) => updateField({ required: e.target.checked || undefined })}
            className="w-4 h-4 rounded border-white/30 bg-black/30 text-primary-400 focus:ring-primary-400"
          />
          <span className="text-sm text-zinc-300">Required field</span>
        </label>

        <label className="flex items-center gap-2 cursor-pointer">
          <input
            type="checkbox"
            checked={field.translatable || false}
            onChange={(e) => updateField({ translatable: e.target.checked || undefined })}
            className="w-4 h-4 rounded border-white/30 bg-black/30 text-primary-400 focus:ring-primary-400"
          />
          <span className="text-sm text-zinc-300">Translatable</span>
        </label>

        <label className="flex items-center gap-2 cursor-pointer">
          <input
            type="checkbox"
            checked={field.multiple || false}
            onChange={(e) => updateField({ multiple: e.target.checked || undefined })}
            className="w-4 h-4 rounded border-white/30 bg-black/30 text-primary-400 focus:ring-primary-400"
          />
          <span className="text-sm text-zinc-300">Multiple values</span>
        </label>

        <label className="flex items-center gap-2 cursor-pointer">
          <input
            type="checkbox"
            checked={field.is_hidden || false}
            onChange={(e) => updateField({ is_hidden: e.target.checked || undefined })}
            className="w-4 h-4 rounded border-white/30 bg-black/30 text-primary-400 focus:ring-primary-400"
          />
          <span className="text-sm text-zinc-300">Hidden field</span>
        </label>

        <label className="flex items-center gap-2 cursor-pointer">
          <input
            type="checkbox"
            checked={field.design_value || false}
            onChange={(e) => updateField({ design_value: e.target.checked || undefined })}
            className="w-4 h-4 rounded border-white/30 bg-black/30 text-primary-400 focus:ring-primary-400"
          />
          <span className="text-sm text-zinc-300">Design value</span>
        </label>
      </div>
    </>
  )
}

function DisplayTab({ field, updateField }: TabProps) {
  return (
    <>
      <div>
        <label className="block text-xs text-zinc-400 mb-1">Label</label>
        <input
          type="text"
          value={field.label || ''}
          onChange={(e) => updateField({ label: e.target.value || undefined })}
          className="w-full px-3 py-2 bg-black/30 border border-white/10 rounded-lg text-white text-sm focus:outline-none focus:ring-2 focus:ring-primary-400"
          placeholder="Human-readable label"
        />
      </div>

      <div>
        <label className="block text-xs text-zinc-400 mb-1">Title</label>
        <input
          type="text"
          value={field.title || ''}
          onChange={(e) => updateField({ title: e.target.value || undefined })}
          className="w-full px-3 py-2 bg-black/30 border border-white/10 rounded-lg text-white text-sm focus:outline-none focus:ring-2 focus:ring-primary-400"
          placeholder="Display title"
        />
      </div>

      <div>
        <label className="block text-xs text-zinc-400 mb-1">Description</label>
        <textarea
          value={field.description || ''}
          onChange={(e) => updateField({ description: e.target.value || undefined })}
          className="w-full px-3 py-2 bg-black/30 border border-white/10 rounded-lg text-white text-sm focus:outline-none focus:ring-2 focus:ring-primary-400 min-h-[60px]"
          placeholder="Field description"
        />
      </div>

      <div>
        <label className="block text-xs text-zinc-400 mb-1">Help Text</label>
        <input
          type="text"
          value={field.help_text || ''}
          onChange={(e) => updateField({ help_text: e.target.value || undefined })}
          className="w-full px-3 py-2 bg-black/30 border border-white/10 rounded-lg text-white text-sm focus:outline-none focus:ring-2 focus:ring-primary-400"
          placeholder="Help text for users"
        />
      </div>
    </>
  )
}

interface ValidationTabProps extends TabProps {
  updateConfig: (updates: any) => void
}

function ValidationTab({ field, updateConfig }: ValidationTabProps) {
  const config = (field as any).config || {}

  if (field.$type === 'TextField' || field.$type === 'RichTextField') {
    return (
      <>
        <div className="grid grid-cols-2 gap-2">
          <div>
            <label className="block text-xs text-zinc-400 mb-1">Min Length</label>
            <input
              type="number"
              value={config.min_length || ''}
              onChange={(e) => updateConfig({ min_length: e.target.value ? Number(e.target.value) : undefined })}
              className="w-full px-3 py-2 bg-black/30 border border-white/10 rounded-lg text-white text-sm focus:outline-none focus:ring-2 focus:ring-primary-400"
              placeholder="0"
            />
          </div>
          <div>
            <label className="block text-xs text-zinc-400 mb-1">Max Length</label>
            <input
              type="number"
              value={config.max_length || ''}
              onChange={(e) => updateConfig({ max_length: e.target.value ? Number(e.target.value) : undefined })}
              className="w-full px-3 py-2 bg-black/30 border border-white/10 rounded-lg text-white text-sm focus:outline-none focus:ring-2 focus:ring-primary-400"
              placeholder="∞"
            />
          </div>
        </div>

        {field.$type === 'TextField' && (
          <div>
            <label className="block text-xs text-zinc-400 mb-1">Pattern (regex)</label>
            <input
              type="text"
              value={config.pattern || ''}
              onChange={(e) => updateConfig({ pattern: e.target.value || undefined })}
              className="w-full px-3 py-2 bg-black/30 border border-white/10 rounded-lg text-white text-sm font-mono focus:outline-none focus:ring-2 focus:ring-primary-400"
              placeholder="^[a-z]+$"
            />
          </div>
        )}
      </>
    )
  }

  if (field.$type === 'NumberField') {
    return (
      <>
        <div className="grid grid-cols-2 gap-2">
          <div>
            <label className="block text-xs text-zinc-400 mb-1">Minimum</label>
            <input
              type="number"
              value={config.minimum ?? ''}
              onChange={(e) => updateConfig({ minimum: e.target.value ? Number(e.target.value) : undefined })}
              className="w-full px-3 py-2 bg-black/30 border border-white/10 rounded-lg text-white text-sm focus:outline-none focus:ring-2 focus:ring-primary-400"
              placeholder="-∞"
            />
          </div>
          <div>
            <label className="block text-xs text-zinc-400 mb-1">Maximum</label>
            <input
              type="number"
              value={config.maximum ?? ''}
              onChange={(e) => updateConfig({ maximum: e.target.value ? Number(e.target.value) : undefined })}
              className="w-full px-3 py-2 bg-black/30 border border-white/10 rounded-lg text-white text-sm focus:outline-none focus:ring-2 focus:ring-primary-400"
              placeholder="∞"
            />
          </div>
        </div>
        <div>
          <label className="block text-xs text-zinc-400 mb-1">Step</label>
          <input
            type="number"
            value={config.step ?? ''}
            onChange={(e) => updateConfig({ step: e.target.value ? Number(e.target.value) : undefined })}
            className="w-full px-3 py-2 bg-black/30 border border-white/10 rounded-lg text-white text-sm focus:outline-none focus:ring-2 focus:ring-primary-400"
            placeholder="1"
          />
        </div>
      </>
    )
  }

  return (
    <p className="text-sm text-zinc-500">
      No validation options for {field.$type}.
    </p>
  )
}

interface ConfigTabProps extends TabProps {
  updateConfig: (updates: any) => void
  elementTypes: ElementTypeOption[]
  loadingElementTypes: boolean
}

function ConfigTab({ field, updateField, updateConfig, elementTypes, loadingElementTypes }: ConfigTabProps) {
  const config = (field as any).config || {}

  // TextField
  if (field.$type === 'TextField') {
    return (
      <label className="flex items-center gap-2 cursor-pointer">
        <input
          type="checkbox"
          checked={config.multiline || false}
          onChange={(e) => updateConfig({ multiline: e.target.checked || undefined })}
          className="w-4 h-4 rounded border-white/30 bg-black/30 text-primary-400 focus:ring-primary-400"
        />
        <span className="text-sm text-zinc-300">Multiline</span>
      </label>
    )
  }

  // DateField
  if (field.$type === 'DateField') {
    return (
      <div>
        <label className="block text-xs text-zinc-400 mb-1">Format</label>
        <input
          type="text"
          value={config.format || ''}
          onChange={(e) => updateConfig({ format: e.target.value || undefined })}
          className="w-full px-3 py-2 bg-black/30 border border-white/10 rounded-lg text-white text-sm focus:outline-none focus:ring-2 focus:ring-primary-400"
          placeholder="YYYY-MM-DD"
        />
      </div>
    )
  }

  // MediaField
  if (field.$type === 'MediaField') {
    return (
      <>
        <div>
          <label className="block text-xs text-zinc-400 mb-1">Allowed Types</label>
          <input
            type="text"
            value={config.allowed_types?.join(', ') || ''}
            onChange={(e) => updateConfig({ allowed_types: e.target.value ? e.target.value.split(',').map(s => s.trim()) : undefined })}
            className="w-full px-3 py-2 bg-black/30 border border-white/10 rounded-lg text-white text-sm focus:outline-none focus:ring-2 focus:ring-primary-400"
            placeholder="image/jpeg, image/png"
          />
        </div>
        <div>
          <label className="block text-xs text-zinc-400 mb-1">Max Size (bytes)</label>
          <input
            type="number"
            value={config.max_size || ''}
            onChange={(e) => updateConfig({ max_size: e.target.value ? Number(e.target.value) : undefined })}
            className="w-full px-3 py-2 bg-black/30 border border-white/10 rounded-lg text-white text-sm focus:outline-none focus:ring-2 focus:ring-primary-400"
          />
        </div>
      </>
    )
  }

  // ReferenceField
  if (field.$type === 'ReferenceField') {
    return (
      <div>
        <label className="block text-xs text-zinc-400 mb-1">Reference Type</label>
        <input
          type="text"
          value={config.reference_type || ''}
          onChange={(e) => updateConfig({ reference_type: e.target.value || undefined })}
          className="w-full px-3 py-2 bg-black/30 border border-white/10 rounded-lg text-white text-sm focus:outline-none focus:ring-2 focus:ring-primary-400"
          placeholder="namespace:NodeType"
        />
      </div>
    )
  }

  // TagField
  if (field.$type === 'TagField') {
    return (
      <>
        <div>
          <label className="block text-xs text-zinc-400 mb-1">Allowed Tags</label>
          <input
            type="text"
            value={config.allowed_tags?.join(', ') || ''}
            onChange={(e) => updateConfig({ allowed_tags: e.target.value ? e.target.value.split(',').map(s => s.trim()) : undefined })}
            className="w-full px-3 py-2 bg-black/30 border border-white/10 rounded-lg text-white text-sm focus:outline-none focus:ring-2 focus:ring-primary-400"
            placeholder="tag1, tag2, tag3"
          />
        </div>
        <div>
          <label className="block text-xs text-zinc-400 mb-1">Max Tags</label>
          <input
            type="number"
            value={config.max_tags || ''}
            onChange={(e) => updateConfig({ max_tags: e.target.value ? Number(e.target.value) : undefined })}
            className="w-full px-3 py-2 bg-black/30 border border-white/10 rounded-lg text-white text-sm focus:outline-none focus:ring-2 focus:ring-primary-400"
          />
        </div>
      </>
    )
  }

  // OptionsField
  if (field.$type === 'OptionsField') {
    const options = config.options || []

    const addOption = () => {
      updateConfig({ options: [...options, { value: '', label: '' }] })
    }

    const updateOption = (index: number, updates: any) => {
      const newOptions = [...options]
      newOptions[index] = { ...newOptions[index], ...updates }
      updateConfig({ options: newOptions })
    }

    const removeOption = (index: number) => {
      updateConfig({ options: options.filter((_: any, i: number) => i !== index) })
    }

    return (
      <>
        <label className="flex items-center gap-2 cursor-pointer mb-3">
          <input
            type="checkbox"
            checked={config.multiple || false}
            onChange={(e) => updateConfig({ multiple: e.target.checked || undefined })}
            className="w-4 h-4 rounded border-white/30 bg-black/30 text-primary-400 focus:ring-primary-400"
          />
          <span className="text-sm text-zinc-300">Allow multiple selection</span>
        </label>

        <div className="space-y-2">
          <div className="flex items-center justify-between">
            <label className="text-xs text-zinc-400">Options</label>
            <button
              onClick={addOption}
              className="flex items-center gap-1 px-2 py-1 text-xs bg-primary-500/20 hover:bg-primary-500/30 text-primary-300 rounded transition-colors"
            >
              <Plus className="w-3 h-3" />
              Add
            </button>
          </div>
          <div className="space-y-2 max-h-40 overflow-y-auto">
            {options.map((option: any, index: number) => (
              <div key={index} className="flex gap-1">
                <input
                  type="text"
                  value={option.value}
                  onChange={(e) => updateOption(index, { value: e.target.value })}
                  className="flex-1 px-2 py-1.5 bg-black/30 border border-white/10 rounded text-white text-xs focus:outline-none focus:ring-1 focus:ring-primary-400"
                  placeholder="Value"
                />
                <input
                  type="text"
                  value={option.label || ''}
                  onChange={(e) => updateOption(index, { label: e.target.value || undefined })}
                  className="flex-1 px-2 py-1.5 bg-black/30 border border-white/10 rounded text-white text-xs focus:outline-none focus:ring-1 focus:ring-primary-400"
                  placeholder="Label"
                />
                <button
                  onClick={() => removeOption(index)}
                  className="p-1 text-red-400 hover:bg-red-500/20 rounded transition-colors"
                >
                  <X className="w-3 h-3" />
                </button>
              </div>
            ))}
          </div>
        </div>
      </>
    )
  }

  // ElementField
  if (field.$type === 'ElementField') {
    return (
      <div>
        <label className="block text-xs text-zinc-400 mb-1">Element Type *</label>
        {loadingElementTypes ? (
          <div className="flex items-center gap-2 px-3 py-2 bg-black/30 border border-white/10 rounded-lg text-zinc-400 text-sm">
            <Loader2 className="w-4 h-4 animate-spin" />
            Loading element types...
          </div>
        ) : elementTypes.length === 0 ? (
          <div className="px-3 py-2 bg-black/30 border border-white/10 rounded-lg text-zinc-500 text-sm">
            No element types found in repository
          </div>
        ) : (
          <SearchableSelect
            options={elementTypes.map((et) => ({ value: et.name, label: et.name, description: et.description }))}
            value={(field as any).element_type || ''}
            onChange={(value) => updateField({ element_type: value || undefined } as any)}
            placeholder="Select element type..."
          />
        )}
      </div>
    )
  }

  // SectionField / CompositeField
  if (field.$type === 'SectionField' || field.$type === 'CompositeField') {
    const allowedTypes = (field as any).allowed_element_types || []

    return (
      <>
        <div>
          <label className="block text-xs text-zinc-400 mb-1">Allowed Element Types</label>
          {loadingElementTypes ? (
            <div className="flex items-center gap-2 px-3 py-2 bg-black/30 border border-white/10 rounded-lg text-zinc-400 text-sm">
              <Loader2 className="w-4 h-4 animate-spin" />
              Loading element types...
            </div>
          ) : elementTypes.length === 0 ? (
            <div className="px-3 py-2 bg-black/30 border border-white/10 rounded-lg text-zinc-500 text-sm">
              No element types found in repository
            </div>
          ) : (
            <SearchableMultiSelect
              options={elementTypes.map((et) => ({ value: et.name, label: et.name, description: et.description }))}
              selected={allowedTypes}
              onChange={(types) => updateField({ allowed_element_types: types.length > 0 ? types : undefined } as any)}
              placeholder="Search and select types..."
            />
          )}
          <p className="text-[10px] text-zinc-500 mt-1">
            Leave empty to allow all types
          </p>
        </div>

        {field.$type === 'SectionField' && (
          <div>
            <label className="block text-xs text-zinc-400 mb-1">Render As</label>
            <input
              type="text"
              value={(field as any).render_as || ''}
              onChange={(e) => updateField({ render_as: e.target.value || undefined } as any)}
              className="w-full px-3 py-2 bg-black/30 border border-white/10 rounded-lg text-white text-sm focus:outline-none focus:ring-2 focus:ring-primary-400"
              placeholder="list, grid, carousel"
            />
          </div>
        )}
      </>
    )
  }

  // ListingField
  if (field.$type === 'ListingField') {
    return (
      <div>
        <label className="block text-xs text-zinc-400 mb-1">Listing Type</label>
        <input
          type="text"
          value={config.listing_type || ''}
          onChange={(e) => updateConfig({ listing_type: e.target.value || undefined })}
          className="w-full px-3 py-2 bg-black/30 border border-white/10 rounded-lg text-white text-sm focus:outline-none focus:ring-2 focus:ring-primary-400"
          placeholder="e.g., recent, featured"
        />
      </div>
    )
  }

  return (
    <p className="text-sm text-zinc-500">
      No configuration options for {field.$type}.
    </p>
  )
}

// ============ Searchable Select Components ============

interface SelectOption {
  value: string
  label: string
  description?: string
}

interface SearchableSelectProps {
  options: SelectOption[]
  value: string
  onChange: (value: string) => void
  placeholder?: string
}

function SearchableSelect({ options, value, onChange, placeholder }: SearchableSelectProps) {
  const [isOpen, setIsOpen] = useState(false)
  const [searchQuery, setSearchQuery] = useState('')
  const containerRef = useRef<HTMLDivElement>(null)
  const inputRef = useRef<HTMLInputElement>(null)

  useEffect(() => {
    function handleClickOutside(event: MouseEvent) {
      if (containerRef.current && !containerRef.current.contains(event.target as Node)) {
        setIsOpen(false)
      }
    }
    document.addEventListener('mousedown', handleClickOutside)
    return () => document.removeEventListener('mousedown', handleClickOutside)
  }, [])

  useEffect(() => {
    if (isOpen && inputRef.current) {
      inputRef.current.focus()
    }
  }, [isOpen])

  const filteredOptions = options.filter(
    (option) =>
      option.label.toLowerCase().includes(searchQuery.toLowerCase()) ||
      option.value.toLowerCase().includes(searchQuery.toLowerCase())
  )

  const selectedOption = options.find((o) => o.value === value)

  const handleClear = (e: React.MouseEvent) => {
    e.stopPropagation()
    onChange('')
    setSearchQuery('')
  }

  return (
    <div ref={containerRef} className="relative">
      <button
        type="button"
        onClick={() => setIsOpen(!isOpen)}
        className={`
          w-full px-3 py-2 bg-black/30 border border-white/10 rounded-lg text-left
          flex items-center justify-between
          ${isOpen ? 'ring-2 ring-primary-400' : ''}
        `}
      >
        <span className={`text-sm ${selectedOption ? 'text-white' : 'text-zinc-500'}`}>
          {selectedOption?.label || placeholder}
        </span>
        <div className="flex items-center gap-1">
          {selectedOption && (
            <span
              role="button"
              onClick={handleClear}
              className="p-0.5 hover:bg-white/10 rounded transition-colors"
              title="Clear selection"
            >
              <X className="w-3.5 h-3.5 text-zinc-400 hover:text-white" />
            </span>
          )}
          <ChevronDown className={`w-4 h-4 text-zinc-400 transition-transform ${isOpen ? 'rotate-180' : ''}`} />
        </div>
      </button>

      {isOpen && (
        <div className="absolute z-50 mt-1 w-full bg-zinc-800 border border-white/20 rounded-lg shadow-xl overflow-hidden">
          <div className="p-2 border-b border-white/10">
            <div className="relative">
              <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 w-4 h-4 text-zinc-400" />
              <input
                ref={inputRef}
                type="text"
                value={searchQuery}
                onChange={(e) => setSearchQuery(e.target.value)}
                placeholder="Search..."
                className="w-full pl-8 pr-3 py-1.5 bg-white/5 border border-white/10 rounded text-sm text-white placeholder-zinc-500 focus:outline-none focus:border-primary-400"
              />
            </div>
          </div>

          <div className="max-h-48 overflow-y-auto">
            {filteredOptions.length === 0 ? (
              <div className="px-3 py-4 text-sm text-zinc-500 text-center">No options found</div>
            ) : (
              filteredOptions.map((option) => (
                <button
                  key={option.value}
                  onClick={() => {
                    onChange(option.value)
                    setIsOpen(false)
                    setSearchQuery('')
                  }}
                  className={`
                    w-full px-3 py-2 text-left transition-colors
                    ${option.value === value ? 'bg-primary-500/20' : 'hover:bg-white/5'}
                  `}
                >
                  <div className="text-sm text-white">{option.label}</div>
                  {option.description && (
                    <div className="text-xs text-zinc-500">{option.description}</div>
                  )}
                </button>
              ))
            )}
          </div>
        </div>
      )}
    </div>
  )
}

interface SearchableMultiSelectProps {
  options: SelectOption[]
  selected: string[]
  onChange: (selected: string[]) => void
  placeholder?: string
}

function SearchableMultiSelect({ options, selected, onChange, placeholder }: SearchableMultiSelectProps) {
  const [isOpen, setIsOpen] = useState(false)
  const [searchQuery, setSearchQuery] = useState('')
  const containerRef = useRef<HTMLDivElement>(null)
  const inputRef = useRef<HTMLInputElement>(null)

  useEffect(() => {
    function handleClickOutside(event: MouseEvent) {
      if (containerRef.current && !containerRef.current.contains(event.target as Node)) {
        setIsOpen(false)
      }
    }
    document.addEventListener('mousedown', handleClickOutside)
    return () => document.removeEventListener('mousedown', handleClickOutside)
  }, [])

  useEffect(() => {
    if (isOpen && inputRef.current) {
      inputRef.current.focus()
    }
  }, [isOpen])

  const filteredOptions = options.filter(
    (option) =>
      option.label.toLowerCase().includes(searchQuery.toLowerCase()) ||
      option.value.toLowerCase().includes(searchQuery.toLowerCase())
  )

  const toggleOption = (value: string) => {
    if (selected.includes(value)) {
      onChange(selected.filter((v) => v !== value))
    } else {
      onChange([...selected, value])
    }
  }

  const selectedOptions = options.filter((o) => selected.includes(o.value))

  return (
    <div ref={containerRef} className="relative">
      {/* Selected tags display */}
      <div
        onClick={() => setIsOpen(!isOpen)}
        className={`
          min-h-[40px] px-3 py-2 bg-black/30 border border-white/10 rounded-lg
          flex flex-wrap gap-1.5 items-center cursor-pointer
          ${isOpen ? 'ring-2 ring-primary-400' : ''}
        `}
      >
        {selectedOptions.length === 0 ? (
          <span className="text-sm text-zinc-500">{placeholder}</span>
        ) : (
          selectedOptions.map((option) => (
            <span
              key={option.value}
              className="inline-flex items-center gap-1 px-2 py-0.5 bg-primary-500/20 text-primary-300 text-xs rounded-full"
            >
              {option.label}
              <button
                onClick={(e) => {
                  e.stopPropagation()
                  toggleOption(option.value)
                }}
                className="hover:bg-primary-500/30 rounded-full p-0.5"
              >
                <X className="w-3 h-3" />
              </button>
            </span>
          ))
        )}
        <ChevronDown
          className={`w-4 h-4 text-zinc-400 ml-auto transition-transform ${isOpen ? 'rotate-180' : ''}`}
        />
      </div>

      {/* Dropdown */}
      {isOpen && (
        <div className="absolute z-50 mt-1 w-full bg-zinc-800 border border-white/20 rounded-lg shadow-xl overflow-hidden">
          {/* Search input */}
          <div className="p-2 border-b border-white/10">
            <div className="relative">
              <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 w-4 h-4 text-zinc-400" />
              <input
                ref={inputRef}
                type="text"
                value={searchQuery}
                onChange={(e) => setSearchQuery(e.target.value)}
                placeholder="Search..."
                className="w-full pl-8 pr-3 py-1.5 bg-white/5 border border-white/10 rounded text-sm text-white placeholder-zinc-500 focus:outline-none focus:border-primary-400"
              />
            </div>
          </div>

          {/* Options list */}
          <div className="max-h-48 overflow-y-auto">
            {filteredOptions.length === 0 ? (
              <div className="px-3 py-4 text-sm text-zinc-500 text-center">No options found</div>
            ) : (
              filteredOptions.map((option) => {
                const isSelected = selected.includes(option.value)
                return (
                  <button
                    key={option.value}
                    onClick={() => toggleOption(option.value)}
                    className={`
                      w-full px-3 py-2 text-left flex items-start gap-2 transition-colors
                      ${isSelected ? 'bg-primary-500/20' : 'hover:bg-white/5'}
                    `}
                  >
                    <div
                      className={`
                        mt-0.5 w-4 h-4 rounded border flex items-center justify-center flex-shrink-0
                        ${isSelected ? 'bg-primary-500 border-primary-500' : 'border-white/30'}
                      `}
                    >
                      {isSelected && <Check className="w-3 h-3 text-white" />}
                    </div>
                    <div className="flex-1 min-w-0">
                      <div className="text-sm text-white truncate">{option.label}</div>
                      {option.description && (
                        <div className="text-xs text-zinc-500 truncate">{option.description}</div>
                      )}
                    </div>
                  </button>
                )
              })
            )}
          </div>
        </div>
      )}
    </div>
  )
}
