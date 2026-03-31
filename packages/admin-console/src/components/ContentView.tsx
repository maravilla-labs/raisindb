import { useState, useEffect, useMemo } from 'react'
import { Link } from 'react-router-dom'
import {
  FileText, Folder, Calendar, User, Hash, Type,
  ToggleLeft, Link2, Package, Edit3,
  Plus, Copy, Move, Trash2,
  Eye, EyeOff, AlertCircle, Layout,
  SquareStack, Layers, Puzzle
} from 'lucide-react'
import NodeTypeAwareEditor from './NodeTypeAwareEditor'
import { getSchemaType } from '../utils/propertySchema'
import CreateNodeDialog from './CreateNodeDialog'
import CopyNodeModal from './CopyNodeModal'
import MoveNodeModal from './MoveNodeModal'
import VersionHistory from './VersionHistory'
import AuditLog from './AuditLog'
import NodeOperationHistory from './NodeOperationHistory'
import { RelationshipManager } from './RelationshipManager'
import type { Node, CreateNodeRequest } from '../api/nodes'
import { nodeTypesApi, type ResolvedNodeType, type NodeType as NodeTypeDef } from '../api/nodetypes'
import { archetypesApi, type ResolvedArchetype } from '../api/archetypes'
import { translationsApi } from '../api/translations'

interface ContentViewProps {
  repo: string
  branch: string
  workspace: string
  node: Node | null
  allNodes: Node[]
  showEditor?: boolean
  currentLocale?: string | null
  onUpdate: (node: Partial<Node>) => Promise<void>
  onDelete: (node: Node) => void
  onPublish: (node: Node) => void
  onUnpublish: (node: Node) => void
  onCreateChild: (parent: Node | null, nodeData: CreateNodeRequest) => Promise<void>
  onCopy: (node: Node, destination: string, newName?: string, recursive?: boolean) => Promise<void>
  onMove: (node: Node, destination: string) => Promise<void>
  onCloseEditor?: () => void
  onTranslationUpdate?: () => void
  readonly?: boolean
}

