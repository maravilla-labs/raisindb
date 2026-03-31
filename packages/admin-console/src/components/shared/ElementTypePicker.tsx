/**
 * ElementTypePicker - Wrapper for selecting Element Types
 *
 * Fetches published element types from the API and wraps TypePicker
 * with element type-specific configuration.
 */

import { useState, useEffect } from 'react'
import { useParams } from 'react-router-dom'
import { Shapes } from 'lucide-react'
import { TypePicker, type SelectionMode, type PickableType } from './type-picker'
import { elementTypesApi, type ElementType } from '../../api/elementtypes'

interface ElementTypePickerProps {
  mode: SelectionMode
  value: string | string[]
  onChange: (value: string | string[]) => void
  allowNone?: boolean
  noneLabel?: string
  placeholder?: string
  disabled?: boolean
  className?: string
  error?: string
  /** Exclude these element type names from the list (e.g., to prevent self-reference) */
  excludeNames?: string[]
  /** If true, only show published element types. Default: false (show all) */
  publishedOnly?: boolean
}

/**
 * Convert ElementType to PickableType
 */
function elementTypeToPickable(et: ElementType): PickableType {
  return {
    name: et.name,
    description: et.description || et.title,
    icon: et.icon,
  }
}

export default function ElementTypePicker({
  mode,
  value,
  onChange,
  allowNone = false,
  noneLabel = 'None',
  placeholder,
  disabled = false,
  className = '',
  error,
  excludeNames = [],
  publishedOnly = false,
}: ElementTypePickerProps) {
  const { repo, branch } = useParams<{ repo: string; branch?: string }>()
  const activeBranch = branch || 'main'

  const [elementTypes, setElementTypes] = useState<ElementType[]>([])
  const [loading, setLoading] = useState(true)
  const [loadError, setLoadError] = useState<string | null>(null)

  useEffect(() => {
    async function loadElementTypes() {
      if (!repo) {
        setLoading(false)
        return
      }

      try {
        setLoading(true)
        setLoadError(null)
        // Use list (all) or listPublished based on publishedOnly prop
        const data = publishedOnly
          ? await elementTypesApi.listPublished(repo, activeBranch)
          : await elementTypesApi.list(repo, activeBranch)
        setElementTypes(data)
      } catch (err) {
        console.error('Failed to load element types:', err)
        setLoadError('Failed to load element types')
      } finally {
        setLoading(false)
      }
    }

    loadElementTypes()
  }, [repo, activeBranch, publishedOnly])

  // Filter out excluded names and convert to pickable items
  const items: PickableType[] = elementTypes
    .filter((et) => !excludeNames.includes(et.name))
    .map(elementTypeToPickable)

  return (
    <TypePicker
      items={items}
      loading={loading}
      mode={mode}
      value={value}
      onChange={onChange}
      allowNone={allowNone}
      noneLabel={noneLabel}
      placeholder={placeholder || (mode === 'single' ? 'Select element type...' : 'Select element types...')}
      disabled={disabled}
      className={className}
      itemIcon={Shapes}
      itemIconColor="text-blue-400"
      error={error || loadError || undefined}
    />
  )
}
