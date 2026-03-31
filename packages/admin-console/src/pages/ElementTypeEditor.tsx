/**
 * Element Type Editor Page
 *
 * IDE-style editor for element types with resizable panels,
 * Visual/YAML tabs, and undo/redo support.
 */

import { useEffect, useState } from 'react'
import { useNavigate, useParams } from 'react-router-dom'
import { CheckCircle, Shapes, XCircle } from 'lucide-react'
import { Allotment } from 'allotment'
import 'allotment/dist/style.css'
import * as yaml from 'js-yaml'

import YamlEditor from '../components/YamlEditor'
import CommitDialog from '../components/CommitDialog'
import { useToast, ToastContainer } from '../components/Toast'
import {
  ElementTypeBuilderProvider,
  useElementTypeBuilderContext,
  ElementCoreSettingsPanel,
  parseYamlToElementType,
  serializeElementTypeToYaml,
  validateElementType,
  type ElementTypeDefinition,
} from '../components/element-builder'
// Reuse archetype-builder components for fields
import FieldTypeToolbox from '../components/archetype-builder/FieldTypeToolbox'
import FieldCanvas from '../components/archetype-builder/FieldCanvas'
import FieldEditorPanel from '../components/archetype-builder/FieldEditorPanel'
import { createNewField } from '../components/archetype-builder/utils'
import type { FieldSchema, FieldType } from '../components/archetype-builder/types'
import {
  BuilderToolbar,
  EditorTabs,
  type EditorMode,
  useBuilderDropMonitor,
  DragPreviewProvider,
  DragOverlay,
  type DropResult,
} from '../components/shared/builder'
import {
  elementTypesApi,
  type ElementType,
  type ElementTypeCommitPayload,
} from '../api/elementtypes'

// Helper functions for path manipulation (same as ArchetypeEditor)
interface PathSegment {
  type: 'key' | 'index'
  value: string | number
}

