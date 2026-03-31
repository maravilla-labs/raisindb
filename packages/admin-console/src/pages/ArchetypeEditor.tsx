/**
 * Archetype Editor Page
 *
 * IDE-style editor for archetypes with resizable panels,
 * Visual/YAML tabs, and undo/redo support.
 */

import { useEffect, useState } from 'react'
import { useNavigate, useParams } from 'react-router-dom'
import { CheckCircle, Sparkles, XCircle } from 'lucide-react'
import { Allotment } from 'allotment'
import 'allotment/dist/style.css'
import * as yaml from 'js-yaml'

import YamlEditor from '../components/YamlEditor'
import CommitDialog from '../components/CommitDialog'
import { useToast, ToastContainer } from '../components/Toast'
import {
  ArchetypeBuilderProvider,
  useArchetypeBuilderContext,
} from '../components/archetype-builder/ArchetypeBuilderContext'
import FieldTypeToolbox from '../components/archetype-builder/FieldTypeToolbox'
import FieldCanvas from '../components/archetype-builder/FieldCanvas'
import FieldEditorPanel from '../components/archetype-builder/FieldEditorPanel'
import CoreSettingsPanel from '../components/archetype-builder/CoreSettingsPanel'
import {
  parseYamlToArchetype,
  serializeArchetypeToYaml,
  addFieldIds,
  createNewField,
  validateArchetype,
} from '../components/archetype-builder/utils'
import { DEFAULT_ARCHETYPE } from '../components/archetype-builder/constants'
import type { ArchetypeDefinition, FieldSchema, FieldType } from '../components/archetype-builder/types'
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
  archetypesApi,
  type Archetype,
  type ArchetypeCommitPayload,
} from '../api/archetypes'

// Helper functions for path manipulation (moved from ArchetypeVisualBuilder)
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
  archetype: Archetype
  action: 'create' | 'update'
  targetName: string
}

