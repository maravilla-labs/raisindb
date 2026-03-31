/**
 * Schema Visual Builder Component
 *
 * Drag-and-drop visual builder for JSON Schema properties.
 * Allows adding, removing, and reordering properties visually.
 */

import { useCallback } from 'react'
import { DragDropContext, Droppable, Draggable, type DropResult } from '@hello-pangea/dnd'
import { Plus, AlertCircle, Braces } from 'lucide-react'
import { SchemaPropertyEditor } from './SchemaPropertyEditor'
import {
  type EditableSchema,
  type SchemaProperty,
  type SchemaValidationError,
  createEmptyProperty,
} from './schema-types'

interface SchemaVisualBuilderProps {
  /** The schema being edited */
  schema: EditableSchema
  /** Called when the schema changes */
  onChange: (schema: EditableSchema) => void
  /** Validation errors to display */
  errors?: SchemaValidationError[]
}

export function SchemaVisualBuilder({
  schema,
  onChange,
  errors = [],
}: SchemaVisualBuilderProps) {
  const inputClass = `w-full px-2 py-1.5 bg-white/5 border border-white/10 rounded text-sm text-white
    placeholder-gray-500 focus:outline-none focus:ring-1 focus:ring-primary-500 focus:border-primary-500`

  // Handle drag end
  const handleDragEnd = useCallback(
    (result: DropResult) => {
      if (!result.destination) return

      const items = Array.from(schema.properties)
      const [reorderedItem] = items.splice(result.source.index, 1)
      items.splice(result.destination.index, 0, reorderedItem)

      onChange({ ...schema, properties: items })
    },
    [schema, onChange]
  )

  // Handle description change
  const handleDescriptionChange = useCallback(
    (e: React.ChangeEvent<HTMLTextAreaElement>) => {
      onChange({ ...schema, description: e.target.value })
    },
    [schema, onChange]
  )

  // Add a new property
  const handleAddProperty = useCallback(() => {
    const newProperty = createEmptyProperty('string')
    onChange({
      ...schema,
      properties: [...schema.properties, newProperty],
    })
  }, [schema, onChange])

  // Update a property
  const handlePropertyChange = useCallback(
    (index: number, updated: SchemaProperty) => {
      const items = [...schema.properties]
      items[index] = updated
      onChange({ ...schema, properties: items })
    },
    [schema, onChange]
  )

  // Remove a property
  const handlePropertyRemove = useCallback(
    (index: number) => {
      const items = [...schema.properties]
      items.splice(index, 1)
      onChange({ ...schema, properties: items })
    },
    [schema, onChange]
  )

  return (
    <div className="space-y-4">
      {/* Schema description */}
      <div>
        <label className="block text-xs text-gray-400 mb-1">Schema Description</label>
        <textarea
          value={schema.description || ''}
          onChange={handleDescriptionChange}
          placeholder="Describe what this schema represents..."
          rows={2}
          className={`${inputClass} resize-none`}
        />
      </div>

      {/* Properties header */}
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <Braces className="w-4 h-4 text-gray-400" />
          <span className="text-sm font-medium text-gray-300">Properties</span>
          <span className="text-xs text-gray-500">({schema.properties.length})</span>
        </div>
        <button
          type="button"
          onClick={handleAddProperty}
          className="flex items-center gap-1.5 px-2 py-1 text-xs bg-primary-500/20 text-primary-300 rounded hover:bg-primary-500/30"
        >
          <Plus className="w-3.5 h-3.5" />
          Add Property
        </button>
      </div>

      {/* Properties list with drag-drop */}
      {schema.properties.length > 0 ? (
        <DragDropContext onDragEnd={handleDragEnd}>
          <Droppable droppableId="schema-properties">
            {(provided) => (
              <div
                ref={provided.innerRef}
                {...provided.droppableProps}
                className="space-y-2"
              >
                {schema.properties.map((property, index) => (
                  <Draggable key={property.id} draggableId={property.id} index={index}>
                    {(provided, snapshot) => (
                      <div
                        ref={provided.innerRef}
                        {...provided.draggableProps}
                      >
                        <SchemaPropertyEditor
                          property={property}
                          onChange={(updated) => handlePropertyChange(index, updated)}
                          onRemove={() => handlePropertyRemove(index)}
                          dragHandleProps={provided.dragHandleProps}
                          isDragging={snapshot.isDragging}
                        />
                      </div>
                    )}
                  </Draggable>
                ))}
                {provided.placeholder}
              </div>
            )}
          </Droppable>
        </DragDropContext>
      ) : (
        <div className="text-center py-8 border border-dashed border-white/10 rounded-lg">
          <Braces className="w-8 h-8 mx-auto text-gray-600 mb-2" />
          <p className="text-sm text-gray-500 mb-3">No properties defined yet</p>
          <button
            type="button"
            onClick={handleAddProperty}
            className="inline-flex items-center gap-1.5 px-3 py-1.5 text-sm bg-primary-500/20 text-primary-300 rounded hover:bg-primary-500/30"
          >
            <Plus className="w-4 h-4" />
            Add First Property
          </button>
        </div>
      )}

      {/* Validation errors */}
      {errors.length > 0 && (
        <div className="p-3 bg-red-500/10 border border-red-500/30 rounded-lg">
          <div className="flex items-center gap-2 text-red-400 text-sm font-medium mb-2">
            <AlertCircle className="w-4 h-4" />
            Validation Errors ({errors.length})
          </div>
          <ul className="space-y-1 text-sm text-red-300">
            {errors.slice(0, 5).map((error, i) => (
              <li key={i} className="flex gap-2">
                <span className="text-red-500/70 shrink-0">
                  {error.path || 'root'}:
                </span>
                <span>{error.message}</span>
              </li>
            ))}
            {errors.length > 5 && (
              <li className="text-red-400/70">
                ... and {errors.length - 5} more errors
              </li>
            )}
          </ul>
        </div>
      )}

      {/* Help text */}
      <div className="text-xs text-gray-500 space-y-1">
        <p>Drag properties to reorder them. Click the expand arrow to configure nested properties.</p>
        <p>Property names must be valid identifiers (letters, numbers, underscores; cannot start with a number).</p>
      </div>
    </div>
  )
}
