/**
 * NodeType Editor Page
 *
 * IDE-style editor for node types with resizable panels,
 * Visual/YAML tabs, and undo/redo support.
 */

import { useEffect, useState } from 'react'
import { useNavigate, useParams } from 'react-router-dom'
import { CheckCircle, FileType, X, Eye, History, Clock } from 'lucide-react'
import { Allotment } from 'allotment'
import 'allotment/dist/style.css'
import * as yaml from 'js-yaml'

import YamlEditor from '../components/YamlEditor'
import CommitDialog from '../components/CommitDialog'
import { useToast, ToastContainer } from '../components/Toast'
import {
  NodeTypeBuilderProvider,
  useNodeTypeBuilderContext,
} from '../components/nodetype-builder/NodeTypeBuilderContext'
import PropertyTypeToolbox from '../components/nodetype-builder/PropertyTypeToolbox'
import PropertyCanvas from '../components/nodetype-builder/PropertyCanvas'
import PropertyEditorPanel from '../components/nodetype-builder/PropertyEditorPanel'
import CoreSettingsPanel from '../components/nodetype-builder/CoreSettingsPanel'
import {
  parseYamlToNodeType,
  serializeNodeTypeToYaml,
  createNewProperty,
  validateNodeType,
} from '../components/nodetype-builder/utils'
import type { NodeTypeDefinition, PropertyValueSchema, PropertyType } from '../components/nodetype-builder/types'
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
  nodeTypesApi,
  type NodeType,
  type ResolvedNodeType,
  type NodeTypeCommitPayload,
} from '../api/nodetypes'

// Helper functions for path manipulation
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

function getPropertyByPath(
  properties: PropertyValueSchema[],
  path: string
): PropertyValueSchema | undefined {
  const segments = parsePath(path)
  if (segments.length === 0) return undefined

  const first = segments[0]
  if (first.type !== 'key') return undefined

  let current = properties.find((p) => p.name === first.value)
  if (!current) return undefined

  for (let i = 1; i < segments.length; i++) {
    const seg = segments[i]
    if (seg.type === 'key') {
      if (!current.structure) return undefined
      current = current.structure[seg.value as string]
    } else {
      if (!Array.isArray(current.items)) return undefined
      current = current.items[seg.value as number]
    }
    if (!current) return undefined
  }

  return current
}

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

    const newProp = JSON.parse(JSON.stringify(prop)) as PropertyValueSchema
    let current = newProp

    for (let i = 1; i < segments.length - 1; i++) {
      const seg = segments[i]
      if (seg.type === 'key') {
        current = current.structure![seg.value as string]
      } else {
        current = (current.fields as PropertyValueSchema[])[seg.value as number]
      }
    }

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

function deletePropertyByPath(
  properties: PropertyValueSchema[],
  path: string
): PropertyValueSchema[] {
  const segments = parsePath(path)
  if (segments.length === 0) return properties

  const first = segments[0]
  if (first.type !== 'key') return properties

  if (segments.length === 1) {
    return properties.filter((p) => p.name !== first.value)
  }

  return properties.map((prop) => {
    if (prop.name !== first.value) return prop

    const newProp = JSON.parse(JSON.stringify(prop)) as PropertyValueSchema
    let current = newProp

    for (let i = 1; i < segments.length - 1; i++) {
      const seg = segments[i]
      if (seg.type === 'key') {
        current = current.structure![seg.value as string]
      } else {
        current = (current.fields as PropertyValueSchema[])[seg.value as number]
      }
    }

    const lastSeg = segments[segments.length - 1]
    if (lastSeg.type === 'key') {
      delete current.structure![lastSeg.value as string]
    } else {
      (current.fields as PropertyValueSchema[]).splice(lastSeg.value as number, 1)
    }

    return newProp
  })
}

function getParentPath(path: string): string | null {
  const segments = parsePath(path)
  if (segments.length <= 1) return null
  return reconstructPath(segments.slice(0, -1))
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

    const newProp = JSON.parse(JSON.stringify(prop)) as PropertyValueSchema
    let current = newProp

    for (let i = 1; i < segments.length; i++) {
      const seg = segments[i]
      if (seg.type === 'key') {
        current = current.structure![seg.value as string]
      } else {
        current = (current.fields as PropertyValueSchema[])[seg.value as number]
      }
    }

    if (current.type === 'Object') {
      if (!current.structure) current.structure = {}
      let propName = newProperty.name || `prop_${Date.now()}`
      let counter = 1
      while (current.structure[propName]) {
        propName = `${newProperty.name || 'prop'}_${counter++}`
      }
      current.structure[propName] = { ...newProperty, name: propName }
      resultPath = `${targetPath}.${propName}`
    } else if (current.type === 'Composite') {
      if (!Array.isArray(current.fields)) current.fields = []
      const insertIndex = index !== undefined ? index : current.fields.length
      current.fields.splice(insertIndex, 0, newProperty)
      resultPath = `${targetPath}[${insertIndex}]`
    }

    return newProp
  })

  return { properties: newProperties, newPath: resultPath }
}