export default function ContentView({
  repo,
  branch,
  workspace,
  node,
  allNodes,
  showEditor = false,
  currentLocale,
  onUpdate,
  onDelete,
  onPublish,
  onUnpublish,
  onCreateChild,
  onCopy,
  onMove,
  onCloseEditor,
  onTranslationUpdate,
  readonly = false
}: ContentViewProps) {
  const [editing, setEditing] = useState(false)
  const [nodeType, setNodeType] = useState<ResolvedNodeType | null>(null)
  const [resolvedArchetype, setResolvedArchetype] = useState<ResolvedArchetype | null>(null)
  const [showCreateDialog, setShowCreateDialog] = useState(false)
  const [showCopyModal, setShowCopyModal] = useState(false)
  const [showMoveModal, setShowMoveModal] = useState(false)
  const [defaultLanguage, setDefaultLanguage] = useState<string>('en')
  const [mixinDefs, setMixinDefs] = useState<Map<string, NodeTypeDef>>(new Map())

  useEffect(() => {
    if (showEditor) {
      if (node) {
        setEditing(true)
      } else {
        // Creating a root node
        setShowCreateDialog(true)
      }
    }
  }, [showEditor, node])

  useEffect(() => {
    if (node?.node_type) {
      loadNodeType(node.node_type)
    } else {
      setNodeType(null)
    }
  }, [node?.node_type])

  useEffect(() => {
    if (node?.archetype) {
      archetypesApi.getResolved(repo, branch, node.archetype)
        .then(setResolvedArchetype)
        .catch(err => {
          console.error('Failed to load resolved archetype:', err)
          setResolvedArchetype(null)
        })
    } else {
      setResolvedArchetype(null)
    }
  }, [node?.archetype, repo, branch])

  useEffect(() => {
    // Load translation config to get default language
    translationsApi.getConfig(repo).then(config => {
      setDefaultLanguage(config.default_language)
    }).catch(err => {
      console.error('Failed to load translation config:', err)
    })
  }, [repo])

  async function loadNodeType(typeName: string) {
    try {
      const resolved = await nodeTypesApi.getResolved(repo, branch, typeName, workspace)
      setNodeType(resolved)
    } catch (error) {
      console.error('Failed to load node type:', error)
      setNodeType(null)
    }
  }

  // Fetch mixin definitions when the node type has mixins
  const mixins = nodeType?.node_type?.mixins || []
  useEffect(() => {
    if (mixins.length === 0) {
      setMixinDefs(new Map())
      return
    }
    let cancelled = false
    async function fetchMixins() {
      const defs = new Map<string, NodeTypeDef>()
      await Promise.all(
        mixins.map(async (name) => {
          try {
            const def = await nodeTypesApi.get(repo, branch, name)
            if (!cancelled) defs.set(name, def)
          } catch (err) {
            console.error(`Failed to fetch mixin ${name}:`, err)
          }
        })
      )
      if (!cancelled) setMixinDefs(defs)
    }
    fetchMixins()
    return () => { cancelled = true }
  }, [repo, branch, mixins.join(',')])

  // Build a map of property name -> source mixin name
  const propertySourceMap = useMemo(() => {
    const map = new Map<string, string>()
    for (const mixinName of mixins) {
      const def = mixinDefs.get(mixinName)
      if (!def?.properties) continue
      const props = Array.isArray(def.properties) ? def.properties : []
      for (const p of props) {
        if (p.name && !map.has(p.name)) {
          map.set(p.name, mixinName)
        }
      }
    }
    return map
  }, [mixins, mixinDefs])

  function getNodeIcon(nodeTypeName?: string) {
    if (!nodeTypeName) return <FileText className="w-5 h-5" />

    // Map common node types to icons
    if (nodeTypeName.includes('Folder')) return <Folder className="w-5 h-5 text-amber-400" />
    if (nodeTypeName.includes('Page')) return <FileText className="w-5 h-5 text-secondary-400" />
    if (nodeTypeName.includes('Asset')) return <Package className="w-5 h-5 text-green-400" />
    if (nodeTypeName.includes('User')) return <User className="w-5 h-5 text-primary-400" />

    return <FileText className="w-5 h-5 text-zinc-400" />
  }

  function getPropertyIcon(propertyType: string) {
    switch (propertyType?.toLowerCase()) {
      case 'string': return <Type className="w-4 h-4 text-secondary-400" />
      case 'number': return <Hash className="w-4 h-4 text-green-400" />
      case 'boolean': return <ToggleLeft className="w-4 h-4 text-primary-400" />
      case 'date': return <Calendar className="w-4 h-4 text-amber-400" />
      case 'reference': return <Link2 className="w-4 h-4 text-accent-400" />
      case 'url': return <Link2 className="w-4 h-4 text-accent-400" />
      case 'resource': return <Package className="w-4 h-4 text-green-400" />
      case 'nodetype': return <SquareStack className="w-4 h-4 text-blue-400" />
      case 'element': return <FileText className="w-4 h-4 text-purple-400" />
      case 'composite': return <Layout className="w-4 h-4 text-purple-400" />
      default: return <FileText className="w-4 h-4 text-zinc-400" />
    }
  }

  function getPublishState(node: Node): 'unpublished' | 'published' | 'draft' {
    if (!node.published_at) return 'unpublished'

    // No updated_at means published immediately after creation
    if (!node.updated_at) return 'published'

    const pubTime = new Date(node.published_at).getTime()
    const updTime = new Date(node.updated_at).getTime()

    // If updated after published, there are draft changes
    return updTime > pubTime ? 'draft' : 'published'
  }

  async function handleSave(updatedNode: Partial<Node>) {
    await onUpdate(updatedNode)
    setEditing(false)
    if (onCloseEditor) onCloseEditor()
  }

  async function handleCreateChild(nodeData: CreateNodeRequest) {
    await onCreateChild(node, nodeData)
    setShowCreateDialog(false)
    if (onCloseEditor) onCloseEditor()
  }

  if (!node) {
    return (
      <div className="h-full flex items-center justify-center bg-black/20 backdrop-blur-sm">
        {showCreateDialog ? (
          <CreateNodeDialog
            repo={repo}
            branch={branch}
            workspace={workspace}
            parentPath=""
            parentName="Root"
            allowedChildren={undefined} // Allow all node types at root
            onClose={() => {
              setShowCreateDialog(false)
              if (onCloseEditor) onCloseEditor()
            }}
            onCreate={handleCreateChild}
          />
        ) : (
          <div className="text-center">
            <Folder className="w-16 h-16 text-zinc-600 mx-auto mb-4" />
            <h3 className="text-xl font-semibold text-zinc-400 mb-2">No Node Selected</h3>
            <p className="text-zinc-500">Select a node from the tree to view its details</p>
          </div>
        )}
      </div>
    )
  }

  return (
    <div className="h-full flex flex-col bg-black/20 backdrop-blur-sm">
      {/* Node header */}
      <div className="flex-shrink-0 bg-gradient-to-r from-primary-950/30 to-secondary-950/30 border-b border-white/10">
        <div className="px-6 py-4">
          <div className="flex items-start justify-between">
            <div className="flex items-start gap-3">
              <div className="p-2 bg-white/10 rounded-lg">
                {getNodeIcon(node.node_type)}
              </div>
              <div>
                <h2 className="text-2xl font-bold text-white">{node.name}</h2>
                <p className="text-sm text-zinc-400 mt-1">{node.path}</p>
                <div className="flex items-center gap-2 mt-2 flex-wrap">
                  <span className="inline-flex items-center gap-1 px-2 py-1 bg-primary-500/20 text-primary-300 rounded text-xs">
                    {node.node_type}
                  </span>
                  {node.archetype && (
                    <span className="inline-flex items-center gap-1 px-2 py-1 bg-purple-500/20 text-purple-300 rounded text-xs">
                      <Layers className="w-3 h-3" />
                      {node.archetype}
                    </span>
                  )}
                  {mixins.map((mixin) => (
                    <span key={mixin} className="inline-flex items-center gap-1 px-2 py-1 bg-teal-500/20 text-teal-300 rounded text-xs">
                      <Puzzle className="w-3 h-3" />
                      {mixin}
                    </span>
                  ))}
                  {node.updated_at && (
                    <span className="text-xs text-zinc-500">
                      Updated: {new Date(node.updated_at).toLocaleDateString()}
                    </span>
                  )}
                </div>
              </div>
            </div>

            {/* Action buttons */}
            <div className="flex items-center gap-2">
              {!editing && !readonly && (
                <>
                  <button
                    onClick={() => setEditing(true)}
                    className="p-2 bg-primary-500/20 hover:bg-primary-500/30 text-primary-400 rounded-lg transition-colors"
                    title="Edit properties"
                  >
                    <Edit3 className="w-4 h-4" />
                  </button>
                  <button
                    onClick={() => setShowCreateDialog(true)}
                    className="p-2 bg-green-500/20 hover:bg-green-500/30 text-green-400 rounded-lg transition-colors"
                    title="Add child"
                  >
                    <Plus className="w-4 h-4" />
                  </button>
                  <button
                    onClick={() => setShowCopyModal(true)}
                    className="p-2 bg-secondary-500/20 hover:bg-secondary-500/30 text-secondary-400 rounded-lg transition-colors"
                    title="Copy"
                  >
                    <Copy className="w-4 h-4" />
                  </button>
                  <button
                    onClick={() => setShowMoveModal(true)}
                    className="p-2 bg-accent-500/20 hover:bg-accent-500/30 text-accent-400 rounded-lg transition-colors"
                    title="Move"
                  >
                    <Move className="w-4 h-4" />
                  </button>

                  {/* Three-state publish button (only if nodetype is publishable) */}
                  {nodeType?.node_type.publishable && (() => {
                    const publishState = getPublishState(node)

                    if (publishState === 'unpublished') {
                      // Red state: Never published
                      return (
                        <button
                          onClick={() => onPublish(node)}
                          className="p-2 bg-red-500/20 hover:bg-red-500/30 text-red-400 rounded-lg transition-colors"
                          title="Publish (Unpublished)"
                        >
                          <Eye className="w-4 h-4" />
                        </button>
                      )
                    } else if (publishState === 'draft') {
                      // Orange state: Published but has draft changes
                      return (
                        <button
                          onClick={() => onPublish(node)}
                          className="p-2 bg-orange-500/20 hover:bg-orange-500/30 text-orange-400 rounded-lg transition-colors relative"
                          title="Publish Changes (Draft)"
                        >
                          <Eye className="w-4 h-4" />
                          <AlertCircle className="w-2 h-2 absolute top-0 right-0 text-orange-400" />
                        </button>
                      )
                    } else {
                      // Green state: Published and up-to-date
                      return (
                        <button
                          onClick={() => onUnpublish(node)}
                          className="p-2 bg-green-500/20 hover:bg-green-500/30 text-green-400 rounded-lg transition-colors"
                          title="Unpublish (Published)"
                        >
                          <EyeOff className="w-4 h-4" />
                        </button>
                      )
                    }
                  })()}

                  <button
                    onClick={() => onDelete(node)}
                    className="p-2 bg-red-500/20 hover:bg-red-500/30 text-red-400 rounded-lg transition-colors"
                    title="Delete"
                  >
                    <Trash2 className="w-4 h-4" />
                  </button>
                </>
              )}
            </div>
          </div>
        </div>
      </div>

      {/* Content area */}
      <div className="flex-1 overflow-auto">
        {editing ? (
          <NodeTypeAwareEditor
            node={node}
            nodeType={nodeType}
            resolvedArchetype={resolvedArchetype}
            repo={repo}
            branch={branch}
            workspace={workspace}
            currentLocale={currentLocale}
            defaultLanguage={defaultLanguage}
            onSave={handleSave}
            onCancel={() => {
              setEditing(false)
              if (onCloseEditor) onCloseEditor()
              if (onTranslationUpdate) onTranslationUpdate()
            }}
          />
        ) : (
          <div className="p-6 space-y-6">
            {/* Properties section */}
            <div className="bg-white/5 rounded-xl p-6">
              <h3 className="text-lg font-semibold text-white mb-4">Properties</h3>
              {node.properties && Object.keys(node.properties).length > 0 ? (
                <div className="space-y-4">
                  {(() => {
                    const nodeProps = node.properties!

                    // Get property keys in order defined by node type
                    const propertyKeys = nodeType?.resolved_properties
                      ? nodeType.resolved_properties.map((propDef: any) => propDef.name)
                      : Object.keys(nodeProps)

                    // Filter to only show properties that exist on the node
                    const orderedKeys = propertyKeys.filter((key: string) => key in nodeProps)

                    // Add any properties not in the schema (shouldn't happen, but be defensive)
                    const schemaKeys = new Set(propertyKeys)
                    Object.keys(nodeProps).forEach(key => {
                      if (!schemaKeys.has(key)) {
                        orderedKeys.push(key)
                      }
                    })

                    const renderProp = (key: string) => {
                      const value = nodeProps[key]
                      return (
                        <div key={key} className="flex items-start gap-3">
                          <div className="p-2 bg-white/5 rounded">
                            {getPropertyIcon(typeof value)}
                          </div>
                          <div className="flex-1">
                            <div className="text-sm text-zinc-400">{key}</div>
                            <div className="text-white mt-1">
                              {typeof value === 'object' ? JSON.stringify(value, null, 2) : String(value)}
                            </div>
                          </div>
                        </div>
                      )
                    }

                    // Group properties by source when mixins are present
                    if (mixins.length > 0 && propertySourceMap.size > 0) {
                      const ownKeys = orderedKeys.filter(k => !propertySourceMap.has(k))
                      const mixinGroups = new Map<string, string[]>()
                      for (const key of orderedKeys) {
                        const source = propertySourceMap.get(key)
                        if (source) {
                          const group = mixinGroups.get(source) || []
                          group.push(key)
                          mixinGroups.set(source, group)
                        }
                      }

                      return (
                        <>
                          {ownKeys.length > 0 && (
                            <div>
                              <h4 className="text-xs font-medium text-zinc-500 uppercase tracking-wider mb-2">Own Properties</h4>
                              <div className="space-y-3">{ownKeys.map(renderProp)}</div>
                            </div>
                          )}
                          {Array.from(mixinGroups.entries()).map(([source, keys]) => (
                            <div key={source}>
                              <h4 className="text-xs font-medium text-teal-400/70 uppercase tracking-wider mb-2 flex items-center gap-1.5">
                                <Puzzle className="w-3 h-3" />
                                From {source}
                              </h4>
                              <div className="space-y-3">{keys.map(renderProp)}</div>
                            </div>
                          ))}
                        </>
                      )
                    }

                    return orderedKeys.map(renderProp)
                  })()}
                </div>
              ) : (
                <p className="text-zinc-500">No properties defined</p>
              )}
            </div>

            {/* Node Type Info */}
            {nodeType && (
              <div className="bg-white/5 rounded-xl p-6">
                <h3 className="text-lg font-semibold text-white mb-4">Node Type Definition</h3>

                {/* Mixins */}
                {mixins.length > 0 && (
                  <div className="mb-4">
                    <h4 className="text-sm font-medium text-zinc-400 mb-2">Mixins</h4>
                    <div className="flex flex-wrap gap-2">
                      {mixins.map((mixin) => (
                        <span key={mixin} className="inline-flex items-center gap-1.5 px-3 py-1 bg-teal-500/20 text-teal-300 rounded-full text-sm">
                          <Puzzle className="w-3.5 h-3.5" />
                          {mixin}
                        </span>
                      ))}
                    </div>
                  </div>
                )}

                {/* Allowed children */}
                {nodeType.resolved_allowed_children && nodeType.resolved_allowed_children.length > 0 && (
                  <div className="mb-4">
                    <h4 className="text-sm font-medium text-zinc-400 mb-2">Allowed Children</h4>
                    <div className="flex flex-wrap gap-2">
                      {nodeType.resolved_allowed_children.map((childType) => (
                        <span key={childType} className="px-3 py-1 bg-secondary-500/20 text-secondary-300 rounded-full text-sm">
                          {childType}
                        </span>
                      ))}
                    </div>
                  </div>
                )}

                {/* Property definitions */}
                {nodeType.resolved_properties && Array.isArray(nodeType.resolved_properties) && nodeType.resolved_properties.length > 0 && (
                  <div>
                    <h4 className="text-sm font-medium text-zinc-400 mb-2">Property Schema</h4>
                    <div className="space-y-2">
                      {nodeType.resolved_properties.map((propDef: any, index: number) => {
                        const propertyType = getSchemaType(propDef)
                        const propName = propDef.name ?? `property-${index}`
                        const source = propertySourceMap.get(propName)
                        return (
                          <div key={propName} className="flex items-center gap-2 text-sm">
                            {getPropertyIcon(propertyType)}
                            <span className="text-zinc-300">{propName}</span>
                            <span className="text-zinc-500">({propertyType})</span>
                            {propDef.required && (
                              <span className="px-2 py-0.5 bg-red-500/20 text-red-300 rounded text-xs">
                                Required
                              </span>
                            )}
                            {source && (
                              <span className="px-2 py-0.5 bg-teal-500/15 text-teal-400/80 rounded text-xs">
                                {source}
                              </span>
                            )}
                          </div>
                        )
                      })}
                    </div>
                  </div>
                )}

                {/* Inheritance chain */}
                {nodeType.inheritance_chain && nodeType.inheritance_chain.length > 1 && (
                  <div className="mt-4">
                    <h4 className="text-sm font-medium text-zinc-400 mb-2">Inheritance Chain</h4>
                    <div className="flex items-center gap-2">
                      {nodeType.inheritance_chain.map((type, index) => (
                        <div key={type} className="flex items-center gap-2">
                          <span className="text-zinc-300 text-sm">{type}</span>
                          {index < nodeType.inheritance_chain.length - 1 && (
                            <span className="text-zinc-500">→</span>
                          )}
                        </div>
                      ))}
                    </div>
                  </div>
                )}
              </div>
            )}

            {/* Archetype Info */}
            {node.archetype && (
              <div className="bg-white/5 rounded-xl p-6">
                <h3 className="text-lg font-semibold text-white mb-4 flex items-center gap-2">
                  <Layers className="w-5 h-5 text-purple-400" />
                  Archetype
                </h3>
                <p className="text-zinc-300 mb-2">{node.archetype}</p>

                {/* Show resolved archetype fields */}
                {resolvedArchetype?.resolved_fields && resolvedArchetype.resolved_fields.length > 0 && (
                  <div className="mb-3">
                    <h4 className="text-sm font-medium text-zinc-400 mb-2">Fields</h4>
                    <div className="space-y-1">
                      {resolvedArchetype.resolved_fields.map((field: any) => {
                        const fieldName = field.base?.name ?? field.name ?? 'unknown'
                        const fieldType = field.$type ?? 'Unknown'
                        return (
                          <div key={fieldName} className="flex items-center gap-2 text-sm">
                            {getPropertyIcon(fieldType.replace('Field', '').toLowerCase())}
                            <span className="text-zinc-300">{fieldName}</span>
                            <span className="text-zinc-500">({fieldType})</span>
                            {field.base?.required && (
                              <span className="px-2 py-0.5 bg-red-500/20 text-red-300 rounded text-xs">
                                Required
                              </span>
                            )}
                          </div>
                        )
                      })}
                    </div>
                  </div>
                )}

                {/* Archetype inheritance chain */}
                {resolvedArchetype?.inheritance_chain && resolvedArchetype.inheritance_chain.length > 1 && (
                  <div className="mb-3">
                    <h4 className="text-sm font-medium text-zinc-400 mb-2">Inheritance Chain</h4>
                    <div className="flex items-center gap-2">
                      {resolvedArchetype.inheritance_chain.map((name, index) => (
                        <div key={name} className="flex items-center gap-2">
                          <span className="text-zinc-300 text-sm">{name}</span>
                          {index < resolvedArchetype.inheritance_chain.length - 1 && (
                            <span className="text-zinc-500">&rarr;</span>
                          )}
                        </div>
                      ))}
                    </div>
                  </div>
                )}

                <Link
                  to={`/${repo}/archetypes/${encodeURIComponent(node.archetype)}`}
                  className="text-primary-400 hover:text-primary-300 text-sm inline-block"
                >
                  View Archetype Definition &rarr;
                </Link>
              </div>
            )}

            {/* Children section */}
            {node.children && node.children.length > 0 && (
              <div className="bg-white/5 rounded-xl p-6">
                <h3 className="text-lg font-semibold text-white mb-4">
                  Children ({node.children.length})
                </h3>
                <div className="grid grid-cols-2 md:grid-cols-3 lg:grid-cols-4 gap-3">
                  {(node.children as any[]).map((child) => {
                    const isString = typeof child === 'string'
                    const childName = isString ? child : child.name
                    const childType = isString ? undefined : child.node_type
                    return (
                      <div
                        key={isString ? child : child.id}
                        className="flex items-center gap-2 p-3 bg-white/5 rounded-lg hover:bg-white/10 transition-colors"
                      >
                        {getNodeIcon(childType)}
                        <span className="text-white text-sm truncate">{childName}</span>
                      </div>
                    )
                  })}
                </div>
              </div>
            )}

            {/* Metadata */}
            <div className="bg-white/5 rounded-xl p-6">
              <h3 className="text-lg font-semibold text-white mb-4">Metadata</h3>
              <dl className="grid grid-cols-2 gap-4">
                <div>
                  <dt className="text-sm text-zinc-400">ID</dt>
                  <dd className="text-white font-mono text-sm">{node.id}</dd>
                </div>
                <div>
                  <dt className="text-sm text-zinc-400">Created</dt>
                  <dd className="text-white text-sm">
                    {node.created_at ? new Date(node.created_at).toLocaleString() : 'N/A'}
                  </dd>
                </div>
                <div>
                  <dt className="text-sm text-zinc-400">Updated</dt>
                  <dd className="text-white text-sm">
                    {node.updated_at ? new Date(node.updated_at).toLocaleString() : 'N/A'}
                  </dd>
                </div>
                <div>
                  <dt className="text-sm text-zinc-400">Path</dt>
                  <dd className="text-white font-mono text-sm">{node.path}</dd>
                </div>
              </dl>
            </div>

            {/* Version History */}
            <div className="bg-white/5 rounded-xl p-6">
              <VersionHistory
                repo={repo}
                branch={branch}
                workspace={workspace}
                nodePath={node.path}
                onRestore={async (restoredNode) => {
                  // Update the node in the parent component
                  await onUpdate(restoredNode)
                }}
              />
            </div>

            {/* Audit Log */}
            <div className="bg-white/5 rounded-xl p-6">
              <AuditLog
                repo={repo}
                branch={branch}
                workspace={workspace}
                nodePath={node.path}
              />
            </div>

            {/* Operation History - Shows move/copy/rename operations */}
            <div className="bg-white/5 rounded-xl p-6">
              <NodeOperationHistory
                repo={repo}
                branch={branch}
                workspace={workspace}
                nodeId={node.id}
                nodePath={node.path}
                limit={20}
              />
            </div>

            {/* Relationships */}
            {!readonly && (
              <div className="bg-white/5 rounded-xl p-6">
                <RelationshipManager
                  repo={repo}
                  branch={branch}
                  workspace={workspace}
                  nodePath={node.path}
                />
              </div>
            )}
          </div>
        )}
      </div>

      {/* Create Child Dialog */}
      {showCreateDialog && (
        <CreateNodeDialog
          repo={repo}
          branch={branch}
          workspace={workspace}
          parentPath={node.path}
          parentName={node.name}
          allowedChildren={nodeType?.resolved_allowed_children}
          onClose={() => setShowCreateDialog(false)}
          onCreate={handleCreateChild}
        />
      )}

      {/* Copy Modal */}
      {showCopyModal && (
        <CopyNodeModal
          node={node}
          allNodes={allNodes}
          onCopy={(dest, name, recursive) => onCopy(node, dest, name, recursive)}
          onClose={() => setShowCopyModal(false)}
        />
      )}

      {/* Move Modal */}
      {showMoveModal && (
        <MoveNodeModal
          node={node}
          allNodes={allNodes}
          onMove={(dest) => onMove(node, dest)}
          onClose={() => setShowMoveModal(false)}
        />
      )}
    </div>
  )
}
