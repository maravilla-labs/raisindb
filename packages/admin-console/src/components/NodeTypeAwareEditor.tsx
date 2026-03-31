import { useState, useEffect, useMemo } from 'react'
import { Save, X, AlertCircle, Globe, Info, AlertTriangle, CheckCircle2, Puzzle } from 'lucide-react'
import type { Node } from '../api/nodes'
import type { ResolvedNodeType, NodeType as NodeTypeDef } from '../api/nodetypes'
import { nodeTypesApi } from '../api/nodetypes'
import type { ResolvedArchetype } from '../api/archetypes'
import { translationsApi, StalenessReport } from '../api/translations'
import { nodesApi } from '../api/nodes'
import StringField from './PropertyFields/StringField'
import NumberField from './PropertyFields/NumberField'
import BooleanField from './PropertyFields/BooleanField'
import SelectField from './PropertyFields/SelectField'
import ObjectField from './PropertyFields/ObjectField'
import ArrayField from './PropertyFields/ArrayField'
import DateField from './PropertyFields/DateField'
import ArchetypeFieldRenderer from './ArchetypeFieldRenderer'
import {
  getSchemaType,
  getSchemaLabel,
  getSchemaPlaceholder,
  getSchemaItems,
  getSchemaStructure,
  getEnumOptions,
  validateValueAgainstSchema,
  getSchemaDescription,
  isSchemaTranslatable,
} from '../utils/propertySchema'

interface NodeTypeAwareEditorProps {
  node: Node
  nodeType: ResolvedNodeType | null
  resolvedArchetype?: ResolvedArchetype | null
  repo?: string
  branch?: string
  workspace?: string
  currentLocale?: string | null
  defaultLanguage?: string
  onSave: (node: Partial<Node>) => Promise<void>
  onCancel: () => void
}

