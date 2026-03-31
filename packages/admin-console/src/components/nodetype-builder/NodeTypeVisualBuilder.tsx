/**
 * NodeType Visual Builder
 *
 * Visual drag-and-drop builder for nodetype property definitions.
 * Uses Pragmatic Drag and Drop for reliable nested drop support.
 */

import { useState, useEffect, useCallback } from 'react'
import PropertyTypeToolbox from './PropertyTypeToolbox'
import PropertyCanvas from './PropertyCanvas'
import PropertyEditorPanel from './PropertyEditorPanel'
import CoreSettingsPanel from './CoreSettingsPanel'
import {
  useBuilderDropMonitor,
  DragPreviewProvider,
  DragOverlay,
  type DropResult,
} from '../shared/builder'
import { createNewProperty, validateNodeType } from './utils'
import type { NodeTypeDefinition, PropertyType, PropertyValueSchema } from './types'

interface NodeTypeVisualBuilderProps {
  nodeType: NodeTypeDefinition
  onChange: (nodeType: NodeTypeDefinition) => void
}

// Parse path segment - handles both "name" and "[index]"
interface PathSegment {
  type: 'key' | 'index'
  value: string | number
}

function parsePath(path: string): PathSegment[] {
  const segments: PathSegment[] = []
  // Match either property names or array indices
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

// Navigate to a property by path and return it
function getPropertyByPath(
  properties: PropertyValueSchema[],
  path: string
): PropertyValueSchema | undefined {
  const segments = parsePath(path)
  if (segments.length === 0) return undefined

  // First segment must be a key (property name)
  const first = segments[0]
  if (first.type !== 'key') return undefined

  let current = properties.find((p) => p.name === first.value)
  if (!current) return undefined

  // Navigate remaining segments
  for (let i = 1; i < segments.length; i++) {
    const seg = segments[i]
    if (seg.type === 'key') {
      // Object structure navigation
      if (!current.structure) return undefined
      current = current.structure[seg.value as string]
    } else {
      // Composite fields array navigation
      if (!Array.isArray(current.fields)) return undefined
      current = current.fields[seg.value as number]
    }
    if (!current) return undefined
  }

  return current
}

// Reconstruct path string from segments
function reconstructPath(segments: PathSegment[]): string {
  let path = ''
  for (const seg of segments) {
    if (seg.type === 'key') {
      path += path ? `.${seg.value}` : String(seg.value)
    } else {
      path += `[${seg.value}]`
    }
  }
  return path
}

// Get parent path from a property path
function getParentPath(path: string): string | null {
  const segments = parsePath(path)
  if (segments.length <= 1) return null

  // Reconstruct parent path without last segment
  const parentSegments = segments.slice(0, -1)
  return reconstructPath(parentSegments)
}

// Get the index within a container from a path
function getIndexFromPath(path: string): number | null {
  const segments = parsePath(path)
  if (segments.length === 0) return null

  const lastSeg = segments[segments.length - 1]
  if (lastSeg.type === 'index') {
    return lastSeg.value as number
  }

  // For top-level properties, return null (use property lookup)
  return null
}

// Deep clone and update property by path
function updatePropertyByPath(
  properties: PropertyValueSchema[],
  path: string,
  updater: (prop: PropertyValueSchema) => PropertyValueSchema
): PropertyValueSchema[] {
  const segments = parsePath(path)
  if (segments.length === 0) return properties

  const first = segments[0]
  if (first.type !== 'key') return properties

  return properties.map((prop) => {
    if (prop.name !== first.value) return prop
    if (segments.length === 1) return updater(prop)

    // Deep clone
    const newProp = JSON.parse(JSON.stringify(prop)) as PropertyValueSchema
    let current = newProp

    // Navigate to parent of target
    for (let i = 1; i < segments.length - 1; i++) {
      const seg = segments[i]
      if (seg.type === 'key') {
        current = current.structure![seg.value as string]
      } else {
        current = (current.fields as PropertyValueSchema[])[seg.value as number]
      }
    }

    // Update the leaf
    const lastSeg = segments[segments.length - 1]
    if (lastSeg.type === 'key') {
      current.structure![lastSeg.value as string] = updater(current.structure![lastSeg.value as string])
    } else {
      (current.fields as PropertyValueSchema[])[lastSeg.value as number] = updater(
        (current.fields as PropertyValueSchema[])[lastSeg.value as number]
      )
    }

    return newProp
  })
}

// Delete property by path
function deletePropertyByPath(
  properties: PropertyValueSchema[],
  path: string
): PropertyValueSchema[] {
  const segments = parsePath(path)
  if (segments.length === 0) return properties

  const first = segments[0]
  if (first.type !== 'key') return properties

  // Top-level deletion
  if (segments.length === 1) {
    return properties.filter((p) => p.name !== first.value)
  }

  return properties.map((prop) => {
    if (prop.name !== first.value) return prop

    // Deep clone
    const newProp = JSON.parse(JSON.stringify(prop)) as PropertyValueSchema
    let current = newProp

    // Navigate to parent of target
    for (let i = 1; i < segments.length - 1; i++) {
      const seg = segments[i]
      if (seg.type === 'key') {
        current = current.structure![seg.value as string]
      } else {
        current = (current.fields as PropertyValueSchema[])[seg.value as number]
      }
    }

    // Delete the leaf
    const lastSeg = segments[segments.length - 1]
    if (lastSeg.type === 'key') {
      delete current.structure![lastSeg.value as string]
    } else {
      (current.fields as PropertyValueSchema[]).splice(lastSeg.value as number, 1)
    }

    return newProp
  })
}

// Add property to nested path (for Object: structure, for Composite: fields array)
function addNestedProperty(
  properties: PropertyValueSchema[],
  targetPath: string,
  newProperty: PropertyValueSchema,
  index?: number
): { properties: PropertyValueSchema[]; newPath: string } {
  const segments = parsePath(targetPath)
  if (segments.length === 0) {
    return { properties, newPath: '' }
  }

  const first = segments[0]
  if (first.type !== 'key') {
    return { properties, newPath: '' }
  }

  let resultPath = ''

  const newProperties = properties.map((prop) => {
    if (prop.name !== first.value) return prop

    // Deep clone
    const newProp = JSON.parse(JSON.stringify(prop)) as PropertyValueSchema
    let current = newProp

    // Navigate to target
    for (let i = 1; i < segments.length; i++) {
      const seg = segments[i]
      if (seg.type === 'key') {
        current = current.structure![seg.value as string]
      } else {
        current = (current.fields as PropertyValueSchema[])[seg.value as number]
      }
    }

    // Add to the target based on its type
    if (current.type === 'Object') {
      // Object uses structure
      if (!current.structure) current.structure = {}
      let propName = newProperty.name || `prop_${Date.now()}`
      let counter = 1
      while (current.structure[propName]) {
        propName = `${newProperty.name || 'prop'}_${counter++}`
      }
      current.structure[propName] = { ...newProperty, name: propName }
      resultPath = `${targetPath}.${propName}`
    } else if (current.type === 'Composite') {
      // Composite uses fields array
      if (!Array.isArray(current.fields)) current.fields = []
      const insertIndex = index !== undefined ? index : current.fields.length
      current.fields.splice(insertIndex, 0, newProperty)
      resultPath = `${targetPath}[${insertIndex}]`
    }

    return newProp
  })

  return { properties: newProperties, newPath: resultPath }
}

// Move a property from one location to another
function moveProperty(
  properties: PropertyValueSchema[],
  sourcePath: string,
  targetPath: string | null,
  position: 'before' | 'after' | 'inside',
  targetIndex?: number
): { properties: PropertyValueSchema[]; newPath: string } {
  // Get the property to move
  const propertyToMove = getPropertyByPath(properties, sourcePath)
  if (!propertyToMove) return { properties, newPath: sourcePath }

  // Clone the property
  const clonedProperty = JSON.parse(JSON.stringify(propertyToMove)) as PropertyValueSchema

  // Delete from source
  let newProperties = deletePropertyByPath(properties, sourcePath)

  // Handle top-level drop (targetPath is null or empty)
  if (!targetPath) {
    const insertIndex = targetIndex !== undefined ? targetIndex : newProperties.length
    newProperties = [
      ...newProperties.slice(0, insertIndex),
      clonedProperty,
      ...newProperties.slice(insertIndex),
    ]
    return { properties: newProperties, newPath: clonedProperty.name! }
  }

  // Get target property info
  const targetProperty = getPropertyByPath(newProperties, targetPath)
  if (!targetProperty) return { properties: newProperties, newPath: clonedProperty.name! }

  const parentPath = getParentPath(targetPath)
  const targetIdx = getIndexFromPath(targetPath)

  if (position === 'inside') {
    // Drop inside a container
    if (targetProperty.type === 'Object' || targetProperty.type === 'Composite') {
      const { properties: updatedProperties, newPath } = addNestedProperty(
        newProperties,
        targetPath,
        clonedProperty,
        targetIndex
      )
      return { properties: updatedProperties, newPath }
    }
    // Can't drop inside non-container
    return { properties: newProperties, newPath: clonedProperty.name! }
  }

  // Before/after positioning
  if (parentPath === null) {
    // Target is top-level
    const topLevelIndex = newProperties.findIndex((p) => p.name === targetProperty.name)
    if (topLevelIndex === -1) return { properties: newProperties, newPath: clonedProperty.name! }

    const insertIndex = position === 'before' ? topLevelIndex : topLevelIndex + 1
    newProperties = [
      ...newProperties.slice(0, insertIndex),
      clonedProperty,
      ...newProperties.slice(insertIndex),
    ]
    return { properties: newProperties, newPath: clonedProperty.name! }
  }

  // Target is nested - add relative to sibling
  if (targetIdx !== null) {
    const insertIndex = position === 'before' ? targetIdx : targetIdx + 1
    const { properties: updatedProperties, newPath } = addNestedProperty(
      newProperties,
      parentPath,
      clonedProperty,
      insertIndex
    )
    return { properties: updatedProperties, newPath }
  }

  return { properties: newProperties, newPath: clonedProperty.name! }
}

export default function NodeTypeVisualBuilder({
  nodeType,
  onChange,
}: NodeTypeVisualBuilderProps) {
  const [selectedPath, setSelectedPath] = useState<string | undefined>()
  const [validationErrors, setValidationErrors] = useState<Record<string, string>>({})

  // Validate on changes
  useEffect(() => {
    const errors = validateNodeType(nodeType)
    setValidationErrors(errors)
  }, [nodeType])

  // Handle drop from Pragmatic DnD
  const handleDrop = useCallback(
    (result: DropResult) => {
      const { source, targetPath, position } = result

      console.log('Drop result:', { source, targetPath, position })

      if (source.type === 'toolbox-item') {
        // Adding new property from toolbox
        const propertyType = source.itemType as PropertyType
        const newProperty = createNewProperty(propertyType)

        if (!targetPath) {
          // Drop at top level (end of list)
          const newProperties = [...nodeType.properties, newProperty]
          onChange({ ...nodeType, properties: newProperties })
          setSelectedPath(newProperty.name)
          return
        }

        // Get target property to determine insertion point
        const targetProperty = getPropertyByPath(nodeType.properties, targetPath)
        if (!targetProperty) {
          // Invalid target, add to end
          const newProperties = [...nodeType.properties, newProperty]
          onChange({ ...nodeType, properties: newProperties })
          setSelectedPath(newProperty.name)
          return
        }

        if (position === 'inside') {
          // Drop inside an Object or Composite
          if (targetProperty.type === 'Object' || targetProperty.type === 'Composite') {
            const { properties: newProperties, newPath } = addNestedProperty(
              nodeType.properties,
              targetPath,
              newProperty
            )
            onChange({ ...nodeType, properties: newProperties })
            if (newPath) setSelectedPath(newPath)
          }
          return
        }

        // Before/after positioning
        const parentPath = getParentPath(targetPath)
        const targetIdx = getIndexFromPath(targetPath)

        if (parentPath === null) {
          // Target is top-level
          const topLevelIndex = nodeType.properties.findIndex((p) => p.name === targetProperty.name)
          if (topLevelIndex === -1) return

          const insertIndex = position === 'before' ? topLevelIndex : topLevelIndex + 1
          const newProperties = [
            ...nodeType.properties.slice(0, insertIndex),
            newProperty,
            ...nodeType.properties.slice(insertIndex),
          ]
          onChange({ ...nodeType, properties: newProperties })
          setSelectedPath(newProperty.name)
          return
        }

        // Target is nested
        if (targetIdx !== null) {
          const insertIndex = position === 'before' ? targetIdx : targetIdx + 1
          const { properties: newProperties, newPath } = addNestedProperty(
            nodeType.properties,
            parentPath,
            newProperty,
            insertIndex
          )
          onChange({ ...nodeType, properties: newProperties })
          if (newPath) setSelectedPath(newPath)
        }
        return
      }

      if (source.type === 'builder-item') {
        // Moving existing property
        const sourcePath = source.path
        if (!sourcePath) return

        const { properties: newProperties, newPath } = moveProperty(
          nodeType.properties,
          sourcePath,
          targetPath,
          position
        )

        onChange({ ...nodeType, properties: newProperties })
        setSelectedPath(newPath)
      }
    },
    [nodeType, onChange]
  )

  // Use the drop monitor hook
  useBuilderDropMonitor({ onDrop: handleDrop })

  const handleAddProperty = (type: PropertyType) => {
    const newProperty = createNewProperty(type)
    onChange({
      ...nodeType,
      properties: [...nodeType.properties, newProperty],
    })
    setSelectedPath(newProperty.name)
  }

  const handlePropertyChange = (updatedProperty: PropertyValueSchema) => {
    if (!selectedPath) return

    const newProperties = updatePropertyByPath(
      nodeType.properties,
      selectedPath,
      () => updatedProperty
    )

    // Handle name change for path update
    const segments = parsePath(selectedPath)
    const lastSeg = segments[segments.length - 1]
    if (lastSeg.type === 'key' && updatedProperty.name && updatedProperty.name !== lastSeg.value) {
      segments[segments.length - 1] = { type: 'key', value: updatedProperty.name }
      setSelectedPath(reconstructPath(segments))
    }

    onChange({ ...nodeType, properties: newProperties })
  }

  const handlePropertyDelete = (path: string) => {
    const newProperties = deletePropertyByPath(nodeType.properties, path)
    onChange({ ...nodeType, properties: newProperties })

    if (selectedPath === path || selectedPath?.startsWith(path + '.') || selectedPath?.startsWith(path + '[')) {
      setSelectedPath(undefined)
    }
  }

  const selectedProperty = selectedPath
    ? getPropertyByPath(nodeType.properties, selectedPath)
    : undefined

  return (
    <DragPreviewProvider>
      <div className="h-full flex overflow-hidden">
        {/* Left: Toolbox */}
        <div className="w-40 flex-shrink-0">
          <PropertyTypeToolbox onAddProperty={handleAddProperty} />
        </div>

        {/* Center: Canvas */}
        <div className="flex-1 overflow-hidden">
          <PropertyCanvas
            nodeType={nodeType}
            selectedPath={selectedPath}
            onPathSelect={setSelectedPath}
            onPathDelete={handlePropertyDelete}
          />
        </div>

        {/* Right: Property Editor OR Core Settings */}
        <div className="w-80 flex-shrink-0">
          {selectedProperty ? (
            <PropertyEditorPanel
              property={selectedProperty}
              path={selectedPath}
              onChange={handlePropertyChange}
              onDelete={() => handlePropertyDelete(selectedPath!)}
            />
          ) : (
            <CoreSettingsPanel
              nodeType={nodeType}
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
