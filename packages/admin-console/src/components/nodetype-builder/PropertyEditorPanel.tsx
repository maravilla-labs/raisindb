/**
 * Property Editor Panel
 *
 * Tabbed editor for property settings, organized by concern.
 * Uses icon tabs with tooltips for compact navigation.
 */

import { useState } from 'react'
import {
  Settings,
  Trash2,
  Plus,
  ChevronRight,
  Type,
  Eye,
  Shield,
  Search as SearchIcon,
  Sliders,
  X,
} from 'lucide-react'
import { PROPERTY_TYPE_ICONS, PROPERTY_TYPE_LABELS } from './constants'
import type { PropertyValueSchema, PropertyType, IndexType } from './types'
import { pathSegments } from './types'

interface PropertyEditorPanelProps {
  property: PropertyValueSchema
  onChange: (property: PropertyValueSchema) => void
  onDelete: () => void
  path?: string
}

type TabId = 'basic' | 'display' | 'validation' | 'indexing' | 'advanced'

interface TabConfig {
  id: TabId
  icon: typeof Settings
  label: string
  tooltip: string
}

const TABS: TabConfig[] = [
  { id: 'basic', icon: Type, label: 'Basic', tooltip: 'Name & core settings' },
  { id: 'display', icon: Eye, label: 'Display', tooltip: 'Labels & presentation' },
  { id: 'validation', icon: Shield, label: 'Validation', tooltip: 'Constraints & rules' },
  { id: 'indexing', icon: SearchIcon, label: 'Indexing', tooltip: 'Search indexes' },
  { id: 'advanced', icon: Sliders, label: 'Advanced', tooltip: 'Type-specific options' },
]