// Inner component that uses context
function ArchetypeEditorContent() {
  const { repo, branch, name } = useParams<{ repo: string; branch?: string; name?: string }>()
  const activeBranch = branch || 'main'
  const navigate = useNavigate()

  const {
    archetype,
    setArchetype,
    selectedPath,
    setSelectedPath,
    undo,
    redo,
    canUndo,
    canRedo,
    preferences,
    setToolboxWidth,
    setPropertiesWidth,
  } = useArchetypeBuilderContext()

  const [editorMode, setEditorMode] = useState<EditorMode>('visual')
  const [yamlContent, setYamlContent] = useState<string>('')
  const [yamlError, setYamlError] = useState<string | null>(null)
  const [pendingCommit, setPendingCommit] = useState<PendingCommit | null>(null)
  const [saving, setSaving] = useState(false)
  const [currentArchetype, setCurrentArchetype] = useState<Archetype | null>(null)
  const [validationErrors, setValidationErrors] = useState<Record<string, string>>({})

  const { toasts, error: showError, success: showSuccess, closeToast } = useToast()

  const isNew = !name
  const isPublished = currentArchetype?.publishable ?? false

  // Validate on archetype changes
  useEffect(() => {
    const errors = validateArchetype(archetype)
    setValidationErrors(errors)
  }, [archetype])

  // Sync YAML when switching to source mode
  useEffect(() => {
    if (editorMode === 'source') {
      try {
        const yamlStr = serializeArchetypeToYaml(archetype)
        setYamlContent(yamlStr)
        setYamlError(null)
      } catch (err: any) {
        setYamlError(err.message || 'Failed to serialize')
      }
    }
  }, [editorMode, archetype])

  // Handle drop from DnD
  const handleDrop = (result: DropResult) => {
    const { source, targetPath, position } = result

    if (source.type === 'toolbox-item') {
      const fieldType = source.itemType as FieldType
      const newField = createNewField(fieldType)

      if (!targetPath) {
        const newFields = [...archetype.fields, newField]
        setArchetype({ ...archetype, fields: newFields }, `Add ${fieldType}`)
        setSelectedPath(newField.id!)
        return
      }

      const targetField = getFieldByPath(archetype.fields, targetPath)
      if (!targetField) {
        const newFields = [...archetype.fields, newField]
        setArchetype({ ...archetype, fields: newFields }, `Add ${fieldType}`)
        setSelectedPath(newField.id!)
        return
      }

      if (position === 'inside') {
        if (targetField.$type === 'CompositeField') {
          const { fields: newFields, newPath } = addNestedField(
            archetype.fields,
            targetPath,
            newField
          )
          setArchetype({ ...archetype, fields: newFields }, `Add ${fieldType} to composite`)
          if (newPath) setSelectedPath(newPath)
        }
        return
      }

      const parentPath = getParentPath(targetPath)
      const targetIdx = getIndexFromPath(targetPath)

      if (parentPath === null) {
        const topLevelIndex = archetype.fields.findIndex((f) => f.id === targetField.id)
        if (topLevelIndex === -1) return

        const insertIndex = position === 'before' ? topLevelIndex : topLevelIndex + 1
        const newFields = [
          ...archetype.fields.slice(0, insertIndex),
          newField,
          ...archetype.fields.slice(insertIndex),
        ]
        setArchetype({ ...archetype, fields: newFields }, `Add ${fieldType}`)
        setSelectedPath(newField.id!)
        return
      }

      if (targetIdx !== null) {
        const insertIndex = position === 'before' ? targetIdx : targetIdx + 1
        const { fields: newFields, newPath } = addNestedField(
          archetype.fields,
          parentPath,
          newField,
          insertIndex
        )
        setArchetype({ ...archetype, fields: newFields }, `Add ${fieldType}`)
        if (newPath) setSelectedPath(newPath)
      }
      return
    }

    if (source.type === 'builder-item') {
      const sourcePath = source.path
      if (!sourcePath) return

      const { fields: newFields, newPath } = moveField(
        archetype.fields,
        sourcePath,
        targetPath,
        position
      )

      setArchetype({ ...archetype, fields: newFields }, 'Move field')
      setSelectedPath(newPath)
    }
  }

  useBuilderDropMonitor({ onDrop: handleDrop })

  const handleAddField = (type: FieldType) => {
    const newField = createNewField(type)
    setArchetype({
      ...archetype,
      fields: [...archetype.fields, newField],
    }, `Add ${type}`)
    setSelectedPath(newField.id!)
  }

  const handleFieldChange = (updatedField: FieldSchema) => {
    if (!selectedPath) return

    const newFields = updateFieldByPath(
      archetype.fields,
      selectedPath,
      () => updatedField
    )
    setArchetype({ ...archetype, fields: newFields }, 'Update field')
  }

  const handleFieldDelete = (path: string) => {
    const newFields = deleteFieldByPath(archetype.fields, path)
    setArchetype({ ...archetype, fields: newFields }, 'Delete field')

    if (selectedPath === path || selectedPath?.startsWith(path + '.') || selectedPath?.startsWith(path + '[')) {
      setSelectedPath(undefined)
    }
  }

  const handleTabChange = (tab: EditorMode) => {
    if (tab === editorMode) return

    if (tab === 'visual') {
      try {
        const parsed = parseYamlToArchetype(yamlContent)
        setArchetype(parsed, 'Update from YAML')
        setEditorMode('visual')
        setYamlError(null)
      } catch (err: any) {
        setYamlError(err?.message || 'Failed to parse YAML')
      }
    } else {
      try {
        const yamlStr = serializeArchetypeToYaml(archetype)
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
      let archetypeToSave: Archetype

      if (editorMode === 'visual') {
        const yamlStr = serializeArchetypeToYaml(archetype)
        archetypeToSave = yaml.load(yamlStr) as Archetype
        setYamlContent(yamlStr)
      } else {
        archetypeToSave = yaml.load(yamlContent) as Archetype
      }

      if (!archetypeToSave || typeof archetypeToSave !== 'object') {
        throw new Error('YAML must define an archetype object')
      }
      if (!archetypeToSave.name || archetypeToSave.name.trim().length === 0) {
        throw new Error('Archetype name is required')
      }

      setPendingCommit({
        archetype: archetypeToSave,
        action: isNew ? 'create' : 'update',
        targetName: isNew ? archetypeToSave.name : name ?? archetypeToSave.name,
      })
    } catch (err: any) {
      console.error('Failed to prepare archetype for saving:', err)
      showError('Error', err.message || 'Failed to prepare archetype for saving')
    }
  }

  const executeCommit = async (message: string, actor: string) => {
    if (!pendingCommit || !repo) return

    const commit: ArchetypeCommitPayload = {
      message: message.trim(),
      actor: actor.trim() || undefined,
    }

    setSaving(true)

    try {
      let saved: Archetype

      if (pendingCommit.action === 'create') {
        saved = await archetypesApi.create(repo, activeBranch, pendingCommit.archetype, commit)
        showSuccess('Created', 'Archetype created successfully!')
        navigate(`/${repo}/archetypes/${saved.name}`)
      } else {
        saved = await archetypesApi.update(
          repo,
          activeBranch,
          pendingCommit.targetName,
          pendingCommit.archetype,
          commit
        )
        showSuccess('Updated', 'Archetype updated successfully!')
        setCurrentArchetype(saved)
      }

      setPendingCommit(null)
    } catch (err: any) {
      console.error('Failed to save archetype:', err)
      showError('Error', err.message || 'Failed to save archetype')
      throw err
    } finally {
      setSaving(false)
    }
  }

  const handlePublish = async (desired: boolean) => {
    if (!repo || !name) return
    try {
      const result = desired
        ? await archetypesApi.publish(repo, activeBranch, name)
        : await archetypesApi.unpublish(repo, activeBranch, name)
      setCurrentArchetype(result)
      showSuccess('Success', `Archetype ${desired ? 'published' : 'unpublished'} successfully!`)
    } catch (err) {
      console.error('Failed to toggle archetype publish', err)
      showError('Update Failed', 'Failed to toggle publish status')
    }
  }

  const selectedField = selectedPath
    ? getFieldByPath(archetype.fields, selectedPath)
    : undefined

  return (
    <DragPreviewProvider>
      <div className="h-full flex flex-col">
        {/* Toolbar */}
        <BuilderToolbar
          title={isNew ? 'New Archetype' : currentArchetype?.name ?? name ?? 'Archetype'}
          icon={<Sparkles className="w-5 h-5 text-primary-300" />}
          backLink={{ to: `/${repo}/archetypes`, label: 'Archetypes' }}
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
                      archetype={archetype}
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
                      />
                    ) : (
                      <CoreSettingsPanel
                        archetype={archetype}
                        onChange={(updated) => setArchetype(updated, 'Update settings')}
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
          title={isNew ? 'Create Archetype' : 'Update Archetype'}
          action={
            pendingCommit.action === 'create'
              ? `Creating new archetype "${pendingCommit.archetype.name}"`
              : `Updating archetype "${pendingCommit.targetName}"`
          }
          onCommit={executeCommit}
          onClose={() => setPendingCommit(null)}
        />
      )}

      <ToastContainer toasts={toasts} onClose={closeToast} />
    </DragPreviewProvider>
  )
}