function parsePath(path: string): PathSegment[] {
  const segments: PathSegment[] = []
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

function getFieldByPath(fields: FieldSchema[], path: string): FieldSchema | undefined {
  const segments = parsePath(path)
  if (segments.length === 0) return undefined

  const first = segments[0]
  if (first.type !== 'key') return undefined

  let current = fields.find((f) => f.id === first.value)
  if (!current) return undefined

  for (let i = 1; i < segments.length; i++) {
    const seg = segments[i]
    if (seg.type === 'index') {
      if (current.$type !== 'CompositeField' || !Array.isArray(current.fields)) return undefined
      current = current.fields[seg.value as number]
    } else {
      if (current.$type !== 'CompositeField' || !Array.isArray(current.fields)) return undefined
      current = current.fields.find((f) => f.name === seg.value)
    }
    if (!current) return undefined
  }

  return current
}

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

    const newField = JSON.parse(JSON.stringify(field)) as FieldSchema
    let current = newField

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

function deleteFieldByPath(fields: FieldSchema[], path: string): FieldSchema[] {
  const segments = parsePath(path)
  if (segments.length === 0) return fields

  const first = segments[0]
  if (first.type !== 'key') return fields

  if (segments.length === 1) {
    return fields.filter((f) => f.id !== first.value)
  }

  return fields.map((field) => {
    if (field.id !== first.value) return field

    const newField = JSON.parse(JSON.stringify(field)) as FieldSchema
    let current = newField

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

function getParentPath(path: string): string | null {
  const segments = parsePath(path)
  if (segments.length <= 1) return null

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

function getIndexFromPath(path: string): number | null {
  const segments = parsePath(path)
  if (segments.length === 0) return null

  const lastSeg = segments[segments.length - 1]
  if (lastSeg.type === 'index') {
    return lastSeg.value as number
  }
  return null
}

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

    const clonedField = JSON.parse(JSON.stringify(field)) as FieldSchema
    let current = clonedField

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

function moveField(
  fields: FieldSchema[],
  sourcePath: string,
  targetPath: string | null,
  position: 'before' | 'after' | 'inside',
  targetIndex?: number
): { fields: FieldSchema[]; newPath: string } {
  const fieldToMove = getFieldByPath(fields, sourcePath)
  if (!fieldToMove) return { fields, newPath: sourcePath }

  const clonedField = JSON.parse(JSON.stringify(fieldToMove)) as FieldSchema
  let newFields = deleteFieldByPath(fields, sourcePath)

  if (!targetPath) {
    const insertIndex = targetIndex !== undefined ? targetIndex : newFields.length
    newFields = [...newFields.slice(0, insertIndex), clonedField, ...newFields.slice(insertIndex)]
    return { fields: newFields, newPath: clonedField.id! }
  }

  const targetField = getFieldByPath(newFields, targetPath)
  if (!targetField) return { fields: newFields, newPath: clonedField.id! }

  const parentPath = getParentPath(targetPath)
  const targetIdx = getIndexFromPath(targetPath)

  if (position === 'inside') {
    if (targetField.$type === 'CompositeField') {
      const { fields: updatedFields, newPath } = addNestedField(
        newFields,
        targetPath,
        clonedField,
        targetIndex
      )
      return { fields: updatedFields, newPath }
    }
    return { fields: newFields, newPath: clonedField.id! }
  }

  if (parentPath === null) {
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

type PendingCommit = {
  elementType: ElementType
  action: 'create' | 'update'
  targetName: string
}

// Default template for new element type
const DEFAULT_ELEMENT_TYPE: ElementTypeDefinition = {
  name: '',
  fields: [],
}

// Inner component that uses context
function ElementTypeEditorContent() {
  const { repo, branch, name } = useParams<{ repo: string; branch?: string; name?: string }>()
  const activeBranch = branch || 'main'
  const navigate = useNavigate()

  const {
    elementType,
    setElementType,
    selectedPath,
    setSelectedPath,
    undo,
    redo,
    canUndo,
    canRedo,
    preferences,
    setToolboxWidth,
    setPropertiesWidth,
  } = useElementTypeBuilderContext()

  const [editorMode, setEditorMode] = useState<EditorMode>('visual')
  const [yamlContent, setYamlContent] = useState<string>('')
  const [yamlError, setYamlError] = useState<string | null>(null)
  const [pendingCommit, setPendingCommit] = useState<PendingCommit | null>(null)
  const [saving, setSaving] = useState(false)
  const [currentElementType, setCurrentElementType] = useState<ElementType | null>(null)
  const [validationErrors, setValidationErrors] = useState<Record<string, string>>({})

  const { toasts, error: showError, success: showSuccess, closeToast } = useToast()

  const isNew = !name
  const isPublished = currentElementType?.publishable ?? false

  // Validate on elementType changes
  useEffect(() => {
    const errors = validateElementType(elementType)
    setValidationErrors(errors)
  }, [elementType])

  // Sync YAML when switching to source mode
  useEffect(() => {
    if (editorMode === 'source') {
      try {
        const yamlStr = serializeElementTypeToYaml(elementType)
        setYamlContent(yamlStr)
        setYamlError(null)
      } catch (err: any) {
        setYamlError(err.message || 'Failed to serialize')
      }
    }
  }, [editorMode, elementType])

  // Handle drop from DnD
  const handleDrop = (result: DropResult) => {
    const { source, targetPath, position } = result

    if (source.type === 'toolbox-item') {
      const fieldType = source.itemType as FieldType
      const newField = createNewField(fieldType)

      if (!targetPath) {
        const newFields = [...elementType.fields, newField]
        setElementType({ ...elementType, fields: newFields }, `Add ${fieldType}`)
        setSelectedPath(newField.id!)
        return
      }

      const targetField = getFieldByPath(elementType.fields, targetPath)
      if (!targetField) {
        const newFields = [...elementType.fields, newField]
        setElementType({ ...elementType, fields: newFields }, `Add ${fieldType}`)
        setSelectedPath(newField.id!)
        return
      }

      if (position === 'inside') {
        if (targetField.$type === 'CompositeField') {
          const { fields: newFields, newPath } = addNestedField(
            elementType.fields,
            targetPath,
            newField
          )
          setElementType({ ...elementType, fields: newFields }, `Add ${fieldType} to composite`)
          if (newPath) setSelectedPath(newPath)
        }
        return
      }

      const parentPath = getParentPath(targetPath)
      const targetIdx = getIndexFromPath(targetPath)

      if (parentPath === null) {
        const topLevelIndex = elementType.fields.findIndex((f) => f.id === targetField.id)
        if (topLevelIndex === -1) return

        const insertIndex = position === 'before' ? topLevelIndex : topLevelIndex + 1
        const newFields = [
          ...elementType.fields.slice(0, insertIndex),
          newField,
          ...elementType.fields.slice(insertIndex),
        ]
        setElementType({ ...elementType, fields: newFields }, `Add ${fieldType}`)
        setSelectedPath(newField.id!)
        return
      }

      if (targetIdx !== null) {
        const insertIndex = position === 'before' ? targetIdx : targetIdx + 1
        const { fields: newFields, newPath } = addNestedField(
          elementType.fields,
          parentPath,
          newField,
          insertIndex
        )
        setElementType({ ...elementType, fields: newFields }, `Add ${fieldType}`)
        if (newPath) setSelectedPath(newPath)
      }
      return
    }

    if (source.type === 'builder-item') {
      const sourcePath = source.path
      if (!sourcePath) return

      const { fields: newFields, newPath } = moveField(
        elementType.fields,
        sourcePath,
        targetPath,
        position
      )

      setElementType({ ...elementType, fields: newFields }, 'Move field')
      setSelectedPath(newPath)
    }
  }

  useBuilderDropMonitor({ onDrop: handleDrop })

  const handleAddField = (type: FieldType) => {
    const newField = createNewField(type)
    setElementType({
      ...elementType,
      fields: [...elementType.fields, newField],
    }, `Add ${type}`)
    setSelectedPath(newField.id!)
  }

  const handleFieldChange = (updatedField: FieldSchema) => {
    if (!selectedPath) return

    const newFields = updateFieldByPath(
      elementType.fields,
      selectedPath,
      () => updatedField
    )
    setElementType({ ...elementType, fields: newFields }, 'Update field')
  }

  const handleFieldDelete = (path: string) => {
    const newFields = deleteFieldByPath(elementType.fields, path)
    setElementType({ ...elementType, fields: newFields }, 'Delete field')

    if (selectedPath === path || selectedPath?.startsWith(path + '.') || selectedPath?.startsWith(path + '[')) {
      setSelectedPath(undefined)
    }
  }

  const handleTabChange = (tab: EditorMode) => {
    if (tab === editorMode) return

    if (tab === 'visual') {
      try {
        const parsed = parseYamlToElementType(yamlContent)
        setElementType(parsed, 'Update from YAML')
        setEditorMode('visual')
        setYamlError(null)
      } catch (err: any) {
        setYamlError(err?.message || 'Failed to parse YAML')
      }
    } else {
      try {
        const yamlStr = serializeElementTypeToYaml(elementType)
        setYamlContent(yamlStr)
        setEditorMode('source')
        setYamlError(null)
      } catch (err: any) {
        setYamlError(err.message || 'Failed to convert to YAML')
      }
    }
  }

  const handleSave = async () => {
    if (!repo) return

    try {
      let elementTypeToSave: ElementType

      if (editorMode === 'visual') {
        const yamlStr = serializeElementTypeToYaml(elementType)
        elementTypeToSave = yaml.load(yamlStr) as ElementType
        setYamlContent(yamlStr)
      } else {
        elementTypeToSave = yaml.load(yamlContent) as ElementType
      }

      if (!elementTypeToSave || typeof elementTypeToSave !== 'object') {
        throw new Error('YAML must define an element type object')
      }
      if (!elementTypeToSave.name || elementTypeToSave.name.trim().length === 0) {
        throw new Error('Element type name is required')
      }

      setPendingCommit({
        elementType: elementTypeToSave,
        action: isNew ? 'create' : 'update',
        targetName: isNew ? elementTypeToSave.name : name ?? elementTypeToSave.name,
      })
    } catch (err: any) {
      console.error('Failed to prepare element type for saving:', err)
      showError('Error', err.message || 'Failed to prepare element type for saving')
    }
  }

  const executeCommit = async (message: string, actor: string) => {
    if (!pendingCommit || !repo) return

    const commit: ElementTypeCommitPayload = {
      message: message.trim(),
      actor: actor.trim() || undefined,
    }

    setSaving(true)

    try {
      let saved: ElementType

      if (pendingCommit.action === 'create') {
        saved = await elementTypesApi.create(repo, activeBranch, pendingCommit.elementType, commit)
        showSuccess('Created', 'Element type created successfully!')
        navigate(`/${repo}/elementtypes/${saved.name}`)
      } else {
        saved = await elementTypesApi.update(
          repo,
          activeBranch,
          pendingCommit.targetName,
          pendingCommit.elementType,
          commit
        )
        showSuccess('Updated', 'Element type updated successfully!')
        setCurrentElementType(saved)
      }

      setPendingCommit(null)
    } catch (err: any) {
      console.error('Failed to save element type:', err)
      showError('Error', err.message || 'Failed to save element type')
      throw err
    } finally {
      setSaving(false)
    }
  }

  const handlePublish = async (desired: boolean) => {
    if (!repo || !name) return
    try {
      const result = desired
        ? await elementTypesApi.publish(repo, activeBranch, name)
        : await elementTypesApi.unpublish(repo, activeBranch, name)
      setCurrentElementType(result)
      showSuccess('Success', `Element type ${desired ? 'published' : 'unpublished'} successfully!`)
    } catch (err) {
      console.error('Failed to toggle element type publish', err)
      showError('Update Failed', 'Failed to toggle publish status')
    }
  }

  const selectedField = selectedPath
    ? getFieldByPath(elementType.fields, selectedPath)
    : undefined

  // Create archetype-compatible object for FieldCanvas
  const archetypeForCanvas = {
    name: elementType.name,
    fields: elementType.fields,
  }

  return (
    <DragPreviewProvider>
      <div className="h-full flex flex-col">
        {/* Toolbar */}
        <BuilderToolbar
          title={isNew ? 'New Element Type' : currentElementType?.name ?? name ?? 'Element Type'}
          icon={<Shapes className="w-5 h-5 text-primary-300" />}
          backLink={{ to: `/${repo}/elementtypes`, label: 'Element Types' }}
          status={
            !isNew && (
              isPublished ? (
                <span className="flex items-center gap-1 px-2 py-0.5 bg-green-500/20 text-green-400 text-xs rounded-full">
                  <CheckCircle className="w-3 h-3" /> Published
                </span>
              ) : (
                <span className="flex items-center gap-1 px-2 py-0.5 bg-zinc-500/20 text-zinc-400 text-xs rounded-full">
                  <XCircle className="w-3 h-3" /> Draft
                </span>
              )
            )
          }
          onSave={handleSave}
          saving={saving}
          canUndo={canUndo}
          canRedo={canRedo}
          onUndo={undo}
          onRedo={redo}
          extraActions={
            !isNew && (
              <button
                onClick={() => handlePublish(!isPublished)}
                className={`px-3 py-1.5 text-sm font-medium rounded transition-colors ${
                  isPublished
                    ? 'bg-red-500/20 text-red-300 hover:bg-red-500/30'
                    : 'bg-green-500/20 text-green-300 hover:bg-green-500/30'
                }`}
              >
                {isPublished ? 'Unpublish' : 'Publish'}
              </button>
            )
          }
        />

        {/* Main Content */}
        <div className="flex-1 min-h-0 flex flex-col">
          {/* Editor Tabs - Always visible */}
          <EditorTabs
            activeTab={editorMode}
            onTabChange={handleTabChange}
            error={yamlError}
          />

          {/* Content Area - Different layouts for Visual vs Source */}
          <div className="flex-1 min-h-0">
            {editorMode === 'visual' ? (
              /* Visual Mode - Allotment with Toolbox, Canvas, Properties */
              <Allotment
                onChange={(sizes) => {
                  if (sizes[0] !== undefined) setToolboxWidth(sizes[0])
                  if (sizes[2] !== undefined) setPropertiesWidth(sizes[2])
                }}
              >
                {/* Toolbox Panel */}
                <Allotment.Pane
                  preferredSize={preferences.toolboxWidth}
                  minSize={140}
                  maxSize={300}
                >
                  <div className="h-full bg-zinc-900/50 border-r border-white/10 overflow-hidden">
                    <FieldTypeToolbox onAddField={handleAddField} />
                  </div>
                </Allotment.Pane>

                {/* Canvas Panel */}
                <Allotment.Pane minSize={400}>
                  <div className="h-full bg-zinc-900/30 overflow-hidden">
                    <FieldCanvas
                      archetype={archetypeForCanvas}
                      selectedPath={selectedPath}
                      onPathSelect={setSelectedPath}
                      onPathDelete={handleFieldDelete}
                    />
                  </div>
                </Allotment.Pane>

                {/* Properties Panel */}
                <Allotment.Pane
                  preferredSize={preferences.propertiesWidth}
                  minSize={250}
                  maxSize={500}
                >
                  <div className="h-full bg-zinc-900/50 border-l border-white/10 overflow-hidden">
                    {selectedField ? (
                      <FieldEditorPanel
                        field={selectedField}
                        onChange={handleFieldChange}
                        onDelete={() => handleFieldDelete(selectedPath!)}
                        repo={repo}
                        branch={activeBranch}
                        currentElementTypeName={elementType.name}
                      />
                    ) : (
                      <ElementCoreSettingsPanel
                        elementType={elementType}
                        onChange={(updated) => setElementType(updated, 'Update settings')}
                        validationErrors={validationErrors}
                      />
                    )}
                  </div>
                </Allotment.Pane>
              </Allotment>
            ) : (
              /* Source Mode - Full width YAML editor */
              <div className="h-full bg-zinc-900/30">
                <YamlEditor
                  value={yamlContent}
                  onChange={(v) => setYamlContent(v ?? '')}
                  height="100%"
                />
              </div>
            )}
          </div>
        </div>
      </div>

      <DragOverlay />

      {pendingCommit && (
        <CommitDialog
          title={isNew ? 'Create Element Type' : 'Update Element Type'}
          action={
            pendingCommit.action === 'create'
              ? `Creating element type "${pendingCommit.elementType.name}"`
              : `Updating element type "${pendingCommit.targetName}"`
          }
          onCommit={executeCommit}
          onClose={() => setPendingCommit(null)}
        />
      )}

      <ToastContainer toasts={toasts} onClose={closeToast} />
    </DragPreviewProvider>
  )
}

// Wrapper that loads elementType and provides context
export default function ElementTypeEditor() {
  const { repo, branch, name } = useParams<{ repo: string; branch?: string; name?: string }>()
  const activeBranch = branch || 'main'
  const navigate = useNavigate()

  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [initialElementType, setInitialElementType] = useState<ElementTypeDefinition>(DEFAULT_ELEMENT_TYPE)

  useEffect(() => {
    if (!repo) return

    if (name) {
      loadElementType()
    } else {
      setInitialElementType(DEFAULT_ELEMENT_TYPE)
      setLoading(false)
    }

    async function loadElementType() {
      setLoading(true)
      setError(null)

      try {
        const data = await elementTypesApi.get(repo!, activeBranch, name!)
        const definition = parseYamlToElementType(yaml.dump(data, { indent: 2 }))
        setInitialElementType(definition)
      } catch (err) {
        console.error('Failed to load element type:', err)
        setError('Failed to load element type')
      } finally {
        setLoading(false)
      }
    }
  }, [repo, name, activeBranch])

  if (loading) {
    return (
      <div className="h-full flex items-center justify-center">
        <div className="text-zinc-400">Loading element type...</div>
      </div>
    )
  }

  if (error) {
    return (
      <div className="h-full flex items-center justify-center">
        <div className="text-center">
          <div className="text-red-400 mb-4">{error}</div>
          <button
            onClick={() => navigate(`/${repo}/elementtypes`)}
            className="px-4 py-2 bg-primary-500 text-white rounded hover:bg-primary-600"
          >
            Back to Element Types
          </button>
        </div>
      </div>
    )
  }

  return (
    <ElementTypeBuilderProvider
      initialElementType={initialElementType}
      onChange={() => {
        // State is managed internally by context
      }}
    >
      <ElementTypeEditorContent />
    </ElementTypeBuilderProvider>
  )
}