export default function NodeTypeAwareEditor({
  node,
  nodeType,
  resolvedArchetype,
  repo,
  branch,
  workspace,
  currentLocale,
  defaultLanguage,
  onSave,
  onCancel
}: NodeTypeAwareEditorProps) {
  const [editedNode, setEditedNode] = useState<Partial<Node>>({})
  const [properties, setProperties] = useState<Record<string, any>>({})
  const [originalProperties, setOriginalProperties] = useState<Record<string, any>>({})
  const [errors, setErrors] = useState<Record<string, string>>({})
  const [saving, setSaving] = useState(false)
  const [_loadingOriginal, setLoadingOriginal] = useState(false)
  const [stalenessReport, setStalenessReport] = useState<StalenessReport | null>(null)
  const [_loadingStaleness, setLoadingStaleness] = useState(false)
  const [mixinDefs, setMixinDefs] = useState<Map<string, NodeTypeDef>>(new Map())

  // Fetch mixin definitions for property source attribution
  const mixins = nodeType?.node_type?.mixins || []
  useEffect(() => {
    if (mixins.length === 0 || !repo || !branch) {
      setMixinDefs(new Map())
      return
    }
    let cancelled = false
    async function fetchMixins() {
      const defs = new Map<string, NodeTypeDef>()
      await Promise.all(
        mixins.map(async (name) => {
          try {
            const def = await nodeTypesApi.get(repo!, branch!, name)
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

  // Build property name -> source mixin name map
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

  // Translation mode is active when currentLocale is set and different from default
  const isTranslationMode = currentLocale && defaultLanguage && currentLocale !== defaultLanguage

  // When an archetype is present with fields, use those as the primary schema
  const hasArchetypeFields = !!(
    resolvedArchetype?.resolved_fields && resolvedArchetype.resolved_fields.length > 0
  )

  // Set of property names covered by archetype fields
  const archetypeFieldNames = useMemo(() => {
    if (!hasArchetypeFields) return new Set<string>()
    return new Set(
      resolvedArchetype!.resolved_fields.map(
        (f: any) => (f.base?.name ?? f.name) as string
      )
    )
  }, [hasArchetypeFields, resolvedArchetype])

  // Map of field name → $type for archetype fields (used for smart translation diff)
  const archetypeFieldTypes = useMemo(() => {
    if (!hasArchetypeFields) return new Map<string, string>()
    const map = new Map<string, string>()
    for (const f of resolvedArchetype!.resolved_fields) {
      const name = ((f as any).base?.name ?? (f as any).name) as string
      const type = (f as any).$type as string
      if (name && type) map.set(name, type)
    }
    return map
  }, [hasArchetypeFields, resolvedArchetype])

  useEffect(() => {
    // Initialize with current node data
    setEditedNode({
      name: node.name,
      node_type: node.node_type,
    })
    setProperties(node.properties || {})

    // In translation mode, fetch the master/original node (without locale)
    // to show the true original values, not the fallback values
    if (isTranslationMode && repo && branch && workspace) {
      setLoadingOriginal(true)
      nodesApi.getAtHead(repo, branch, workspace, node.path)
        .then(masterNode => {
          setOriginalProperties(masterNode.properties || {})
        })
        .catch(err => {
          console.error('Failed to load master node for translation reference:', err)
          // Fallback to current node properties if fetch fails
          setOriginalProperties(node.properties || {})
        })
        .finally(() => {
          setLoadingOriginal(false)
        })
    } else {
      // Not in translation mode, just use the current node properties
      setOriginalProperties(node.properties || {})
    }
  }, [node, isTranslationMode, repo, branch, workspace])

  // Fetch staleness report in translation mode
  useEffect(() => {
    if (isTranslationMode && repo && branch && workspace && currentLocale) {
      setLoadingStaleness(true)
      translationsApi.checkStaleness(repo, branch, workspace, node.path, currentLocale)
        .then(report => {
          setStalenessReport(report)
        })
        .catch(err => {
          console.error('Failed to load staleness report:', err)
          setStalenessReport(null)
        })
        .finally(() => {
          setLoadingStaleness(false)
        })
    } else {
      setStalenessReport(null)
    }
  }, [node.path, isTranslationMode, repo, branch, workspace, currentLocale])

  function validateProperty(name: string, value: any, schema: any): string | null {
    return validateValueAgainstSchema(name, value, schema)
  }

  // Get staleness status for a field
  function getFieldStaleness(fieldName: string): 'fresh' | 'stale' | 'missing' | 'unknown' | null {
    if (!stalenessReport || !isTranslationMode) return null

    const pointer = `/${fieldName}`

    // Check stale fields
    if (stalenessReport.stale.some(s => s.pointer === pointer || s.pointer.startsWith(`${pointer}/`))) {
      return 'stale'
    }

    // Check fresh fields
    if (stalenessReport.fresh.some(f => f === pointer || f.startsWith(`${pointer}/`))) {
      return 'fresh'
    }

    // Check missing fields
    if (stalenessReport.missing.some(m => m.pointer === pointer || m.pointer.startsWith(`${pointer}/`))) {
      return 'missing'
    }

    // Check unknown fields (legacy - no hash record)
    if (stalenessReport.unknown.some(u => u === pointer || u.startsWith(`${pointer}/`))) {
      return 'unknown'
    }

    return null
  }

  // Get stale field info for a field
  function getStaleFieldInfo(fieldName: string) {
    if (!stalenessReport) return null
    const pointer = `/${fieldName}`
    return stalenessReport.stale.find(s => s.pointer === pointer || s.pointer.startsWith(`${pointer}/`))
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

  async function handleSave() {
    // Validate all properties
    const newErrors: Record<string, string> = {}

    const propsToValidate = isTranslationMode
      ? nodeType?.resolved_properties?.filter((p: any) => isSchemaTranslatable(p))
      : nodeType?.resolved_properties

    if (propsToValidate && Array.isArray(propsToValidate)) {
      propsToValidate.forEach((propSchema: any) => {
        const propName = propSchema.name
        const error = validateProperty(propName, properties[propName], propSchema)
        if (error) {
          newErrors[propName] = error
        }
      })
    }

    if (Object.keys(newErrors).length > 0) {
      setErrors(newErrors)
      return
    }

    setSaving(true)
    try {
      if (isTranslationMode && repo && branch && workspace && currentLocale) {
        // Build granular translation pointers.
        // For section fields, diff per-element by UUID instead of replacing the
        // entire array, so translations survive element reordering.
        const translations: Record<string, any> = {}

        for (const [key, value] of Object.entries(properties)) {
          const original = originalProperties[key]
          if (JSON.stringify(value) === JSON.stringify(original)) continue

          const fieldType = archetypeFieldTypes.get(key)

          if (fieldType === 'SectionField') {
            // Normalise both sides to Element[]
            const items: any[] = Array.isArray(value) ? value : (value?.items ?? [])
            const origItems: any[] = Array.isArray(original) ? original : (original?.items ?? [])
            const origByUuid = new Map<string, any>()
            for (const el of origItems) {
              if (el.uuid) origByUuid.set(el.uuid, el)
            }

            for (const el of items) {
              if (!el.uuid) continue
              const origEl = origByUuid.get(el.uuid)
              if (!origEl) continue // new element added in translation — skip

              for (const [fieldName, fieldValue] of Object.entries(el)) {
                if (fieldName === 'element_type' || fieldName === 'uuid') continue
                if (JSON.stringify(fieldValue) !== JSON.stringify(origEl[fieldName])) {
                  translations[`/${key}/${el.uuid}/${fieldName}`] = fieldValue
                }
              }
            }
          } else {
            // Flat property diff
            translations[`/${key}`] = value
          }
        }

        // Only call API if there are actual changes
        if (Object.keys(translations).length > 0) {
          await translationsApi.updateTranslation(
            repo,
            branch,
            workspace,
            node.path,
            currentLocale,
            {
              translations,
              message: `Updated ${currentLocale} translation for ${node.name}`,
              actor: 'admin-user'
            }
          )
        }
        onCancel() // Close editor after save
      } else {
        // Normal mode: save all properties
        await onSave({
          ...editedNode,
          properties
        })
      }
    } catch (error) {
      console.error('Failed to save:', error)
    } finally {
      setSaving(false)
    }
  }

  function renderPropertyField(name: string, schema: any) {
    const value = properties[name]
    const originalValue = originalProperties[name]
    const error = errors[name]
    const schemaType = getSchemaType(schema)
    const label = getSchemaLabel(name, schema)
    const placeholder = getSchemaPlaceholder(schema)
    const options = getEnumOptions(schema)
    const structure = getSchemaStructure(schema)
    const items = getSchemaItems(schema)
    const description = getSchemaDescription(schema)
    const commonProps = {
      name,
      label,
      value,
      error,
      required: schema?.required,
      onChange: (val: any) => handlePropertyChange(name, val)
    }

    const field = (() => {
      switch (schemaType) {
        case 'string':
          if (options && options.length > 0) {
            return <SelectField {...commonProps} options={options} placeholder={placeholder} />
          }
          return (
            <StringField
              {...commonProps}
              multiline={schema?.multiline}
              placeholder={placeholder}
            />
          )
        case 'number':
        case 'integer':
          return (
            <NumberField
              {...commonProps}
              min={schema?.minimum}
              max={schema?.maximum}
              step={schema?.step}
            />
          )
        case 'boolean':
          return <BooleanField {...commonProps} />
        case 'array':
          return (
            <ArrayField
              {...commonProps}
              itemType={items}
            />
          )
        case 'object':
          return (
            <ObjectField
              {...commonProps}
              schema={structure}
            />
          )
        case 'date':
          return <DateField {...commonProps} placeholder={placeholder} />
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
          // Fallback to string field for unknown types
          return (
            <StringField
              {...commonProps}
              placeholder={placeholder}
            />
          )
      }
    })()

    return (
      <div className="space-y-1">
        {field}
        {description && <p className="text-xs text-zinc-500">{description}</p>}
        {isTranslationMode && originalValue !== undefined && originalValue !== null && (
          <div className="mt-1.5 flex items-start gap-1.5 text-xs text-white/50">
            <Info className="w-3 h-3 mt-0.5 flex-shrink-0" />
            <span className="break-words min-w-0">
              Original{defaultLanguage ? ` (${defaultLanguage})` : ''}:{' '}
              {typeof originalValue === 'object'
                ? JSON.stringify(originalValue)
                : String(originalValue)}
            </span>
          </div>
        )}
      </div>
    )
  }

  return (
    <div className="h-full flex flex-col p-6">
      {/* Header */}
      <div className="flex items-center justify-between mb-6">
        <div>
          <h2 className="text-xl font-semibold text-white">
            {isTranslationMode ? 'Edit Translation' : 'Edit Node Properties'}
          </h2>
          {isTranslationMode && (
            <div className="flex items-center gap-2 mt-2 text-sm">
              <Globe className="w-4 h-4 text-accent-400" />
              <span className="text-accent-400 font-semibold">
                Editing translation for: {currentLocale}
              </span>
            </div>
          )}
        </div>
        <div className="flex items-center gap-3">
          <button
            onClick={onCancel}
            className="px-4 py-2 bg-white/10 hover:bg-white/20 text-white rounded-lg transition-colors"
          >
            <X className="w-4 h-4 inline-block mr-2" />
            Cancel
          </button>
          <button
            onClick={handleSave}
            disabled={saving}
            className="px-4 py-2 bg-purple-500 hover:bg-purple-600 text-white rounded-lg transition-colors disabled:opacity-50"
          >
            <Save className="w-4 h-4 inline-block mr-2" />
            {saving ? 'Saving...' : 'Save'}
          </button>
        </div>
      </div>

      {/* Staleness summary banner */}
      {isTranslationMode && stalenessReport && (stalenessReport.stale.length > 0 || stalenessReport.missing.length > 0) && (
        <div className="mb-4 p-4 bg-amber-500/20 border border-amber-500/50 rounded-lg">
          <div className="flex items-center gap-2 text-amber-300">
            <AlertTriangle className="w-5 h-5" />
            <span className="font-semibold">Translation Status</span>
          </div>
          <div className="mt-2 text-amber-200 text-sm">
            {stalenessReport.stale.length > 0 && (
              <p>{stalenessReport.stale.length} field{stalenessReport.stale.length > 1 ? 's' : ''} may be stale (original changed since translation)</p>
            )}
            {stalenessReport.missing.length > 0 && (
              <p>{stalenessReport.missing.length} field{stalenessReport.missing.length > 1 ? 's' : ''} need{stalenessReport.missing.length === 1 ? 's' : ''} translation</p>
            )}
          </div>
        </div>
      )}

      {isTranslationMode && stalenessReport && stalenessReport.stale.length === 0 && stalenessReport.missing.length === 0 && stalenessReport.fresh.length > 0 && (
        <div className="mb-4 p-4 bg-green-500/20 border border-green-500/50 rounded-lg">
          <div className="flex items-center gap-2 text-green-300">
            <CheckCircle2 className="w-5 h-5" />
            <span className="font-semibold">Translation Up to Date</span>
          </div>
          <p className="mt-1 text-green-200 text-sm">
            All {stalenessReport.fresh.length} translated field{stalenessReport.fresh.length > 1 ? 's are' : ' is'} current with the original content.
          </p>
        </div>
      )}

      {/* Error summary */}
      {Object.keys(errors).length > 0 && (
        <div className="mb-4 p-4 bg-red-500/20 border border-red-500/50 rounded-lg">
          <div className="flex items-center gap-2 text-red-300">
            <AlertCircle className="w-5 h-5" />
            <span className="font-semibold">Please fix the following errors:</span>
          </div>
          <ul className="mt-2 list-disc list-inside text-red-300 text-sm">
            {Object.entries(errors).map(([field, error]) => (
              <li key={field}>{error}</li>
            ))}
          </ul>
        </div>
      )}

      {/* Form fields */}
      <div className="flex-1 overflow-auto">
        <div className="space-y-6">
          {/* Basic fields - hide in translation mode */}
          {!isTranslationMode && (
            <div className="bg-white/5 rounded-xl p-6">
              <h3 className="text-lg font-semibold text-white mb-4">Basic Information</h3>
              <div className="space-y-4">
                <div>
                  <label className="block text-sm font-medium text-gray-300 mb-2">
                    Name <span className="text-red-400">*</span>
                  </label>
                  <input
                    type="text"
                    value={editedNode.name || ''}
                    onChange={(e) => setEditedNode(prev => ({ ...prev, name: e.target.value }))}
                    className="w-full px-4 py-2 bg-white/10 border border-white/20 rounded-lg text-white focus:outline-none focus:ring-2 focus:ring-purple-500"
                  />
                </div>
                <div>
                  <label className="block text-sm font-medium text-gray-300 mb-2">Node Type</label>
                  <input
                    type="text"
                    value={editedNode.node_type || ''}
                    disabled
                    className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-gray-400"
                  />
                </div>
              </div>
            </div>
          )}

          {/* Archetype fields (primary when present) */}
          {hasArchetypeFields && (
            <div className="bg-white/5 rounded-xl p-6">
              <h3 className="text-lg font-semibold text-white mb-4">
                {isTranslationMode ? 'Translation' : 'Properties'}
              </h3>
              <div className="space-y-4">
                {resolvedArchetype!.resolved_fields
                  .filter((field: any) => {
                    if (isTranslationMode) {
                      const base = field.base ?? field
                      const fieldType = (field as any).$type as string
                      // Container fields always render - their children handle translatability
                      const CONTAINER_TYPES = ['CompositeField', 'SectionField', 'ElementField']
                      if (CONTAINER_TYPES.includes(fieldType)) {
                        return true
                      }
                      // Leaf fields only render if marked translatable
                      return !!base.translatable
                    }
                    return true
                  })
                  .map((field: any) => {
                    const fieldName = (field.base?.name ?? field.name) as string
                    const staleness = getFieldStaleness(fieldName)
                    const staleInfo = staleness === 'stale' ? getStaleFieldInfo(fieldName) : null
                    return (
                      <div key={fieldName}>
                        <ArchetypeFieldRenderer
                          field={field}
                          value={properties[fieldName]}
                          error={errors[fieldName]}
                          onChange={(val) => handlePropertyChange(fieldName, val)}
                          translationMode={!!isTranslationMode}
                          originalValue={originalProperties[fieldName]}
                          defaultLanguage={defaultLanguage}
                          repo={repo}
                          branch={branch}
                          staleness={staleness}
                          staleInfo={staleInfo}
                        />
                      </div>
                    )
                  })}
              </div>
              {isTranslationMode &&
                resolvedArchetype!.resolved_fields.filter(
                  (f: any) => !!(f.base ?? f).translatable
                ).length === 0 && (
                  <p className="text-zinc-500 text-sm">
                    No translatable fields defined for this archetype.
                  </p>
                )}
            </div>
          )}

          {/* NodeType properties (shown as primary when no archetype, secondary otherwise) */}
          {nodeType?.resolved_properties && Array.isArray(nodeType.resolved_properties) && nodeType.resolved_properties.length > 0 && (() => {
            const filteredProps = nodeType.resolved_properties.filter((propSchema: any) => {
              if (hasArchetypeFields && archetypeFieldNames.has(propSchema.name)) return false
              if (isTranslationMode && !isSchemaTranslatable(propSchema)) return false
              return true
            })

            // Group by source when mixins are present
            const hasMixinProps = mixins.length > 0 && propertySourceMap.size > 0
            const ownProps = hasMixinProps ? filteredProps.filter((p: any) => !propertySourceMap.has(p.name)) : filteredProps
            const mixinGroups = hasMixinProps
              ? mixins.reduce((acc, mixinName) => {
                  const props = filteredProps.filter((p: any) => propertySourceMap.get(p.name) === mixinName)
                  if (props.length > 0) acc.push({ name: mixinName, props })
                  return acc
                }, [] as { name: string; props: any[] }[])
              : []

            return (
              <div className="bg-white/5 rounded-xl p-6">
                <h3 className="text-lg font-semibold text-white mb-4">
                  {hasArchetypeFields
                    ? 'Additional Properties'
                    : isTranslationMode
                      ? 'Translation'
                      : 'Properties'}
                </h3>

                {hasMixinProps ? (
                  <div className="space-y-6">
                    {ownProps.length > 0 && (
                      <div>
                        <h4 className="text-xs font-medium text-zinc-500 uppercase tracking-wider mb-3">Own Properties</h4>
                        <div className="space-y-4">
                          {ownProps.map((propSchema: any) => (
                            <div key={propSchema.name}>{renderPropertyField(propSchema.name, propSchema)}</div>
                          ))}
                        </div>
                      </div>
                    )}
                    {mixinGroups.map((group) => (
                      <div key={group.name}>
                        <h4 className="text-xs font-medium text-teal-400/70 uppercase tracking-wider mb-3 flex items-center gap-1.5">
                          <Puzzle className="w-3 h-3" />
                          From {group.name}
                        </h4>
                        <div className="space-y-4">
                          {group.props.map((propSchema: any) => (
                            <div key={propSchema.name}>{renderPropertyField(propSchema.name, propSchema)}</div>
                          ))}
                        </div>
                      </div>
                    ))}
                  </div>
                ) : (
                  <div className="space-y-4">
                    {filteredProps.map((propSchema: any) => (
                      <div key={propSchema.name}>{renderPropertyField(propSchema.name, propSchema)}</div>
                    ))}
                  </div>
                )}

                {!hasArchetypeFields && isTranslationMode && nodeType.resolved_properties.filter((p: any) => isSchemaTranslatable(p)).length === 0 && (
                  <p className="text-zinc-500 text-sm">No translatable properties defined for this node type.</p>
                )}
              </div>
            )
          })()}

          {/* Additional custom properties - hide in translation mode */}
          {!isTranslationMode && properties && Object.keys(properties).length > 0 && (
            <div className="bg-white/5 rounded-xl p-6">
              <h3 className="text-lg font-semibold text-white mb-4">Custom Properties</h3>
              <div className="space-y-4">
                {Object.entries(properties).map(([propName, propValue]) => {
                  // Skip properties already defined in schema or archetype
                  const isDefinedInSchema = nodeType?.resolved_properties?.some((p: any) => p.name === propName)
                  if (isDefinedInSchema) return null
                  if (archetypeFieldNames.has(propName)) return null

                  return (
                    <div key={propName}>
                      <label className="block text-sm font-medium text-gray-300 mb-2">{propName}</label>
                      <input
                        type="text"
                        value={typeof propValue === 'object' ? JSON.stringify(propValue) : String(propValue || '')}
                        onChange={(e) => {
                          try {
                            const val = JSON.parse(e.target.value)
                            handlePropertyChange(propName, val)
                          } catch {
                            handlePropertyChange(propName, e.target.value)
                          }
                        }}
                        className="w-full px-4 py-2 bg-white/10 border border-white/20 rounded-lg text-white focus:outline-none focus:ring-2 focus:ring-purple-500"
                      />
                    </div>
                  )
                })}
              </div>
            </div>
          )}
        </div>
      </div>
    </div>
  )
}
