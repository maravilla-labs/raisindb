/**
 * Archetype Visual Builder
 *
 * Visual drag-and-drop builder for archetype field definitions.
 * Uses Pragmatic Drag and Drop for reliable nested drop support.
 */

import { useState, useEffect, useCallback } from 'react'
import FieldTypeToolbox from './FieldTypeToolbox'
import FieldCanvas from './FieldCanvas'
import FieldEditorPanel from './FieldEditorPanel'
import CoreSettingsPanel from './CoreSettingsPanel'
import {
  useBuilderDropMonitor,
  DragPreviewProvider,
  DragOverlay,
  type DropResult,
} from '../shared/builder'
import { createNewField, validateArchetype } from './utils'
import type { ArchetypeDefinition, FieldType, FieldSchema } from './types'

interface ArchetypeVisualBuilderProps {
  archetype: ArchetypeDefinition
  onChange: (archetype: ArchetypeDefinition) => void
  repo?: string
  branch?: string
}

// Parse path segment - handles both "name" and "[index]"
interface PathSegment {
  type: 'key' | 'index'
  value: string | number
}

function parsePath(path: string): PathSegment[] {
  const segments: PathSegment[] = []
  // Match either field names or array indices
  const regex = /([^.\[\]]+)|\[(\d+)\]/g
  let match
  while ((match = regex.exec(path)) !== null) {
    if (match[1] !== undefined) {
      segments.push({ type: 'key', value: match[1] })
    } else if (match[2] !== undefined) {
      segments.push({ type: 'index', value: parseInt(match[2], 10) })
    }
  }
  return segments
}

// Navigate to a field by path and return it
// Path format: "fieldId" for top-level, "fieldId[0]" for nested
function getFieldByPath(
  fields: FieldSchema[],
  path: string
): FieldSchema | undefined {
  const segments = parsePath(path)
  if (segments.length === 0) return undefined

  // First segment must be a key (field ID)
  const first = segments[0]
  if (first.type !== 'key') return undefined

  let current = fields.find((f) => f.id === first.value)
  if (!current) return undefined

  // Navigate remaining segments
  for (let i = 1; i < segments.length; i++) {
    const seg = segments[i]
    if (seg.type === 'index') {
      // CompositeField nested fields array navigation
      if (current.$type !== 'CompositeField' || !Array.isArray(current.fields)) return undefined
      current = current.fields[seg.value as number]
    } else {
      // Key navigation - find by name in nested fields
      if (current.$type !== 'CompositeField' || !Array.isArray(current.fields)) return undefined
      current = current.fields.find((f) => f.name === seg.value)
    }
    if (!current) return undefined
  }

  return current
}

// Deep clone and update field by path
function updateFieldByPath(
  fields: FieldSchema[],
  path: string,
  updater: (field: FieldSchema) => FieldSchema
): FieldSchema[] {
  const segments = parsePath(path)
  if (segments.length === 0) return fields

  const first = segments[0]
  if (first.type !== 'key') return fields

  return fields.map((field) => {
    if (field.id !== first.value) return field
    if (segments.length === 1) return updater(field)

    // Deep clone
    const newField = JSON.parse(JSON.stringify(field)) as FieldSchema
    let current = newField

    // Navigate to parent of target
    for (let i = 1; i < segments.length - 1; i++) {
      const seg = segments[i]
      if (seg.type === 'index') {
        if (current.$type !== 'CompositeField' || !Array.isArray(current.fields)) return field
        current = current.fields[seg.value as number]
      } else {
        if (current.$type !== 'CompositeField' || !Array.isArray(current.fields)) return field
        const found = current.fields.find((f) => f.name === seg.value)
        if (!found) return field
        current = found
      }
    }

    // Update the leaf
    const lastSeg = segments[segments.length - 1]
    if (current.$type !== 'CompositeField' || !Array.isArray(current.fields)) return field

    if (lastSeg.type === 'index') {
      current.fields[lastSeg.value as number] = updater(current.fields[lastSeg.value as number])
    } else {
      const idx = current.fields.findIndex((f) => f.name === lastSeg.value)
      if (idx >= 0) {
        current.fields[idx] = updater(current.fields[idx])
      }
    }

    return newField
  })
}