export default function PropertyEditorPanel({
  property,
  onChange,
  onDelete,
  path,
}: PropertyEditorPanelProps) {
  const [activeTab, setActiveTab] = useState<TabId>('basic')
  const Icon = PROPERTY_TYPE_ICONS[property.type]
  const segments = path ? pathSegments(path) : []
  const isNested = segments.length > 1

  const updateProperty = (updates: Partial<PropertyValueSchema>) => {
    const next: PropertyValueSchema = {
      ...property,
      ...updates,
    }

    if ('translatable' in updates && 'is_translatable' in next) {
      delete (next as any).is_translatable
    }

    if ('enum' in updates) {
      if (next.enum && Array.isArray(next.enum) && next.enum.length === 0) {
        delete (next as any).enum
      }
      if ('options' in next) delete (next as any).options
      if ('values' in next) delete (next as any).values
    }

    if ('constraints' in updates) {
      if (!next.constraints) {
        delete (next as any).constraints
      } else {
        Object.keys(next.constraints).forEach((key) => {
          if (next.constraints && next.constraints[key] === undefined) {
            delete next.constraints[key]
          }
        })
        if (next.constraints && Object.keys(next.constraints).length === 0) {
          delete (next as any).constraints
        }
      }
    }

    onChange(next)
  }

  const updatePropertyIndex = (enabled: boolean, indexType: IndexType) => {
    const current = property.index || []
    const updated = enabled ? [...current, indexType] : current.filter((t) => t !== indexType)
    updateProperty({ index: updated.length > 0 ? updated : undefined })
  }

  const rawEnumValues = property.enum ?? property.options ?? property.values
  const enumValues = Array.isArray(rawEnumValues)
    ? rawEnumValues.map((entry: any) => {
        if (typeof entry === 'string') return { value: entry, label: '' }
        if (entry && typeof entry === 'object') {
          return { value: String(entry.value ?? ''), label: entry.label ? String(entry.label) : '' }
        }
        return { value: '', label: '' }
      })
    : []

  const setEnumValues = (values: Array<{ value: string; label?: string }>) => {
    const sanitized = values
      .map((item) => ({ value: item.value.trim(), label: item.label?.trim() ?? '' }))
      .filter((item) => item.value.length > 0)

    if (sanitized.length === 0) {
      updateProperty({ enum: undefined })
      return
    }

    const serialized = sanitized.map((item) =>
      item.label.length > 0 ? { value: item.value, label: item.label } : item.value
    )
    updateProperty({ enum: serialized })
  }

  // Check which tabs should be visible based on property type
  const visibleTabs = TABS.filter((tab) => {
    if (tab.id === 'advanced') {
      // Only show advanced for types with type-specific options
      return ['String', 'Number', 'Array', 'Object', 'Reference', 'Element', 'Composite'].includes(
        property.type
      )
    }
    return true
  })

  return (
    <div className="h-full flex flex-col bg-zinc-900/50 border-l border-white/10 backdrop-blur-sm">
      {/* Header */}
      <div className="px-3 py-2 border-b border-white/10">
        <div className="flex items-center justify-between mb-1">
          <div className="flex items-center gap-2">
            <Settings className="w-4 h-4 text-primary-400" />
            <h3 className="text-sm font-semibold text-white">Property Settings</h3>
          </div>
          <button
            onClick={onDelete}
            className="p-1.5 hover:bg-red-500/20 text-red-400 rounded transition-colors"
            title="Delete property"
          >
            <Trash2 className="w-3.5 h-3.5" />
          </button>
        </div>
        {isNested && (
          <div className="flex items-center gap-1 text-[11px] text-zinc-500 mb-1 flex-wrap">
            {segments.map((segment, idx) => (
              <span key={idx} className="flex items-center gap-1">
                {idx > 0 && <ChevronRight className="w-3 h-3 text-zinc-600" />}
                <span className={idx === segments.length - 1 ? 'text-primary-400 font-medium' : ''}>
                  {segment}
                </span>
              </span>
            ))}
          </div>
        )}
        <div className="flex items-center gap-1.5 text-[11px] text-zinc-400">
          <Icon className="w-3.5 h-3.5" />
          <span>{PROPERTY_TYPE_LABELS[property.type]}</span>
        </div>
      </div>

      {/* Icon Tab Bar */}
      <div className="flex items-center gap-1 px-2 py-1.5 border-b border-white/10 bg-black/20">
        {visibleTabs.map((tab) => {
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
          <BasicTab
            property={property}
            updateProperty={updateProperty}
          />
        )}

        {activeTab === 'display' && (
          <DisplayTab
            property={property}
            updateProperty={updateProperty}
          />
        )}

        {activeTab === 'validation' && (
          <ValidationTab
            property={property}
            updateProperty={updateProperty}
            enumValues={enumValues}
            setEnumValues={setEnumValues}
          />
        )}

        {activeTab === 'indexing' && (
          <IndexingTab
            property={property}
            updatePropertyIndex={updatePropertyIndex}
          />
        )}

        {activeTab === 'advanced' && (
          <AdvancedTab
            property={property}
            updateProperty={updateProperty}
          />
        )}
      </div>
    </div>
  )
}

// ============ Tab Components ============

interface TabProps {
  property: PropertyValueSchema
  updateProperty: (updates: Partial<PropertyValueSchema>) => void
}

function BasicTab({ property, updateProperty }: TabProps) {
  return (
    <>
      {/* Property Name */}
      <div>
        <label className="block text-xs font-medium text-zinc-300 mb-1">
          Name <span className="text-red-400">*</span>
        </label>
        <input
          type="text"
          value={property.name || ''}
          onChange={(e) => updateProperty({ name: e.target.value })}
          className="w-full px-3 py-2 bg-white/5 border border-white/20 rounded-lg text-sm text-white focus:outline-none focus:ring-2 focus:ring-primary-500/50"
          placeholder="property_name"
        />
        <p className="text-xs text-zinc-500 mt-1">
          Use lowercase with underscores
        </p>
      </div>

      {/* Required */}
      <label className="flex items-center gap-2 cursor-pointer">
        <input
          type="checkbox"
          checked={property.required || false}
          onChange={(e) => updateProperty({ required: e.target.checked })}
          className="w-4 h-4 rounded border-white/20 bg-white/5 text-primary-500 focus:ring-2 focus:ring-primary-500/50"
        />
        <span className="text-sm text-zinc-300">Required field</span>
      </label>

      {/* Unique */}
      <label className="flex items-center gap-2 cursor-pointer">
        <input
          type="checkbox"
          checked={property.unique || false}
          onChange={(e) => updateProperty({ unique: e.target.checked })}
          className="w-4 h-4 rounded border-white/20 bg-white/5 text-primary-500 focus:ring-2 focus:ring-primary-500/50"
        />
        <span className="text-sm text-zinc-300">Unique value</span>
      </label>

      {/* Translatable */}
      {['String', 'Object', 'Element', 'Composite'].includes(property.type) && (
        <label className="flex items-center gap-2 cursor-pointer">
          <input
            type="checkbox"
            checked={Boolean(property.translatable ?? property.is_translatable)}
            onChange={(e) => updateProperty({ translatable: e.target.checked })}
            className="w-4 h-4 rounded border-white/20 bg-white/5 text-primary-500 focus:ring-2 focus:ring-primary-500/50"
          />
          <span className="text-sm text-zinc-300">Translatable</span>
        </label>
      )}

      {/* Default Value */}
      <div>
        <label className="block text-xs font-medium text-zinc-300 mb-1">
          Default Value
        </label>
        {renderDefaultValueInput(property, updateProperty)}
      </div>
    </>
  )
}

function DisplayTab({ property, updateProperty }: TabProps) {
  return (
    <>
      <div>
        <label className="block text-xs font-medium text-zinc-300 mb-1">
          Display Label
        </label>
        <input
          type="text"
          value={property.label || ''}
          onChange={(e) => updateProperty({ label: e.target.value || undefined })}
          className="w-full px-3 py-2 bg-white/5 border border-white/20 rounded-lg text-sm text-white focus:outline-none focus:ring-2 focus:ring-primary-500/50"
          placeholder="Readable label shown in forms"
        />
      </div>

      <div>
        <label className="block text-xs font-medium text-zinc-300 mb-1">
          Placeholder
        </label>
        <input
          type="text"
          value={property.placeholder || ''}
          onChange={(e) => updateProperty({ placeholder: e.target.value || undefined })}
          className="w-full px-3 py-2 bg-white/5 border border-white/20 rounded-lg text-sm text-white focus:outline-none focus:ring-2 focus:ring-primary-500/50"
          placeholder="Shown when no value provided"
        />
      </div>

      <div>
        <label className="block text-xs font-medium text-zinc-300 mb-1">
          Description / Help Text
        </label>
        <textarea
          value={property.description || ''}
          onChange={(e) => updateProperty({ description: e.target.value || undefined })}
          rows={3}
          className="w-full px-3 py-2 bg-white/5 border border-white/20 rounded-lg text-sm text-white focus:outline-none focus:ring-2 focus:ring-primary-500/50"
          placeholder="Extra guidance for authors"
        />
      </div>

      {property.type === 'String' && (
        <label className="flex items-center gap-2 cursor-pointer">
          <input
            type="checkbox"
            checked={Boolean(property.multiline)}
            onChange={(e) => updateProperty({ multiline: e.target.checked || undefined })}
            className="w-4 h-4 rounded border-white/20 bg-white/5 text-primary-500 focus:ring-2 focus:ring-primary-500/50"
          />
          <span className="text-sm text-zinc-300">Multiline input</span>
        </label>
      )}
    </>
  )
}

interface ValidationTabProps extends TabProps {
  enumValues: Array<{ value: string; label: string }>
  setEnumValues: (values: Array<{ value: string; label?: string }>) => void
}

function ValidationTab({ property, updateProperty, enumValues, setEnumValues }: ValidationTabProps) {
  return (
    <>
      {/* String constraints */}
      {property.type === 'String' && (
        <>
          <div className="grid grid-cols-2 gap-2">
            <div>
              <label className="block text-xs font-medium text-zinc-300 mb-1">
                Min Length
              </label>
              <input
                type="number"
                value={property.constraints?.minLength || ''}
                onChange={(e) =>
                  updateProperty({
                    constraints: {
                      ...property.constraints,
                      minLength: e.target.value ? parseInt(e.target.value) : undefined,
                    },
                  })
                }
                className="w-full px-3 py-2 bg-white/5 border border-white/20 rounded-lg text-sm text-white focus:outline-none focus:ring-2 focus:ring-primary-500/50"
                placeholder="0"
              />
            </div>
            <div>
              <label className="block text-xs font-medium text-zinc-300 mb-1">
                Max Length
              </label>
              <input
                type="number"
                value={property.constraints?.maxLength || ''}
                onChange={(e) =>
                  updateProperty({
                    constraints: {
                      ...property.constraints,
                      maxLength: e.target.value ? parseInt(e.target.value) : undefined,
                    },
                  })
                }
                className="w-full px-3 py-2 bg-white/5 border border-white/20 rounded-lg text-sm text-white focus:outline-none focus:ring-2 focus:ring-primary-500/50"
                placeholder="∞"
              />
            </div>
          </div>

          <div>
            <label className="block text-xs font-medium text-zinc-300 mb-1">
              Pattern (Regex)
            </label>
            <input
              type="text"
              value={property.constraints?.pattern || ''}
              onChange={(e) =>
                updateProperty({
                  constraints: {
                    ...property.constraints,
                    pattern: e.target.value || undefined,
                  },
                })
              }
              className="w-full px-3 py-2 bg-white/5 border border-white/20 rounded-lg text-sm text-white font-mono focus:outline-none focus:ring-2 focus:ring-primary-500/50"
              placeholder="^[a-z]+$"
            />
          </div>

          {/* Enum Values */}
          <div className="p-3 bg-white/5 border border-white/10 rounded-lg space-y-2">
            <div className="flex items-center justify-between">
              <label className="text-xs font-medium text-zinc-300">
                Allowed Values (Enum)
              </label>
              <button
                type="button"
                onClick={() => setEnumValues([...enumValues, { value: '', label: '' }])}
                className="flex items-center gap-1 px-2 py-1 text-xs bg-primary-500/20 hover:bg-primary-500/30 text-primary-300 rounded transition-colors"
              >
                <Plus className="w-3 h-3" />
                Add
              </button>
            </div>

            {enumValues.length === 0 ? (
              <p className="text-xs text-zinc-500">
                No restrictions. Add options to limit to specific choices.
              </p>
            ) : (
              <div className="space-y-2 max-h-40 overflow-y-auto">
                {enumValues.map((option, index) => (
                  <div key={`enum-${index}`} className="flex items-center gap-1">
                    <input
                      type="text"
                      value={option.value}
                      onChange={(e) => {
                        const next = enumValues.map((entry, idx) =>
                          idx === index ? { ...entry, value: e.target.value } : entry
                        )
                        setEnumValues(next)
                      }}
                      className="flex-1 px-2 py-1.5 bg-white/5 border border-white/20 rounded text-sm text-white focus:outline-none focus:ring-2 focus:ring-primary-500/50"
                      placeholder="Value"
                    />
                    <input
                      type="text"
                      value={option.label ?? ''}
                      onChange={(e) => {
                        const next = enumValues.map((entry, idx) =>
                          idx === index ? { ...entry, label: e.target.value } : entry
                        )
                        setEnumValues(next)
                      }}
                      className="flex-1 px-2 py-1.5 bg-white/5 border border-white/20 rounded text-sm text-white focus:outline-none focus:ring-2 focus:ring-primary-500/50"
                      placeholder="Label"
                    />
                    <button
                      type="button"
                      onClick={() => setEnumValues(enumValues.filter((_, idx) => idx !== index))}
                      className="p-1 hover:bg-red-500/20 text-red-400 rounded transition-colors"
                    >
                      <Trash2 className="w-3.5 h-3.5" />
                    </button>
                  </div>
                ))}
              </div>
            )}
          </div>
        </>
      )}

      {/* Number constraints */}
      {property.type === 'Number' && (
        <div className="grid grid-cols-2 gap-2">
          <div>
            <label className="block text-xs font-medium text-zinc-300 mb-1">
              Minimum
            </label>
            <input
              type="number"
              value={property.constraints?.minimum ?? ''}
              onChange={(e) =>
                updateProperty({
                  constraints: {
                    ...property.constraints,
                    minimum: e.target.value ? parseFloat(e.target.value) : undefined,
                  },
                })
              }
              className="w-full px-3 py-2 bg-white/5 border border-white/20 rounded-lg text-sm text-white focus:outline-none focus:ring-2 focus:ring-primary-500/50"
              placeholder="-∞"
            />
          </div>
          <div>
            <label className="block text-xs font-medium text-zinc-300 mb-1">
              Maximum
            </label>
            <input
              type="number"
              value={property.constraints?.maximum ?? ''}
              onChange={(e) =>
                updateProperty({
                  constraints: {
                    ...property.constraints,
                    maximum: e.target.value ? parseFloat(e.target.value) : undefined,
                  },
                })
              }
              className="w-full px-3 py-2 bg-white/5 border border-white/20 rounded-lg text-sm text-white focus:outline-none focus:ring-2 focus:ring-primary-500/50"
              placeholder="∞"
            />
          </div>
        </div>
      )}

      {/* Array constraints */}
      {property.type === 'Array' && (
        <>
          <div className="grid grid-cols-2 gap-2">
            <div>
              <label className="block text-xs font-medium text-zinc-300 mb-1">
                Min Items
              </label>
              <input
                type="number"
                value={property.constraints?.minItems ?? ''}
                onChange={(e) =>
                  updateProperty({
                    constraints: {
                      ...property.constraints,
                      minItems: e.target.value ? parseInt(e.target.value) : undefined,
                    },
                  })
                }
                className="w-full px-3 py-2 bg-white/5 border border-white/20 rounded-lg text-sm text-white focus:outline-none focus:ring-2 focus:ring-primary-500/50"
                placeholder="0"
              />
            </div>
            <div>
              <label className="block text-xs font-medium text-zinc-300 mb-1">
                Max Items
              </label>
              <input
                type="number"
                value={property.constraints?.maxItems ?? ''}
                onChange={(e) =>
                  updateProperty({
                    constraints: {
                      ...property.constraints,
                      maxItems: e.target.value ? parseInt(e.target.value) : undefined,
                    },
                  })
                }
                className="w-full px-3 py-2 bg-white/5 border border-white/20 rounded-lg text-sm text-white focus:outline-none focus:ring-2 focus:ring-primary-500/50"
                placeholder="∞"
              />
            </div>
          </div>

          <label className="flex items-center gap-2 cursor-pointer">
            <input
              type="checkbox"
              checked={Boolean(property.constraints?.uniqueItems)}
              onChange={(e) =>
                updateProperty({
                  constraints: {
                    ...property.constraints,
                    uniqueItems: e.target.checked || undefined,
                  },
                })
              }
              className="w-4 h-4 rounded border-white/20 bg-white/5 text-primary-500 focus:ring-2 focus:ring-primary-500/50"
            />
            <span className="text-sm text-zinc-300">Enforce unique items</span>
          </label>
        </>
      )}

      {/* Show message for types without validation */}
      {!['String', 'Number', 'Array'].includes(property.type) && (
        <p className="text-sm text-zinc-500">
          No validation constraints available for {PROPERTY_TYPE_LABELS[property.type]} type.
        </p>
      )}
    </>
  )
}

interface IndexingTabProps {
  property: PropertyValueSchema
  updatePropertyIndex: (enabled: boolean, indexType: IndexType) => void
}

function IndexingTab({ property, updatePropertyIndex }: IndexingTabProps) {
  return (
    <>
      <p className="text-xs text-zinc-400 mb-3">
        Configure how this property is indexed for search and queries.
      </p>

      <label className="flex items-start gap-2 cursor-pointer p-2 rounded hover:bg-white/5">
        <input
          type="checkbox"
          checked={property.index?.includes('Fulltext') || false}
          onChange={(e) => updatePropertyIndex(e.target.checked, 'Fulltext')}
          className="w-4 h-4 rounded border-white/20 bg-white/5 text-primary-500 focus:ring-2 focus:ring-primary-500/50 mt-0.5"
        />
        <div className="flex-1">
          <span className="text-sm text-zinc-300 block">Fulltext</span>
          <p className="text-xs text-zinc-500 mt-0.5">
            Include in full-text search index
          </p>
        </div>
      </label>

      <label className="flex items-start gap-2 cursor-pointer p-2 rounded hover:bg-white/5">
        <input
          type="checkbox"
          checked={property.index?.includes('Vector') || false}
          onChange={(e) => updatePropertyIndex(e.target.checked, 'Vector')}
          className="w-4 h-4 rounded border-white/20 bg-white/5 text-primary-500 focus:ring-2 focus:ring-primary-500/50 mt-0.5"
        />
        <div className="flex-1">
          <span className="text-sm text-zinc-300 block">Vector</span>
          <p className="text-xs text-zinc-500 mt-0.5">
            Include in AI semantic search embeddings
          </p>
        </div>
      </label>

      <label className="flex items-start gap-2 cursor-pointer p-2 rounded hover:bg-white/5">
        <input
          type="checkbox"
          checked={property.index?.includes('Property') || false}
          onChange={(e) => updatePropertyIndex(e.target.checked, 'Property')}
          className="w-4 h-4 rounded border-white/20 bg-white/5 text-primary-500 focus:ring-2 focus:ring-primary-500/50 mt-0.5"
        />
        <div className="flex-1">
          <span className="text-sm text-zinc-300 block">Property</span>
          <p className="text-xs text-zinc-500 mt-0.5">
            Enable fast exact-match lookups
          </p>
        </div>
      </label>

      {property.index && property.index.length === 0 && (
        <p className="text-xs text-amber-400 mt-2 flex items-start gap-1 p-2 bg-amber-500/10 rounded">
          <span>⚠️</span>
          <span>This property will not be indexed. Select at least one index type to enable search.</span>
        </p>
      )}
    </>
  )
}

function AdvancedTab({ property, updateProperty }: TabProps) {
  return (
    <>
      {/* Array item type */}
      {property.type === 'Array' && (
        <div>
          <label className="block text-xs font-medium text-zinc-300 mb-1">
            Array Item Type
          </label>
          <select
            value={(!Array.isArray(property.items) && property.items?.type) || 'String'}
            onChange={(e) =>
              updateProperty({
                items: {
                  ...(Array.isArray(property.items) ? {} : property.items),
                  type: e.target.value as PropertyType,
                },
              })
            }
            className="w-full px-3 py-2 bg-white/5 border border-white/20 rounded-lg text-sm text-white focus:outline-none focus:ring-2 focus:ring-primary-500/50"
          >
            <option value="String">String</option>
            <option value="Number">Number</option>
            <option value="Boolean">Boolean</option>
            <option value="Object">Object</option>
            <option value="Reference">Reference</option>
            <option value="Element">Element</option>
          </select>
          <p className="text-xs text-zinc-500 mt-1">Type of items in this array</p>
        </div>
      )}

      {/* Object settings */}
      {property.type === 'Object' && (
        <label className="flex items-center gap-2 cursor-pointer">
          <input
            type="checkbox"
            checked={property.allow_additional_properties || false}
            onChange={(e) => updateProperty({ allow_additional_properties: e.target.checked })}
            className="w-4 h-4 rounded border-white/20 bg-white/5 text-primary-500 focus:ring-2 focus:ring-primary-500/50"
          />
          <span className="text-sm text-zinc-300">Allow additional properties</span>
        </label>
      )}

      {/* Reference type */}
      {property.type === 'Reference' && (
        <div>
          <label className="block text-xs font-medium text-zinc-300 mb-1">
            Reference Type
          </label>
          <input
            type="text"
            value={property.constraints?.referenceType || ''}
            onChange={(e) =>
              updateProperty({
                constraints: {
                  ...property.constraints,
                  referenceType: e.target.value || undefined,
                },
              })
            }
            className="w-full px-3 py-2 bg-white/5 border border-white/20 rounded-lg text-sm text-white focus:outline-none focus:ring-2 focus:ring-primary-500/50"
            placeholder="namespace:TypeName"
          />
          <p className="text-xs text-zinc-500 mt-1">Node type this reference points to</p>
        </div>
      )}

      {/* Element settings - allowed element types */}
      {property.type === 'Element' && (
        <div>
          <label className="block text-xs font-medium text-zinc-300 mb-1">
            Allowed Element Types
          </label>
          <AllowedTypesEditor
            values={property.constraints?.allowedElementTypes || []}
            onChange={(types) =>
              updateProperty({
                constraints: {
                  ...property.constraints,
                  allowedElementTypes: types.length > 0 ? types : undefined,
                },
              })
            }
            placeholder="Add element type..."
          />
          <p className="text-xs text-zinc-500 mt-1">
            Restrict which element types can be used. Leave empty to allow all.
          </p>
        </div>
      )}

      {/* Composite settings */}
      {property.type === 'Composite' && (
        <p className="text-sm text-zinc-400">
          Composite fields are configured by adding child properties in the canvas.
        </p>
      )}

      {/* String has no advanced options */}
      {property.type === 'String' && (
        <p className="text-sm text-zinc-500">
          No advanced options for String type. Use the Validation tab for constraints.
        </p>
      )}

      {property.type === 'Number' && (
        <p className="text-sm text-zinc-500">
          No advanced options for Number type. Use the Validation tab for constraints.
        </p>
      )}
    </>
  )
}

// ============ Helper Components ============

interface AllowedTypesEditorProps {
  values: string[]
  onChange: (values: string[]) => void
  placeholder?: string
}

function AllowedTypesEditor({ values, onChange, placeholder }: AllowedTypesEditorProps) {
  const [inputValue, setInputValue] = useState('')

  const addValue = () => {
    const trimmed = inputValue.trim()
    if (trimmed && !values.includes(trimmed)) {
      onChange([...values, trimmed])
      setInputValue('')
    }
  }

  const removeValue = (value: string) => {
    onChange(values.filter((v) => v !== value))
  }

  return (
    <div className="space-y-2">
      {/* Selected values */}
      {values.length > 0 && (
        <div className="flex flex-wrap gap-1.5">
          {values.map((value) => (
            <span
              key={value}
              className="inline-flex items-center gap-1 px-2 py-0.5 bg-primary-500/20 text-primary-300 text-xs rounded-full"
            >
              {value}
              <button
                onClick={() => removeValue(value)}
                className="hover:bg-primary-500/30 rounded-full p-0.5"
              >
                <X className="w-3 h-3" />
              </button>
            </span>
          ))}
        </div>
      )}

      {/* Input */}
      <div className="flex gap-2">
        <input
          type="text"
          value={inputValue}
          onChange={(e) => setInputValue(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') {
              e.preventDefault()
              addValue()
            }
          }}
          className="flex-1 px-3 py-2 bg-white/5 border border-white/20 rounded-lg text-sm text-white focus:outline-none focus:ring-2 focus:ring-primary-500/50"
          placeholder={placeholder}
        />
        <button
          type="button"
          onClick={addValue}
          disabled={!inputValue.trim()}
          className="px-3 py-2 bg-primary-500/20 hover:bg-primary-500/30 text-primary-300 rounded-lg transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
        >
          <Plus className="w-4 h-4" />
        </button>
      </div>
    </div>
  )
}