function moveProperty(
  properties: PropertyValueSchema[],
  sourcePath: string,
  targetPath: string | null,
  position: 'before' | 'after' | 'inside',
  targetIndex?: number
): { properties: PropertyValueSchema[]; newPath: string } {
  const propertyToMove = getPropertyByPath(properties, sourcePath)
  if (!propertyToMove) return { properties, newPath: sourcePath }

  const clonedProperty = JSON.parse(JSON.stringify(propertyToMove)) as PropertyValueSchema
  let newProperties = deletePropertyByPath(properties, sourcePath)

  if (!targetPath) {
    const insertIndex = targetIndex !== undefined ? targetIndex : newProperties.length
    newProperties = [
      ...newProperties.slice(0, insertIndex),
      clonedProperty,
      ...newProperties.slice(insertIndex),
    ]
    return { properties: newProperties, newPath: clonedProperty.name! }
  }

  const targetProperty = getPropertyByPath(newProperties, targetPath)
  if (!targetProperty) return { properties: newProperties, newPath: clonedProperty.name! }

  const parentPath = getParentPath(targetPath)
  const targetIdx = getIndexFromPath(targetPath)

  if (position === 'inside') {
    if (targetProperty.type === 'Object' || targetProperty.type === 'Composite') {
      const { properties: updatedProperties, newPath } = addNestedProperty(
        newProperties,
        targetPath,
        clonedProperty,
        targetIndex
      )
      return { properties: updatedProperties, newPath }
    }
    return { properties: newProperties, newPath: clonedProperty.name! }
  }

  if (parentPath === null) {
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

type PendingCommit = {
  nodeType: NodeType
  action: 'create' | 'update'
  targetName: string
}

// Default template for new node type
const DEFAULT_NODE_TYPE: NodeTypeDefinition = {
  name: '',
  extends: 'raisin:Folder',
  properties: [
    {
      id: `prop_${Date.now()}`,
      name: 'title',
      type: 'String',
      required: true,
    },
  ],
  allowed_children: ['*'],
}

// Inner component that uses context
function NodeTypeEditorContent() {
  const { repo, branch, name } = useParams<{ repo: string; branch?: string; name: string }>()
  const activeBranch = branch || 'main'
  const navigate = useNavigate()

  const {
    nodeType,
    setNodeType,
    selectedPath,
    setSelectedPath,
    undo,
    redo,
    canUndo,
    canRedo,
    preferences,
    setToolboxWidth,
    setPropertiesWidth,
  } = useNodeTypeBuilderContext()

  const [editorMode, setEditorMode] = useState<EditorMode>('visual')
  const [yamlContent, setYamlContent] = useState<string>('')
  const [yamlError, setYamlError] = useState<string | null>(null)
  const [pendingCommit, setPendingCommit] = useState<PendingCommit | null>(null)
  const [saving, setSaving] = useState(false)
  const [currentNodeType, setCurrentNodeType] = useState<NodeType | null>(null)
  const [validationErrors, setValidationErrors] = useState<Record<string, string>>({})
  const [showResolved, setShowResolved] = useState(false)
  const [resolved, setResolved] = useState<ResolvedNodeType | null>(null)

  const { toasts, error: showError, success: showSuccess, closeToast } = useToast()

  const isNew = !name
  const isPublished = currentNodeType?.published ?? false

  // Validate on nodeType changes
  useEffect(() => {
    const errors = validateNodeType(nodeType)
    setValidationErrors(errors)
  }, [nodeType])

  // Sync YAML when switching to source mode
  useEffect(() => {
    if (editorMode === 'source') {
      try {
        const yamlStr = serializeNodeTypeToYaml(nodeType)
        setYamlContent(yamlStr)
        setYamlError(null)
      } catch (err: any) {
        setYamlError(err.message || 'Failed to serialize')
      }
    }
  }, [editorMode, nodeType])

  // Handle drop from DnD
  const handleDrop = (result: DropResult) => {
    const { source, targetPath, position } = result

    if (source.type === 'toolbox-item') {
      const propertyType = source.itemType as PropertyType
      const newProperty = createNewProperty(propertyType)

      if (!targetPath) {
        const newProperties = [...nodeType.properties, newProperty]
        setNodeType({ ...nodeType, properties: newProperties }, `Add ${propertyType}`)
        setSelectedPath(newProperty.name)
        return
      }

      const targetProperty = getPropertyByPath(nodeType.properties, targetPath)
      if (!targetProperty) {
        const newProperties = [...nodeType.properties, newProperty]
        setNodeType({ ...nodeType, properties: newProperties }, `Add ${propertyType}`)
        setSelectedPath(newProperty.name)
        return
      }

      if (position === 'inside') {
        if (targetProperty.type === 'Object' || targetProperty.type === 'Composite') {
          const { properties: newProperties, newPath } = addNestedProperty(
            nodeType.properties,
            targetPath,
            newProperty
          )
          setNodeType({ ...nodeType, properties: newProperties }, `Add ${propertyType}`)
          if (newPath) setSelectedPath(newPath)
        }
        return
      }

      const parentPath = getParentPath(targetPath)
      const targetIdx = getIndexFromPath(targetPath)

      if (parentPath === null) {
        const topLevelIndex = nodeType.properties.findIndex((p) => p.name === targetProperty.name)
        if (topLevelIndex === -1) return

        const insertIndex = position === 'before' ? topLevelIndex : topLevelIndex + 1
        const newProperties = [
          ...nodeType.properties.slice(0, insertIndex),
          newProperty,
          ...nodeType.properties.slice(insertIndex),
        ]
        setNodeType({ ...nodeType, properties: newProperties }, `Add ${propertyType}`)
        setSelectedPath(newProperty.name)
        return
      }

      if (targetIdx !== null) {
        const insertIndex = position === 'before' ? targetIdx : targetIdx + 1
        const { properties: newProperties, newPath } = addNestedProperty(
          nodeType.properties,
          parentPath,
          newProperty,
          insertIndex
        )
        setNodeType({ ...nodeType, properties: newProperties }, `Add ${propertyType}`)
        if (newPath) setSelectedPath(newPath)
      }
      return
    }

    if (source.type === 'builder-item') {
      const sourcePath = source.path
      if (!sourcePath) return

      const { properties: newProperties, newPath } = moveProperty(
        nodeType.properties,
        sourcePath,
        targetPath,
        position
      )

      setNodeType({ ...nodeType, properties: newProperties }, 'Move property')
      setSelectedPath(newPath)
    }
  }

  useBuilderDropMonitor({ onDrop: handleDrop })

  const handleAddProperty = (type: PropertyType) => {
    const newProperty = createNewProperty(type)
    setNodeType({
      ...nodeType,
      properties: [...nodeType.properties, newProperty],
    }, `Add ${type}`)
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

    setNodeType({ ...nodeType, properties: newProperties }, 'Update property')
  }

  const handlePropertyDelete = (path: string) => {
    const newProperties = deletePropertyByPath(nodeType.properties, path)
    setNodeType({ ...nodeType, properties: newProperties }, 'Delete property')

    if (selectedPath === path || selectedPath?.startsWith(path + '.') || selectedPath?.startsWith(path + '[')) {
      setSelectedPath(undefined)
    }
  }

  const handleTabChange = (tab: EditorMode) => {
    if (tab === editorMode) return

    if (tab === 'visual') {
      try {
        const parsed = parseYamlToNodeType(yamlContent)
        setNodeType(parsed, 'Update from YAML')
        setEditorMode('visual')
        setYamlError(null)
      } catch (err: any) {
        setYamlError(err?.message || 'Failed to parse YAML')
      }
    } else {
      try {
        const yamlStr = serializeNodeTypeToYaml(nodeType)
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
      let nodeTypeToSave: NodeType

      if (editorMode === 'visual') {
        const yamlStr = serializeNodeTypeToYaml(nodeType)
        nodeTypeToSave = yaml.load(yamlStr) as NodeType
        setYamlContent(yamlStr)
      } else {
        nodeTypeToSave = yaml.load(yamlContent) as NodeType
      }

      if (!nodeTypeToSave || typeof nodeTypeToSave !== 'object') {
        throw new Error('YAML must define a node type object')
      }
      if (!nodeTypeToSave.name || nodeTypeToSave.name.trim().length === 0) {
        throw new Error('Node type name is required')
      }

      setPendingCommit({
        nodeType: nodeTypeToSave,
        action: isNew ? 'create' : 'update',
        targetName: isNew ? nodeTypeToSave.name : name ?? nodeTypeToSave.name,
      })
    } catch (err: any) {
      console.error('Failed to prepare node type for saving:', err)
      showError('Error', err.message || 'Failed to prepare node type for saving')
    }
  }

  const executeCommit = async (message: string, actor: string) => {
    if (!pendingCommit || !repo) return

    const commit: NodeTypeCommitPayload = {
      message: message.trim(),
      actor: actor.trim() || undefined,
    }

    setSaving(true)

    try {
      let saved: NodeType

      if (pendingCommit.action === 'create') {
        saved = await nodeTypesApi.create(repo, activeBranch, pendingCommit.nodeType, commit)
        showSuccess('Created', 'Node type created successfully!')
        navigate(`/${repo}/nodetypes/${saved.name}`)
      } else {
        saved = await nodeTypesApi.update(
          repo,
          activeBranch,
          pendingCommit.targetName,
          pendingCommit.nodeType,
          commit
        )
        showSuccess('Updated', 'Node type updated successfully!')
        setCurrentNodeType(saved)
      }

      setPendingCommit(null)
    } catch (err: any) {
      console.error('Failed to save node type:', err)
      showError('Error', err.message || 'Failed to save node type')
      throw err
    } finally {
      setSaving(false)
    }
  }

  const loadResolved = async () => {
    if (!name || !repo) return
    try {
      const data = await nodeTypesApi.getResolved(repo, activeBranch, name)
      setResolved(data)
      setShowResolved(true)
    } catch (error) {
      console.error('Failed to load resolved node type:', error)
      showError('Load Failed', 'Failed to load resolved node type')
    }
  }

  const selectedProperty = selectedPath
    ? getPropertyByPath(nodeType.properties, selectedPath)
    : undefined

  return (
    <DragPreviewProvider>
      <div className="h-full flex flex-col">
        {/* Toolbar */}
        <BuilderToolbar
          title={isNew ? 'New Node Type' : currentNodeType?.name ?? name ?? 'Node Type'}
          icon={<FileType className="w-5 h-5 text-primary-300" />}
          backLink={{ to: `/${repo}/nodetypes`, label: 'Node Types' }}
          status={
            <>
              {currentNodeType?.version && (
                <span className="flex items-center gap-1 px-2 py-0.5 bg-blue-500/20 text-blue-400 text-xs rounded-full">
                  <History className="w-3 h-3" /> v{currentNodeType.version}
                </span>
              )}
              {isPublished && (
                <span className="flex items-center gap-1 px-2 py-0.5 bg-green-500/20 text-green-400 text-xs rounded-full">
                  <CheckCircle className="w-3 h-3" /> Published
                </span>
              )}
            </>
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
                onClick={loadResolved}
                className="flex items-center gap-1.5 px-3 py-1.5 text-sm font-medium rounded bg-secondary-500/20 text-secondary-300 hover:bg-secondary-500/30 transition-colors"
              >
                <Eye className="w-4 h-4" />
                Resolved
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
                    <PropertyTypeToolbox onAddProperty={handleAddProperty} />
                  </div>
                </Allotment.Pane>

                {/* Canvas Panel */}
                <Allotment.Pane minSize={400}>
                  <div className="h-full bg-zinc-900/30 overflow-hidden">
                    <PropertyCanvas
                      nodeType={nodeType}
                      selectedPath={selectedPath}
                      onPathSelect={setSelectedPath}
                      onPathDelete={handlePropertyDelete}
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
                        onChange={(updated) => setNodeType(updated, 'Update settings')}
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

        {/* Metadata info */}
        {currentNodeType && (
          <div className="flex-shrink-0 px-4 py-1 bg-black/20 border-t border-white/5 flex items-center gap-4 text-xs text-zinc-500">
            {currentNodeType.created_at && (
              <span className="flex items-center gap-1">
                <Clock className="w-3 h-3" />
                Created: {new Date(currentNodeType.created_at).toLocaleString()}
              </span>
            )}
            {currentNodeType.updated_at && (
              <span className="flex items-center gap-1">
                <Clock className="w-3 h-3" />
                Updated: {new Date(currentNodeType.updated_at).toLocaleString()}
              </span>
            )}
          </div>
        )}
      </div>

      <DragOverlay />

      {pendingCommit && (
        <CommitDialog
          title={isNew ? 'Create Node Type' : 'Update Node Type'}
          action={
            pendingCommit.action === 'create'
              ? `Creating new node type "${pendingCommit.nodeType.name}"`
              : `Updating node type "${pendingCommit.targetName}"`
          }
          onCommit={executeCommit}
          onClose={() => setPendingCommit(null)}
        />
      )}

      {/* Resolved View Modal */}
      {showResolved && resolved && (
        <div className="fixed inset-0 bg-black/50 backdrop-blur-sm flex items-center justify-center p-8 z-50">
          <div className="glass-dark rounded-xl max-w-4xl w-full max-h-[90vh] overflow-auto p-6">
            <div className="flex justify-between items-start mb-6">
              <h2 className="text-2xl font-bold text-white">Resolved Node Type</h2>
              <button
                onClick={() => setShowResolved(false)}
                className="p-2 hover:bg-white/10 rounded-lg transition-colors"
              >
                <X className="w-6 h-6 text-zinc-400" />
              </button>
            </div>

            <div className="space-y-6">
              <div>
                <h3 className="text-lg font-semibold text-white mb-2">Inheritance Chain</h3>
                <div className="flex gap-2 flex-wrap">
                  {resolved.inheritance_chain.map((type, idx) => (
                    <span key={type} className="px-3 py-1 bg-primary-500/20 text-primary-300 rounded-full text-sm">
                      {type}
                      {idx < resolved.inheritance_chain.length - 1 && ' →'}
                    </span>
                  ))}
                </div>
              </div>

              <div>
                <h3 className="text-lg font-semibold text-white mb-2">Resolved Properties</h3>
                <YamlEditor
                  value={yaml.dump(resolved.resolved_properties, { indent: 2 })}
                  onChange={() => {}}
                  readOnly
                  height="300px"
                />
              </div>

              <div>
                <h3 className="text-lg font-semibold text-white mb-2">Allowed Children</h3>
                <div className="flex gap-2 flex-wrap">
                  {resolved.resolved_allowed_children.map((child) => (
                    <span key={child} className="px-3 py-1 bg-green-500/20 text-green-300 rounded-full text-sm">
                      {child}
                    </span>
                  ))}
                </div>
              </div>

              <div>
                <h3 className="text-lg font-semibold text-white mb-2">Full Definition</h3>
                <YamlEditor
                  value={yaml.dump(resolved.node_type, { indent: 2 })}
                  onChange={() => {}}
                  readOnly
                  height="400px"
                />
              </div>
            </div>
          </div>
        </div>
      )}

      <ToastContainer toasts={toasts} onClose={closeToast} />
    </DragPreviewProvider>
  )
}

// Wrapper that loads nodeType and provides context
export default function NodeTypeEditor() {
  const { repo, branch, name } = useParams<{ repo: string; branch?: string; name: string }>()
  const activeBranch = branch || 'main'
  const navigate = useNavigate()

  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [initialNodeType, setInitialNodeType] = useState<NodeTypeDefinition>(DEFAULT_NODE_TYPE)

  useEffect(() => {
    if (!repo) return

    if (name) {
      loadNodeType()
    } else {
      setInitialNodeType(DEFAULT_NODE_TYPE)
      setLoading(false)
    }

    async function loadNodeType() {
      setLoading(true)
      setError(null)

      try {
        const data = await nodeTypesApi.get(repo!, activeBranch, name!)
        const definition = parseYamlToNodeType(yaml.dump(data, { indent: 2 }))
        setInitialNodeType(definition)
      } catch (err) {
        console.error('Failed to load node type:', err)
        setError('Failed to load node type')
      } finally {
        setLoading(false)
      }
    }
  }, [repo, name, activeBranch])

  if (loading) {
    return (
      <div className="h-full flex items-center justify-center">
        <div className="text-zinc-400">Loading node type...</div>
      </div>
    )
  }

  if (error) {
    return (
      <div className="h-full flex items-center justify-center">
        <div className="text-center">
          <div className="text-red-400 mb-4">{error}</div>
          <button
            onClick={() => navigate(`/${repo}/nodetypes`)}
            className="px-4 py-2 bg-primary-500 text-white rounded hover:bg-primary-600"
          >
            Back to Node Types
          </button>
        </div>
      </div>
    )
  }

  return (
    <NodeTypeBuilderProvider
      initialNodeType={initialNodeType}
      onChange={() => {
        // State is managed internally by context
      }}
    >
      <NodeTypeEditorContent />
    </NodeTypeBuilderProvider>
  )
}