// Delete field by path
function deleteFieldByPath(
  fields: FieldSchema[],
  path: string
): FieldSchema[] {
  const segments = parsePath(path)
  if (segments.length === 0) return fields

  const first = segments[0]
  if (first.type !== 'key') return fields

  // Top-level deletion
  if (segments.length === 1) {
    return fields.filter((f) => f.id !== first.value)
  }

  return fields.map((field) => {
    if (field.id !== first.value) return field

    // Deep clone
    const newField = JSON.parse(JSON.stringify(field)) as FieldSchema
    let current = newField

    // Navigate to parent of target
    for (let i = 1; i < segments.length - 1; i++) {
      const seg = segments[i]
      if (seg.type === 'index') {
        if (current.$type !== 'CompositeField' || !Array.isArray(current.fields)) return field
        current = current.fields[seg.value as number]
      } else {
        if (current.$type !== 'CompositeField' || !Array.isArray(current.fields)) return field
        const found = current.fields.find((f) => f.name === seg.value)
        if (!found) return field
        current = found
      }
    }

    // Delete the leaf
    const lastSeg = segments[segments.length - 1]
    if (current.$type !== 'CompositeField' || !Array.isArray(current.fields)) return field

    if (lastSeg.type === 'index') {
      current.fields.splice(lastSeg.value as number, 1)
    } else {
      current.fields = current.fields.filter((f) => f.name !== lastSeg.value)
    }

    return newField
  })
}

// Get parent path from a field path
function getParentPath(path: string): string | null {
  const segments = parsePath(path)
  if (segments.length <= 1) return null

  // Reconstruct parent path without last segment
  const parentSegments = segments.slice(0, -1)
  let result = ''
  for (const seg of parentSegments) {
    if (seg.type === 'key') {
      result = result ? `${result}.${seg.value}` : String(seg.value)
    } else {
      result = `${result}[${seg.value}]`
    }
  }
  return result
}

// Get the index within a container from a path
function getIndexFromPath(path: string): number | null {
  const segments = parsePath(path)
  if (segments.length === 0) return null

  const lastSeg = segments[segments.length - 1]
  if (lastSeg.type === 'index') {
    return lastSeg.value as number
  }

  // For top-level fields, return null (use field lookup)
  return null
}

// Add field to nested path (for CompositeField)
function addNestedField(
  fields: FieldSchema[],
  targetPath: string,
  newField: FieldSchema,
  index?: number
): { fields: FieldSchema[]; newPath: string } {
  const segments = parsePath(targetPath)
  if (segments.length === 0) {
    return { fields, newPath: '' }
  }

  const first = segments[0]
  if (first.type !== 'key') {
    return { fields, newPath: '' }
  }

  let resultPath = ''

  const newFields = fields.map((field) => {
    if (field.id !== first.value) return field

    // Deep clone
    const clonedField = JSON.parse(JSON.stringify(field)) as FieldSchema
    let current = clonedField

    // Navigate to target
    for (let i = 1; i < segments.length; i++) {
      const seg = segments[i]
      if (seg.type === 'index') {
        if (current.$type !== 'CompositeField' || !Array.isArray(current.fields)) return field
        current = current.fields[seg.value as number]
      } else {
        if (current.$type !== 'CompositeField' || !Array.isArray(current.fields)) return field
        const found = current.fields.find((f) => f.name === seg.value)
        if (!found) return field
        current = found
      }
    }

    // Add to the target's fields array (CompositeField)
    if (current.$type === 'CompositeField') {
      if (!Array.isArray(current.fields)) current.fields = []
      const insertIndex = index !== undefined ? index : current.fields.length
      current.fields.splice(insertIndex, 0, newField)
      resultPath = `${targetPath}[${insertIndex}]`
    }

    return clonedField
  })

  return { fields: newFields, newPath: resultPath }
}

