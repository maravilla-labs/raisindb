/**
 * NodeTypePicker - Wrapper for selecting NodeTypes
 *
 * Fetches published node types from the API and wraps TypePicker
 * with nodetype-specific configuration.
 */

import { useState, useEffect } from 'react'
import { useParams } from 'react-router-dom'
import { FileType } from 'lucide-react'
import { TypePicker, type SelectionMode, type PickableType } from './type-picker'
import { nodeTypesApi, type NodeType } from '../../api/nodetypes'
import { mixinsApi } from '../../api/mixins'

interface NodeTypePickerProps {
  mode: SelectionMode
  value: string | string[]
  onChange: (value: string | string[]) => void
  allowWildcard?: boolean
  allowNone?: boolean
  noneLabel?: string
  placeholder?: string
  disabled?: boolean
  className?: string
  error?: string
  /** Exclude these type names from the list (e.g., to prevent self-reference) */
  excludeNames?: string[]
  /** If true, only show published node types. Default: false (show all) */
  publishedOnly?: boolean
  /** Which catalog to list from. Default: 'nodetypes'. Use 'mixins' to pick mixins. */
  source?: 'nodetypes' | 'mixins'
}

/**
 * Convert NodeType to PickableType
 */
function nodeTypeToPickable(nt: NodeType): PickableType {
  return {
    name: nt.name,
    description: nt.description,
    icon: nt.icon,
  }
}

export default function NodeTypePicker({
  mode,
  value,
  onChange,
  allowWildcard = false,
  allowNone = false,
  noneLabel = 'None (no parent)',
  placeholder,
  disabled = false,
  className = '',
  error,
  excludeNames = [],
  publishedOnly = false,
  source = 'nodetypes',
}: NodeTypePickerProps) {
  const { repo, branch } = useParams<{ repo: string; branch?: string }>()
  const activeBranch = branch || 'main'

  const [nodeTypes, setNodeTypes] = useState<NodeType[]>([])
  const [loading, setLoading] = useState(true)
  const [loadError, setLoadError] = useState<string | null>(null)

  useEffect(() => {
    async function loadNodeTypes() {
      if (!repo) {
        setLoading(false)
        return
      }

      try {
        setLoading(true)
        setLoadError(null)
        const catalog = source === 'mixins' ? mixinsApi : nodeTypesApi
        // Use list (all) or listPublished based on publishedOnly prop
        const data = publishedOnly
          ? await catalog.listPublished(repo, activeBranch)
          : await catalog.list(repo, activeBranch)
        setNodeTypes(data)
      } catch (err) {
        console.error('Failed to load types:', err)
        setLoadError('Failed to load types')
      } finally {
        setLoading(false)
      }
    }

    loadNodeTypes()
  }, [repo, activeBranch, publishedOnly, source])

  // Filter out excluded names and convert to pickable items
  const items: PickableType[] = nodeTypes
    .filter((nt) => !excludeNames.includes(nt.name))
    .map(nodeTypeToPickable)

  return (
    <TypePicker
      items={items}
      loading={loading}
      mode={mode}
      value={value}
      onChange={onChange}
      allowWildcard={allowWildcard}
      allowNone={allowNone}
      noneLabel={noneLabel}
      wildcardLabel="Allow All (*)"
      placeholder={placeholder || (mode === 'single' ? 'Select type...' : 'Select types...')}
      disabled={disabled}
      className={className}
      itemIcon={FileType}
      itemIconColor="text-primary-400"
      error={error || loadError || undefined}
    />
  )
}