function renderDefaultValueInput(
  property: PropertyValueSchema,
  updateProperty: (updates: Partial<PropertyValueSchema>) => void
) {
  switch (property.type) {
    case 'String':
    case 'URL':
      return (
        <input
          type="text"
          value={property.default || ''}
          onChange={(e) => updateProperty({ default: e.target.value || undefined })}
          className="w-full px-3 py-2 bg-white/5 border border-white/20 rounded-lg text-sm text-white focus:outline-none focus:ring-2 focus:ring-primary-500/50"
          placeholder="Enter default value"
        />
      )

    case 'Number':
      return (
        <input
          type="number"
          value={property.default ?? ''}
          onChange={(e) =>
            updateProperty({ default: e.target.value ? parseFloat(e.target.value) : undefined })
          }
          className="w-full px-3 py-2 bg-white/5 border border-white/20 rounded-lg text-sm text-white focus:outline-none focus:ring-2 focus:ring-primary-500/50"
          placeholder="Enter default number"
        />
      )

    case 'Boolean':
      return (
        <select
          value={property.default === undefined ? '' : String(property.default)}
          onChange={(e) =>
            updateProperty({
              default: e.target.value === '' ? undefined : e.target.value === 'true',
            })
          }
          className="w-full px-3 py-2 bg-white/5 border border-white/20 rounded-lg text-sm text-white focus:outline-none focus:ring-2 focus:ring-primary-500/50"
        >
          <option value="">No default</option>
          <option value="true">True</option>
          <option value="false">False</option>
        </select>
      )

    case 'Date':
      return (
        <input
          type="datetime-local"
          value={property.default || ''}
          onChange={(e) => updateProperty({ default: e.target.value || undefined })}
          className="w-full px-3 py-2 bg-white/5 border border-white/20 rounded-lg text-sm text-white focus:outline-none focus:ring-2 focus:ring-primary-500/50"
        />
      )

    default:
      return (
        <textarea
          value={
            typeof property.default === 'string'
              ? property.default
              : property.default
              ? JSON.stringify(property.default, null, 2)
              : ''
          }
          onChange={(e) => {
            try {
              const parsed = e.target.value ? JSON.parse(e.target.value) : undefined
              updateProperty({ default: parsed })
            } catch {
              updateProperty({ default: e.target.value || undefined })
            }
          }}
          rows={2}
          className="w-full px-3 py-2 bg-white/5 border border-white/20 rounded-lg text-sm text-white font-mono focus:outline-none focus:ring-2 focus:ring-primary-500/50"
          placeholder="JSON value"
        />
      )
  }
}