// Move a field from one location to another
function moveField(
  fields: FieldSchema[],
  sourcePath: string,
  targetPath: string | null,
  position: 'before' | 'after' | 'inside',
  targetIndex?: number
): { fields: FieldSchema[]; newPath: string } {
  // Get the field to move
  const fieldToMove = getFieldByPath(fields, sourcePath)
  if (!fieldToMove) return { fields, newPath: sourcePath }

  // Clone the field
  const clonedField = JSON.parse(JSON.stringify(fieldToMove)) as FieldSchema

  // Delete from source
  let newFields = deleteFieldByPath(fields, sourcePath)

  // Handle top-level drop (targetPath is null or empty)
  if (!targetPath) {
    const insertIndex = targetIndex !== undefined ? targetIndex : newFields.length
    newFields = [...newFields.slice(0, insertIndex), clonedField, ...newFields.slice(insertIndex)]
    return { fields: newFields, newPath: clonedField.id! }
  }

  // Get target field info
  const targetField = getFieldByPath(newFields, targetPath)
  if (!targetField) return { fields: newFields, newPath: clonedField.id! }

  const parentPath = getParentPath(targetPath)
  const targetIdx = getIndexFromPath(targetPath)

  if (position === 'inside') {
    // Drop inside a container
    if (targetField.$type === 'CompositeField') {
      const { fields: updatedFields, newPath } = addNestedField(
        newFields,
        targetPath,
        clonedField,
        targetIndex
      )
      return { fields: updatedFields, newPath }
    }
    // Can't drop inside non-container
    return { fields: newFields, newPath: clonedField.id! }
  }

  // Before/after positioning
  if (parentPath === null) {
    // Target is top-level
    const topLevelIndex = newFields.findIndex((f) => f.id === targetField.id)
    if (topLevelIndex === -1) return { fields: newFields, newPath: clonedField.id! }

    const insertIndex = position === 'before' ? topLevelIndex : topLevelIndex + 1
    newFields = [
      ...newFields.slice(0, insertIndex),
      clonedField,
      ...newFields.slice(insertIndex),
    ]
    return { fields: newFields, newPath: clonedField.id! }
  }

  // Target is nested - add relative to sibling
  if (targetIdx !== null) {
    const insertIndex = position === 'before' ? targetIdx : targetIdx + 1
    const { fields: updatedFields, newPath } = addNestedField(
      newFields,
      parentPath,
      clonedField,
      insertIndex
    )
    return { fields: updatedFields, newPath }
  }

  return { fields: newFields, newPath: clonedField.id! }
}

