import { useState, useEffect } from 'react'
import { createPortal } from 'react-dom'
import { Plus, Loader, AlertCircle } from 'lucide-react'
import { useNodeType } from '../hooks/useNodeType'
import StringField from './PropertyFields/StringField'
import NumberField from './PropertyFields/NumberField'
import BooleanField from './PropertyFields/BooleanField'
import SelectField from './PropertyFields/SelectField'
import ObjectField from './PropertyFields/ObjectField'
import ArrayField from './PropertyFields/ArrayField'
import DateField from './PropertyFields/DateField'
import ArchetypePicker from './shared/ArchetypePicker'
import type { CreateNodeRequest } from '../api/nodes'
import { nodeTypesApi, type NodeType } from '../api/nodetypes'
import { workspacesApi } from '../api/workspaces'
import {
  getSchemaType,
  getSchemaLabel,
  getSchemaPlaceholder,
  getSchemaItems,
  getSchemaStructure,
  getEnumOptions,
  createDefaultFromSchema,
  validateValueAgainstSchema,
  getSchemaDescription,
} from '../utils/propertySchema'

interface CreateNodeDialogProps {
  repo: string
  branch: string
  workspace: string
  parentPath?: string
  parentName?: string
  allowedChildren?: string[]
  onClose: () => void
  onCreate: (nodeData: CreateNodeRequest) => Promise<void>
}

