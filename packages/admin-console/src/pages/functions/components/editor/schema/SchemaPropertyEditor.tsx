/**
 * Schema Property Editor Component
 *
 * Individual property row editor for the visual JSON Schema builder.
 * Supports all PropertyDef fields from raisin-models.
 */

import { useState, useCallback } from 'react'
import {
  GripVertical,
  X,
  ChevronDown,
  ChevronRight,
  Plus,
  Type,
  Hash,
  ToggleLeft,
  Braces,
  List,
  Calendar,
  Link,
  ExternalLink,
  File,
  Layers,
  Box,
  FileType,
  Settings2,
} from 'lucide-react'
import {
  type SchemaProperty,
  type SchemaPropertyType,
  type IndexType,
  type DefaultValue,
  type PropertyConstraints,
  createEmptyProperty,
  VALID_INDEX_TYPES,
} from './schema-types'

interface SchemaPropertyEditorProps {
  /** The property being edited */
  property: SchemaProperty
  /** Called when the property changes */
  onChange: (property: SchemaProperty) => void
  /** Called when the property should be removed */
  onRemove: () => void
  /** Drag handle props from react-beautiful-dnd */
  dragHandleProps?: React.HTMLAttributes<HTMLDivElement> | null
  /** Nesting depth for indentation */
  depth?: number
  /** Whether the property is being dragged */
  isDragging?: boolean
}

const TYPE_OPTIONS: { value: SchemaPropertyType; label: string; icon: React.ReactNode }[] = [
  { value: 'string', label: 'String', icon: <Type className="w-3.5 h-3.5" /> },
  { value: 'number', label: 'Number', icon: <Hash className="w-3.5 h-3.5" /> },
  { value: 'boolean', label: 'Boolean', icon: <ToggleLeft className="w-3.5 h-3.5" /> },
  { value: 'date', label: 'Date', icon: <Calendar className="w-3.5 h-3.5" /> },
  { value: 'url', label: 'URL', icon: <Link className="w-3.5 h-3.5" /> },
  { value: 'reference', label: 'Reference', icon: <ExternalLink className="w-3.5 h-3.5" /> },
  { value: 'resource', label: 'Resource', icon: <File className="w-3.5 h-3.5" /> },
  { value: 'composite', label: 'Composite', icon: <Layers className="w-3.5 h-3.5" /> },
  { value: 'element', label: 'Element', icon: <Box className="w-3.5 h-3.5" /> },
  { value: 'nodetype', label: 'NodeType', icon: <FileType className="w-3.5 h-3.5" /> },
  { value: 'object', label: 'Object', icon: <Braces className="w-3.5 h-3.5" /> },
  { value: 'array', label: 'Array', icon: <List className="w-3.5 h-3.5" /> },
]