export default function ArchetypeVisualBuilder({
  archetype,
  onChange,
  repo,
  branch,
}: ArchetypeVisualBuilderProps) {
  const [selectedPath, setSelectedPath] = useState<string | undefined>()
  const [validationErrors, setValidationErrors] = useState<Record<string, string>>({})

  // Validate on changes
  useEffect(() => {
    const errors = validateArchetype(archetype)
    setValidationErrors(errors)
  }, [archetype])

  // Handle drop from Pragmatic DnD
  const handleDrop = useCallback(
    (result: DropResult) => {
      const { source, targetPath, position } = result

      console.log('Drop result:', { source, targetPath, position })

      if (source.type === 'toolbox-item') {
        // Adding new field from toolbox
        const fieldType = source.itemType as FieldType
        const newField = createNewField(fieldType)

        if (!targetPath) {
          // Drop at top level (end of list)
          const newFields = [...archetype.fields, newField]
          onChange({ ...archetype, fields: newFields })
          setSelectedPath(newField.id!)
          return
        }

        // Get target field to determine insertion point
        const targetField = getFieldByPath(archetype.fields, targetPath)
        if (!targetField) {
          // Invalid target, add to end
          const newFields = [...archetype.fields, newField]
          onChange({ ...archetype, fields: newFields })
          setSelectedPath(newField.id!)
          return
        }

        if (position === 'inside') {
          // Drop inside a CompositeField
          if (targetField.$type === 'CompositeField') {
            const { fields: newFields, newPath } = addNestedField(
              archetype.fields,
              targetPath,
              newField
            )
            onChange({ ...archetype, fields: newFields })
            if (newPath) setSelectedPath(newPath)
          }
          return
        }

        // Before/after positioning
        const parentPath = getParentPath(targetPath)
        const targetIdx = getIndexFromPath(targetPath)

        if (parentPath === null) {
          // Target is top-level
          const topLevelIndex = archetype.fields.findIndex((f) => f.id === targetField.id)
          if (topLevelIndex === -1) return

          const insertIndex = position === 'before' ? topLevelIndex : topLevelIndex + 1
          const newFields = [
            ...archetype.fields.slice(0, insertIndex),
            newField,
            ...archetype.fields.slice(insertIndex),
          ]
          onChange({ ...archetype, fields: newFields })
          setSelectedPath(newField.id!)
          return
        }

        // Target is nested
        if (targetIdx !== null) {
          const insertIndex = position === 'before' ? targetIdx : targetIdx + 1
          const { fields: newFields, newPath } = addNestedField(
            archetype.fields,
            parentPath,
            newField,
            insertIndex
          )
          onChange({ ...archetype, fields: newFields })
          if (newPath) setSelectedPath(newPath)
        }
        return
      }

      if (source.type === 'builder-item') {
        // Moving existing field
        const sourcePath = source.path
        if (!sourcePath) return

        const { fields: newFields, newPath } = moveField(
          archetype.fields,
          sourcePath,
          targetPath,
          position
        )

        onChange({ ...archetype, fields: newFields })
        setSelectedPath(newPath)
      }
    },
    [archetype, onChange]
  )

  // Use the drop monitor hook
  useBuilderDropMonitor({ onDrop: handleDrop })

  const handleAddField = (type: FieldType) => {
    const newField = createNewField(type)
    onChange({
      ...archetype,
      fields: [...archetype.fields, newField],
    })
    setSelectedPath(newField.id!)
  }

  const handleFieldChange = (updatedField: FieldSchema) => {
    if (!selectedPath) return

    const newFields = updateFieldByPath(
      archetype.fields,
      selectedPath,
      () => updatedField
    )

    // Path uses field IDs, so no update needed when name changes
    onChange({ ...archetype, fields: newFields })
  }

  const handleFieldDelete = (path: string) => {
    const newFields = deleteFieldByPath(archetype.fields, path)
    onChange({ ...archetype, fields: newFields })

    if (selectedPath === path || selectedPath?.startsWith(path + '.') || selectedPath?.startsWith(path + '[')) {
      setSelectedPath(undefined)
    }
  }

  const selectedField = selectedPath
    ? getFieldByPath(archetype.fields, selectedPath)
    : undefined

  return (
    <DragPreviewProvider>
      <div className="h-full flex overflow-hidden">
        {/* Left: Toolbox */}
        <div className="w-40 flex-shrink-0">
          <FieldTypeToolbox onAddField={handleAddField} />
        </div>

        {/* Center: Canvas */}
        <div className="flex-1 overflow-hidden">
          <FieldCanvas
            archetype={archetype}
            selectedPath={selectedPath}
            onPathSelect={setSelectedPath}
            onPathDelete={handleFieldDelete}
          />
        </div>

        {/* Right: Field Editor OR Core Settings */}
        <div className="w-80 flex-shrink-0">
          {selectedField ? (
            <FieldEditorPanel
              field={selectedField}
              onChange={handleFieldChange}
              onDelete={() => handleFieldDelete(selectedPath!)}
              repo={repo}
              branch={branch}
            />
          ) : (
            <CoreSettingsPanel
              archetype={archetype}
              onChange={onChange}
              validationErrors={validationErrors}
            />
          )}
        </div>
      </div>
      <DragOverlay />
    </DragPreviewProvider>
  )
}