export default function CreateNodeDialog({
  repo,
  branch,
  workspace,
  parentPath,
  parentName,
  allowedChildren,
  onClose,
  onCreate
}: CreateNodeDialogProps) {
  const [selectedType, setSelectedType] = useState<string>('')
  const [selectedArchetype, setSelectedArchetype] = useState<string>('')
  const [availableTypes, setAvailableTypes] = useState<NodeType[]>([])
  const [nodeName, setNodeName] = useState('')
  const [properties, setProperties] = useState<Record<string, any>>({})
  const [errors, setErrors] = useState<Record<string, string>>({})
  const [creating, setCreating] = useState(false)
  const [loadingTypes, setLoadingTypes] = useState(true)
  const [workspaceConfig, setWorkspaceConfig] = useState<{
    allowed_node_types?: string[]
    allowed_root_node_types?: string[]
  } | null>(null)

  // Fetch node type definition when selected
  const { nodeType, loading: loadingNodeType } = useNodeType(repo, branch, selectedType, workspace)

  // Load workspace configuration on mount
  useEffect(() => {
    async function loadWorkspace() {
      try {
        const ws = await workspacesApi.get(repo, workspace)
        setWorkspaceConfig(ws)
      } catch (error) {
        console.error('Failed to load workspace config:', error)
        // Continue with no restrictions if workspace not found
        setWorkspaceConfig(null)
      }
    }

    loadWorkspace()
  }, [repo, workspace])

  // Load available node types on mount
  useEffect(() => {
    async function loadTypes() {
      try {
        const allTypes = await nodeTypesApi.list(repo, branch)
        const isRootNode = !parentPath || parentPath === '/'

        let filtered = allTypes

        // VALIDATION 1: Apply workspace-level restrictions FIRST
        if (workspaceConfig) {
          // For root nodes: filter by allowed_root_node_types
          if (isRootNode && workspaceConfig.allowed_root_node_types && workspaceConfig.allowed_root_node_types.length > 0) {
            // Check for wildcard
            if (!workspaceConfig.allowed_root_node_types.includes('*')) {
              filtered = filtered.filter(t => workspaceConfig.allowed_root_node_types!.includes(t.name))
            }
          }

          // For all nodes: filter by allowed_node_types
          if (workspaceConfig.allowed_node_types && workspaceConfig.allowed_node_types.length > 0) {
            // Check for wildcard
            if (!workspaceConfig.allowed_node_types.includes('*')) {
              filtered = filtered.filter(t => workspaceConfig.allowed_node_types!.includes(t.name))
            }
          }
        }

        // VALIDATION 2: Apply parent's allowed_children (for child nodes only)
        if (!isRootNode && allowedChildren && allowedChildren.length > 0) {
          // Check for wildcard
          if (!allowedChildren.includes('*')) {
            filtered = filtered.filter(t => allowedChildren.includes(t.name))
          }
        }

        setAvailableTypes(filtered)

        // Pre-select first type
        if (filtered.length > 0) {
          setSelectedType(filtered[0].name)
        }
      } catch (error) {
        console.error('Failed to load node types:', error)
      } finally {
        setLoadingTypes(false)
      }
    }

    // Only load types after workspace config is loaded (or failed to load)
    if (workspaceConfig !== undefined) {
      loadTypes()
    }
  }, [repo, branch, workspace, parentPath, allowedChildren, workspaceConfig])

  // Initialize properties with defaults when node type changes
  useEffect(() => {
    if (nodeType?.resolved_properties && Array.isArray(nodeType.resolved_properties)) {
      const defaults: Record<string, any> = {}

      nodeType.resolved_properties.forEach((propDef: any) => {
        const propName = propDef.name
        if (!propName) return
        const initialValue =
          propDef.default !== undefined
            ? propDef.default
            : createDefaultFromSchema(propDef)

        if (initialValue !== undefined) {
          defaults[propName] = initialValue
        }
      })

      setProperties(defaults)
      setErrors({})
    }
  }, [nodeType])

  function validateField(name: string, value: any, schema: any): string | null {
    return validateValueAgainstSchema(name, value, schema)
  }

  function handlePropertyChange(name: string, value: any) {
    setProperties(prev => ({
      ...prev,
      [name]: value
    }))

    // Clear error when user starts typing
    if (errors[name]) {
      setErrors(prev => {
        const next = { ...prev }
        delete next[name]
        return next
      })
    }
  }

  async function handleCreate() {
    // Validate name
    if (!nodeName.trim()) {
      setErrors({ name: 'Name is required' })
      return
    }

    // Validate all properties
    const newErrors: Record<string, string> = {}

    if (nodeType?.resolved_properties && Array.isArray(nodeType.resolved_properties)) {
      nodeType.resolved_properties.forEach((propSchema: any) => {
        const propName = propSchema.name
        if (!propName) return
        const error = validateField(propName, properties[propName], propSchema)
        if (error) {
          newErrors[propName] = error
        }
      })
    }

    if (Object.keys(newErrors).length > 0) {
      setErrors(newErrors)
      return
    }

    setCreating(true)
    try {
      // Filter out empty/undefined properties
      const filteredProps = Object.entries(properties).reduce((acc, [key, value]) => {
        if (value !== undefined && value !== null && value !== '') {
          acc[key] = value
        }
        return acc
      }, {} as Record<string, any>)

      await onCreate({
        name: nodeName.trim(),
        node_type: selectedType,
        archetype: selectedArchetype || undefined,
        properties: Object.keys(filteredProps).length > 0 ? filteredProps : undefined
      })
      onClose()
    } catch (error) {
      console.error('Failed to create node:', error)
      setErrors({ _general: 'Failed to create node. Please try again.' })
    } finally {
      setCreating(false)
    }
  }

  function renderPropertyField(name: string, schema: any) {
    const value = properties[name]
    const error = errors[name]
    const schemaType = getSchemaType(schema)
    const label = getSchemaLabel(name, schema)
    const placeholder = getSchemaPlaceholder(schema)

    const commonProps = {
      name,
      label,
      value,
      error,
      required: schema.required,
      onChange: (val: any) => handlePropertyChange(name, val)
    }

    switch (schemaType) {
      case 'string': {
        const options = getEnumOptions(schema)
        if (options && options.length > 0) {
          return <SelectField {...commonProps} options={options} placeholder={placeholder} />
        }
        return (
          <StringField
            {...commonProps}
            multiline={schema.multiline}
            placeholder={placeholder}
          />
        )
      }
      case 'number':
      case 'integer':
        return (
          <NumberField
            {...commonProps}
            min={schema.minimum}
            max={schema.maximum}
            step={schema.step}
          />
        )
      case 'boolean':
        return <BooleanField {...commonProps} />
      case 'array':
        return (
          <ArrayField
            {...commonProps}
            itemType={getSchemaItems(schema)}
          />
        )
      case 'object':
        return (
          <ObjectField
            {...commonProps}
            schema={getSchemaStructure(schema)}
          />
        )
      case 'date':
        return (
          <DateField
            {...commonProps}
            placeholder={placeholder}
          />
        )
      case 'resource':
      case 'reference':
      case 'nodetype':
      case 'element':
      case 'composite':
      case 'url':
        return (
          <StringField
            {...commonProps}
            placeholder={placeholder || 'Enter value'}
          />
        )
      default:
        return (
          <StringField
            {...commonProps}
            placeholder={placeholder}
          />
        )
    }
  }

  const isValid = nodeName.trim() && selectedType && !loadingNodeType

  return createPortal(
    <div className="fixed inset-0 bg-black/50 backdrop-blur-sm flex items-center justify-center p-8 z-50 overscroll-none">
      <div className="glass-dark rounded-xl max-w-2xl w-full max-h-[90vh] overflow-hidden flex flex-col overscroll-contain">
        {/* Header */}
        <div className="p-6 border-b border-white/10">
          <h2 className="text-xl font-bold text-white">
            {parentPath ? `Create Child Node under "${parentName}"` : 'Create Root Node'}
          </h2>
          {parentPath && (
            <p className="text-sm text-gray-400 mt-1">Parent: {parentPath}</p>
          )}
        </div>

        {/* Content */}
        <div className="flex-1 overflow-y-auto p-6">
          {loadingTypes ? (
            <div className="text-center py-8">
              <Loader className="w-8 h-8 text-purple-400 animate-spin mx-auto mb-2" />
              <p className="text-gray-400">Loading node types...</p>
            </div>
          ) : availableTypes.length === 0 ? (
            <div className="text-center py-8">
              <AlertCircle className="w-8 h-8 text-red-400 mx-auto mb-2" />
              <p className="text-gray-400">No node types available</p>
            </div>
          ) : (
            <div className="space-y-6">
              {/* Error message */}
              {errors._general && (
                <div className="p-4 bg-red-500/20 border border-red-500/50 rounded-lg text-red-300">
                  {errors._general}
                </div>
              )}

              {/* Node Type Selection */}
              <div>
                <label className="block text-sm font-medium text-gray-300 mb-2">
                  Node Type <span className="text-red-400">*</span>
                </label>
                <select
                  value={selectedType}
                  onChange={(e) => setSelectedType(e.target.value)}
                  className="w-full px-4 py-2 bg-white/10 border border-white/20 rounded-lg text-white focus:outline-none focus:ring-2 focus:ring-purple-500"
                >
                  {availableTypes.map(type => (
                    <option key={type.name} value={type.name}>
                      {type.name}
                    </option>
                  ))}
                </select>
              </div>

              {/* Archetype Selection (Optional) */}
              <div>
                <label className="block text-sm font-medium text-gray-300 mb-2">
                  Archetype <span className="text-zinc-500">(optional)</span>
                </label>
                <ArchetypePicker
                  mode="single"
                  value={selectedArchetype}
                  onChange={(value) => setSelectedArchetype(value as string)}
                  allowNone
                  noneLabel="No archetype"
                  publishedOnly
                />
                <p className="text-xs text-zinc-500 mt-1">
                  Use an archetype template for field structure
                </p>
              </div>

              {/* Name Field */}
              <div>
                <label className="block text-sm font-medium text-gray-300 mb-2">
                  Name <span className="text-red-400">*</span>
                </label>
                <input
                  type="text"
                  value={nodeName}
                  onChange={(e) => {
                    setNodeName(e.target.value)
                    if (errors.name) {
                      setErrors(prev => {
                        const next = { ...prev }
                        delete next.name
                        return next
                      })
                    }
                  }}
                  className={`w-full px-4 py-2 bg-white/10 border rounded-lg text-white focus:outline-none focus:ring-2 ${
                    errors.name
                      ? 'border-red-500/50 focus:ring-red-500'
                      : 'border-white/20 focus:ring-purple-500'
                  }`}
                  placeholder="my-node"
                />
                {errors.name && <p className="mt-1 text-sm text-red-400">{errors.name}</p>}
              </div>

              {/* Dynamic Properties based on Node Type */}
              {loadingNodeType ? (
                <div className="text-center py-4">
                  <Loader className="w-6 h-6 text-purple-400 animate-spin mx-auto" />
                  <p className="text-sm text-gray-400 mt-2">Loading node type definition...</p>
                </div>
              ) : nodeType?.resolved_properties && Array.isArray(nodeType.resolved_properties) && nodeType.resolved_properties.length > 0 ? (
                <div className="space-y-4">
                  <h3 className="text-lg font-semibold text-white">Properties</h3>
                  {nodeType.resolved_properties.map((propSchema: any) => {
                    const propName = propSchema.name
                    if (!propName) {
                      return null
                    }
                    const description = getSchemaDescription(propSchema)
                    return (
                      <div key={propName} className="space-y-1">
                        {renderPropertyField(propName, propSchema)}
                        <div className="flex items-center gap-2 text-xs">
                          {description && (
                            <p className="text-gray-500">{description}</p>
                          )}
                          <span className="text-gray-600">• Property: {propName}</span>
                        </div>
                      </div>
                    )
                  })}
                </div>
              ) : (
                nodeType && (
                  <div className="text-sm text-gray-500">
                    This node type has no additional properties.
                  </div>
                )
              )}

              {/* Show validation summary */}
              {Object.keys(errors).filter(k => k !== '_general' && k !== 'name').length > 0 && (
                <div className="p-4 bg-red-500/20 border border-red-500/50 rounded-lg">
                  <div className="flex items-center gap-2 text-red-300">
                    <AlertCircle className="w-5 h-5" />
                    <span className="font-semibold">Please fix the following errors:</span>
                  </div>
                  <ul className="mt-2 list-disc list-inside text-red-300 text-sm">
                    {Object.entries(errors)
                      .filter(([k]) => k !== '_general' && k !== 'name')
                      .map(([field, error]) => (
                        <li key={field}>{error}</li>
                      ))}
                  </ul>
                </div>
              )}
            </div>
          )}
        </div>

        {/* Footer */}
        <div className="p-6 border-t border-white/10">
          <div className="flex gap-3">
            <button
              onClick={handleCreate}
              disabled={!isValid || creating}
              className="flex-1 flex items-center justify-center gap-2 px-4 py-2 bg-purple-500 hover:bg-purple-600 text-white rounded-lg transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
            >
              {creating ? (
                <>
                  <Loader className="w-4 h-4 animate-spin" />
                  Creating...
                </>
              ) : (
                <>
                  <Plus className="w-4 h-4" />
                  Create
                </>
              )}
            </button>
            <button
              onClick={onClose}
              disabled={creating}
              className="flex-1 px-4 py-2 bg-white/10 hover:bg-white/20 text-white rounded-lg transition-colors disabled:opacity-50"
            >
              Cancel
            </button>
          </div>
        </div>
      </div>
    </div>,
    document.body
  )
}