export function SchemaPropertyEditor({
  property,
  onChange,
  onRemove,
  dragHandleProps,
  depth = 0,
  isDragging = false,
}: SchemaPropertyEditorProps) {
  const [isExpanded, setIsExpanded] = useState(true)
  const [showDescription, setShowDescription] = useState(!!property.description)
  const [showAdvanced, setShowAdvanced] = useState(false)
  const [showConstraints, setShowConstraints] = useState(false)

  const inputClass = `px-2 py-1 bg-white/5 border border-white/10 rounded text-sm text-white
    placeholder-gray-500 focus:outline-none focus:ring-1 focus:ring-primary-500 focus:border-primary-500`

  const selectClass = `px-1.5 py-1 bg-white/5 border border-white/10 rounded text-sm text-white
    focus:outline-none focus:ring-1 focus:ring-primary-500 focus:border-primary-500`

  const handleNameChange = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      onChange({ ...property, name: e.target.value })
    },
    [property, onChange]
  )

  const handleTypeChange = useCallback(
    (e: React.ChangeEvent<HTMLSelectElement>) => {
      const newType = e.target.value as SchemaPropertyType
      const updated: SchemaProperty = { ...property, type: newType }

      // Initialize type-specific fields
      if (newType === 'object' && !updated.properties) {
        updated.properties = []
      }
      if (newType === 'array' && !updated.items) {
        updated.items = createEmptyProperty('string')
      }
      if (newType !== 'string') {
        delete updated.enum
      }
      if (newType !== 'object') {
        delete updated.properties
      }
      if (newType !== 'array') {
        delete updated.items
      }

      onChange(updated)
    },
    [property, onChange]
  )

  const handleDescriptionChange = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      onChange({ ...property, description: e.target.value })
    },
    [property, onChange]
  )

  const handleRequiredChange = useCallback(() => {
    onChange({ ...property, required: !property.required })
  }, [property, onChange])

  // Enum value handlers
  const handleAddEnumValue = useCallback(() => {
    const currentEnum = property.enum || []
    onChange({ ...property, enum: [...currentEnum, ''] })
  }, [property, onChange])

  const handleEnumValueChange = useCallback(
    (index: number, value: string) => {
      const currentEnum = [...(property.enum || [])]
      currentEnum[index] = value
      onChange({ ...property, enum: currentEnum })
    },
    [property, onChange]
  )

  const handleRemoveEnumValue = useCallback(
    (index: number) => {
      const currentEnum = [...(property.enum || [])]
      currentEnum.splice(index, 1)
      onChange({ ...property, enum: currentEnum.length > 0 ? currentEnum : undefined })
    },
    [property, onChange]
  )

  // Nested property handlers
  const handleAddNestedProperty = useCallback(() => {
    const currentProps = property.properties || []
    onChange({
      ...property,
      properties: [...currentProps, createEmptyProperty('string')],
    })
  }, [property, onChange])

  const handleNestedPropertyChange = useCallback(
    (index: number, updated: SchemaProperty) => {
      const currentProps = [...(property.properties || [])]
      currentProps[index] = updated
      onChange({ ...property, properties: currentProps })
    },
    [property, onChange]
  )

  const handleRemoveNestedProperty = useCallback(
    (index: number) => {
      const currentProps = [...(property.properties || [])]
      currentProps.splice(index, 1)
      onChange({ ...property, properties: currentProps })
    },
    [property, onChange]
  )

  // Array items handler
  const handleItemsChange = useCallback(
    (updated: SchemaProperty) => {
      onChange({ ...property, items: updated })
    },
    [property, onChange]
  )

  // New PropertyDef field handlers
  const handleUniqueChange = useCallback(() => {
    onChange({ ...property, unique: !property.unique })
  }, [property, onChange])

  const handleTranslatableChange = useCallback(() => {
    onChange({ ...property, translatable: !property.translatable })
  }, [property, onChange])

  const handleLabelChange = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      onChange({ ...property, label: e.target.value || undefined })
    },
    [property, onChange]
  )

  const handleOrderChange = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      const value = e.target.value
      onChange({ ...property, order: value ? parseInt(value, 10) : undefined })
    },
    [property, onChange]
  )

  const handleIndexToggle = useCallback(
    (indexType: IndexType) => {
      const currentIndex = property.index || []
      const newIndex = currentIndex.includes(indexType)
        ? currentIndex.filter((i) => i !== indexType)
        : [...currentIndex, indexType]
      onChange({ ...property, index: newIndex.length > 0 ? newIndex : undefined })
    },
    [property, onChange]
  )

  const handleConstraintChange = useCallback(
    (field: keyof PropertyConstraints, value: string | boolean | undefined) => {
      const constraints = { ...property.constraints }
      if (value === undefined || value === '') {
        delete constraints[field]
      } else if (typeof value === 'boolean') {
        constraints[field] = value as never
      } else if (field === 'pattern') {
        constraints.pattern = value
      } else {
        const num = parseFloat(value)
        if (!isNaN(num)) {
          constraints[field] = num as never
        }
      }
      onChange({
        ...property,
        constraints: Object.keys(constraints).length > 0 ? constraints : undefined,
      })
    },
    [property, onChange]
  )

  const handleDefaultChange = useCallback(
    (value: string) => {
      if (!value) {
        onChange({ ...property, default: undefined })
        return
      }
      // Set default based on type
      let defaultVal: DefaultValue
      if (property.type === 'boolean') {
        defaultVal = { Boolean: value === 'true' }
      } else if (property.type === 'number') {
        const num = parseFloat(value)
        defaultVal = isNaN(num) ? 'Null' : { Number: num }
      } else {
        defaultVal = { String: value }
      }
      onChange({ ...property, default: defaultVal })
    },
    [property, onChange]
  )

  const getDefaultValueDisplay = (): string => {
    if (!property.default) return ''
    if (property.default === 'Null') return ''
    if ('String' in property.default) return property.default.String
    if ('Number' in property.default) return String(property.default.Number)
    if ('Boolean' in property.default) return String(property.default.Boolean)
    return ''
  }

  const canExpand =
    property.type === 'object' || property.type === 'array' || property.type === 'string'

  return (
    <div
      className={`border border-white/10 rounded-lg bg-black/20 ${isDragging ? 'shadow-lg ring-2 ring-primary-500/50' : ''}`}
      style={{ marginLeft: depth > 0 ? 12 : 0 }}
    >
      {/* Single row: drag, expand, type, name, req, desc toggle, delete */}
      <div className="flex items-center gap-1.5 p-2">
        {/* Drag handle */}
        <div
          {...(dragHandleProps || {})}
          className="flex-shrink-0 text-gray-500 hover:text-gray-300 cursor-grab active:cursor-grabbing"
        >
          <GripVertical className="w-3.5 h-3.5" />
        </div>

        {/* Expand/collapse button */}
        {canExpand ? (
          <button
            type="button"
            onClick={() => setIsExpanded(!isExpanded)}
            className="flex-shrink-0 text-gray-500 hover:text-gray-300"
          >
            {isExpanded ? (
              <ChevronDown className="w-3.5 h-3.5" />
            ) : (
              <ChevronRight className="w-3.5 h-3.5" />
            )}
          </button>
        ) : (
          <div className="w-3.5" />
        )}

        {/* Type selector - compact */}
        <select value={property.type} onChange={handleTypeChange} className={`${selectClass} w-[72px] shrink-0 text-xs`}>
          {TYPE_OPTIONS.map((opt) => (
            <option key={opt.value} value={opt.value}>
              {opt.label}
            </option>
          ))}
        </select>

        {/* Property name */}
        <input
          type="text"
          value={property.name}
          onChange={handleNameChange}
          placeholder="key_name"
          className={`${inputClass} flex-1 min-w-0`}
        />

        {/* Required checkbox */}
        <label className="flex items-center gap-1 text-xs text-gray-400 cursor-pointer shrink-0" title="Required">
          <input
            type="checkbox"
            checked={property.required}
            onChange={handleRequiredChange}
            className="w-3 h-3 rounded border-white/20 bg-white/5 text-primary-500 focus:ring-primary-500"
          />
          <span className="hidden sm:inline">Req</span>
        </label>

        {/* Unique checkbox */}
        <label className="flex items-center gap-1 text-xs text-gray-400 cursor-pointer shrink-0" title="Unique">
          <input
            type="checkbox"
            checked={property.unique || false}
            onChange={handleUniqueChange}
            className="w-3 h-3 rounded border-white/20 bg-white/5 text-primary-500 focus:ring-primary-500"
          />
          <span className="hidden sm:inline">Uniq</span>
        </label>

        {/* Translatable checkbox */}
        <label className="flex items-center gap-1 text-xs text-gray-400 cursor-pointer shrink-0" title="Translatable (i18n)">
          <input
            type="checkbox"
            checked={property.translatable || false}
            onChange={handleTranslatableChange}
            className="w-3 h-3 rounded border-white/20 bg-white/5 text-primary-500 focus:ring-primary-500"
          />
          <span className="hidden sm:inline">i18n</span>
        </label>

        {/* Description toggle */}
        <button
          type="button"
          onClick={() => setShowDescription(!showDescription)}
          className={`shrink-0 px-1.5 py-0.5 rounded text-xs ${
            showDescription
              ? 'bg-primary-500/20 text-primary-300'
              : property.description
                ? 'text-gray-400 hover:text-gray-300'
                : 'text-yellow-500 hover:text-yellow-400'
          }`}
          title={property.description ? 'Edit description' : 'Add description (required)'}
        >
          Desc
        </button>

        {/* Advanced settings toggle */}
        <button
          type="button"
          onClick={() => setShowAdvanced(!showAdvanced)}
          className={`shrink-0 p-0.5 rounded ${
            showAdvanced ? 'bg-primary-500/20 text-primary-300' : 'text-gray-500 hover:text-gray-300'
          }`}
          title="Advanced settings (label, order, default, constraints, indexes)"
        >
          <Settings2 className="w-3.5 h-3.5" />
        </button>

        {/* Remove button */}
        <button
          type="button"
          onClick={onRemove}
          className="flex-shrink-0 text-red-400 hover:text-red-300 hover:bg-red-500/10 rounded p-0.5"
        >
          <X className="w-3.5 h-3.5" />
        </button>
      </div>

      {/* Description field - shown when toggled */}
      {showDescription && (
        <div className="px-2 pb-2">
          <input
            type="text"
            value={property.description || ''}
            onChange={handleDescriptionChange}
            placeholder="Description for AI agents (required)"
            className={`${inputClass} w-full ${!property.description ? 'border-yellow-500/50' : ''}`}
          />
        </div>
      )}

      {/* Advanced settings panel */}
      {showAdvanced && (
        <div className="px-3 pb-3 space-y-3 border-t border-white/5 mt-1 pt-2">
          {/* Label and Order row */}
          <div className="flex gap-4">
            <div className="flex-1">
              <label className="block text-xs text-gray-400 mb-1">Label</label>
              <input
                type="text"
                value={property.label || ''}
                onChange={handleLabelChange}
                placeholder="Human-readable label"
                className={`${inputClass} w-full`}
              />
            </div>
            <div className="w-20">
              <label className="block text-xs text-gray-400 mb-1">Order</label>
              <input
                type="number"
                value={property.order ?? ''}
                onChange={handleOrderChange}
                placeholder="#"
                className={`${inputClass} w-full`}
              />
            </div>
          </div>

          {/* Default value */}
          <div>
            <label className="block text-xs text-gray-400 mb-1">Default Value</label>
            {property.type === 'boolean' ? (
              <select
                value={getDefaultValueDisplay()}
                onChange={(e) => handleDefaultChange(e.target.value)}
                className={`${selectClass} w-full`}
              >
                <option value="">No default</option>
                <option value="true">true</option>
                <option value="false">false</option>
              </select>
            ) : (
              <input
                type={property.type === 'number' ? 'number' : 'text'}
                value={getDefaultValueDisplay()}
                onChange={(e) => handleDefaultChange(e.target.value)}
                placeholder={property.type === 'number' ? '0' : 'Default value'}
                className={`${inputClass} w-full`}
              />
            )}
          </div>

          {/* Indexes */}
          <div>
            <div className="flex items-center justify-between mb-1">
              <span className="text-xs text-gray-400">Indexes</span>
            </div>
            <div className="flex gap-3">
              {VALID_INDEX_TYPES.map((idx) => (
                <label key={idx} className="flex items-center gap-1.5 text-xs text-gray-400 cursor-pointer">
                  <input
                    type="checkbox"
                    checked={property.index?.includes(idx) || false}
                    onChange={() => handleIndexToggle(idx)}
                    className="w-3 h-3 rounded border-white/20 bg-white/5 text-primary-500 focus:ring-primary-500"
                  />
                  <span className="capitalize">{idx}</span>
                </label>
              ))}
            </div>
          </div>

          {/* Constraints - type specific */}
          {(property.type === 'number' || property.type === 'string') && (
            <div>
              <button
                type="button"
                onClick={() => setShowConstraints(!showConstraints)}
                className="flex items-center gap-1 text-xs text-gray-400 hover:text-gray-300"
              >
                {showConstraints ? <ChevronDown className="w-3 h-3" /> : <ChevronRight className="w-3 h-3" />}
                Constraints
              </button>
              {showConstraints && (
                <div className="mt-2 pl-4 space-y-2">
                  {property.type === 'number' && (
                    <>
                      <div className="flex gap-3">
                        <div className="flex-1">
                          <label className="block text-xs text-gray-500 mb-1">Min</label>
                          <input
                            type="number"
                            value={property.constraints?.min ?? ''}
                            onChange={(e) => handleConstraintChange('min', e.target.value)}
                            placeholder="Min"
                            className={`${inputClass} w-full`}
                          />
                        </div>
                        <div className="flex-1">
                          <label className="block text-xs text-gray-500 mb-1">Max</label>
                          <input
                            type="number"
                            value={property.constraints?.max ?? ''}
                            onChange={(e) => handleConstraintChange('max', e.target.value)}
                            placeholder="Max"
                            className={`${inputClass} w-full`}
                          />
                        </div>
                      </div>
                      <label className="flex items-center gap-1.5 text-xs text-gray-400 cursor-pointer">
                        <input
                          type="checkbox"
                          checked={property.constraints?.isInteger || false}
                          onChange={(e) => handleConstraintChange('isInteger', e.target.checked)}
                          className="w-3 h-3 rounded border-white/20 bg-white/5 text-primary-500 focus:ring-primary-500"
                        />
                        Integer only (no decimals)
                      </label>
                    </>
                  )}
                  {property.type === 'string' && (
                    <>
                      <div className="flex gap-3">
                        <div className="flex-1">
                          <label className="block text-xs text-gray-500 mb-1">Min Length</label>
                          <input
                            type="number"
                            min="0"
                            value={property.constraints?.minLength ?? ''}
                            onChange={(e) => handleConstraintChange('minLength', e.target.value)}
                            placeholder="0"
                            className={`${inputClass} w-full`}
                          />
                        </div>
                        <div className="flex-1">
                          <label className="block text-xs text-gray-500 mb-1">Max Length</label>
                          <input
                            type="number"
                            min="0"
                            value={property.constraints?.maxLength ?? ''}
                            onChange={(e) => handleConstraintChange('maxLength', e.target.value)}
                            placeholder="∞"
                            className={`${inputClass} w-full`}
                          />
                        </div>
                      </div>
                      <div>
                        <label className="block text-xs text-gray-500 mb-1">Pattern (regex)</label>
                        <input
                          type="text"
                          value={property.constraints?.pattern ?? ''}
                          onChange={(e) => handleConstraintChange('pattern', e.target.value)}
                          placeholder="^[a-z]+$"
                          className={`${inputClass} w-full font-mono text-xs`}
                        />
                      </div>
                    </>
                  )}
                </div>
              )}
            </div>
          )}
        </div>
      )}

      {/* Expanded content */}
      {isExpanded && (
        <>
          {/* Enum values for string type */}
          {property.type === 'string' && (
            <div className="px-3 pb-3 border-t border-white/5 mt-1 pt-2">
              <div className="flex items-center justify-between mb-2">
                <span className="text-xs text-gray-400">Enum Values (optional)</span>
                <button
                  type="button"
                  onClick={handleAddEnumValue}
                  className="flex items-center gap-1 text-xs text-primary-400 hover:text-primary-300"
                >
                  <Plus className="w-3 h-3" />
                  Add
                </button>
              </div>
              {property.enum && property.enum.length > 0 && (
                <div className="flex flex-wrap gap-2">
                  {property.enum.map((val, i) => (
                    <div key={i} className="flex items-center gap-1 bg-white/5 rounded px-1">
                      <input
                        type="text"
                        value={val}
                        onChange={(e) => handleEnumValueChange(i, e.target.value)}
                        placeholder="value"
                        className="w-24 px-1.5 py-1 bg-transparent text-xs text-white focus:outline-none"
                      />
                      <button
                        type="button"
                        onClick={() => handleRemoveEnumValue(i)}
                        className="p-0.5 text-gray-500 hover:text-red-400"
                      >
                        <X className="w-3 h-3" />
                      </button>
                    </div>
                  ))}
                </div>
              )}
            </div>
          )}

          {/* Nested properties for object type */}
          {property.type === 'object' && (
            <div className="px-3 pb-3 border-t border-white/5 mt-1 pt-2">
              <div className="flex items-center justify-between mb-2">
                <span className="text-xs text-gray-400">Nested Properties</span>
                <button
                  type="button"
                  onClick={handleAddNestedProperty}
                  className="flex items-center gap-1 text-xs text-primary-400 hover:text-primary-300"
                >
                  <Plus className="w-3 h-3" />
                  Add Property
                </button>
              </div>
              {property.properties && property.properties.length > 0 && (
                <div className="space-y-2">
                  {property.properties.map((nestedProp, i) => (
                    <SchemaPropertyEditor
                      key={nestedProp.id}
                      property={nestedProp}
                      onChange={(updated) => handleNestedPropertyChange(i, updated)}
                      onRemove={() => handleRemoveNestedProperty(i)}
                      depth={depth + 1}
                    />
                  ))}
                </div>
              )}
            </div>
          )}

          {/* Items type for array type */}
          {property.type === 'array' && property.items && (
            <div className="px-3 pb-3 border-t border-white/5 mt-1 pt-2">
              <div className="mb-2">
                <span className="text-xs text-gray-400">Array Item Type</span>
              </div>
              <SchemaPropertyEditor
                property={property.items}
                onChange={handleItemsChange}
                onRemove={() => {
                  /* Cannot remove array items schema */
                }}
                depth={depth + 1}
              />
            </div>
          )}
        </>
      )}
    </div>
  )
}