// Wrapper that loads archetype and provides context
export default function ArchetypeEditor() {
  const { repo, branch, name } = useParams<{ repo: string; branch?: string; name?: string }>()
  const activeBranch = branch || 'main'
  const navigate = useNavigate()

  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [initialArchetype, setInitialArchetype] = useState<ArchetypeDefinition>(DEFAULT_ARCHETYPE)

  useEffect(() => {
    if (!repo) return

    if (name) {
      loadArchetype()
    } else {
      setInitialArchetype(DEFAULT_ARCHETYPE)
      setLoading(false)
    }

    async function loadArchetype() {
      setLoading(true)
      setError(null)

      try {
        const archetype = await archetypesApi.get(repo!, activeBranch, name!)
        const definition: ArchetypeDefinition = {
          ...archetype,
          fields: (archetype.fields || []).map(addFieldIds),
        }
        setInitialArchetype(definition)
      } catch (err) {
        console.error('Failed to load archetype:', err)
        setError('Failed to load archetype')
      } finally {
        setLoading(false)
      }
    }
  }, [repo, name, activeBranch])

  if (loading) {
    return (
      <div className="h-full flex items-center justify-center">
        <div className="text-zinc-400">Loading archetype...</div>
      </div>
    )
  }

  if (error) {
    return (
      <div className="h-full flex items-center justify-center">
        <div className="text-center">
          <div className="text-red-400 mb-4">{error}</div>
          <button
            onClick={() => navigate(`/${repo}/archetypes`)}
            className="px-4 py-2 bg-primary-500 text-white rounded hover:bg-primary-600"
          >
            Back to Archetypes
          </button>
        </div>
      </div>
    )
  }

  return (
    <ArchetypeBuilderProvider
      initialArchetype={initialArchetype}
      onChange={() => {
        // State is managed internally by context
        // onChange here is for external sync if needed
      }}
    >
      <ArchetypeEditorContent />
    </ArchetypeBuilderProvider>
  )
}
