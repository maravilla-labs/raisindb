/**
 * ArchetypePicker - Wrapper for selecting Archetypes
 *
 * Fetches published archetypes from the API and wraps TypePicker
 * with archetype-specific configuration.
 */

import { useState, useEffect } from 'react'
import { useParams } from 'react-router-dom'
import { Layers } from 'lucide-react'
import { TypePicker, type SelectionMode, type PickableType } from './type-picker'
import { archetypesApi, type Archetype } from '../../api/archetypes'

interface ArchetypePickerProps {
  mode: SelectionMode
  value: string | string[]
  onChange: (value: string | string[]) => void
  allowNone?: boolean
  noneLabel?: string
  placeholder?: string
  disabled?: boolean
  className?: string
  error?: string
  /** Exclude these archetype names from the list (e.g., to prevent self-reference) */
  excludeNames?: string[]
  /** If true, only show published archetypes. Default: false (show all) */
  publishedOnly?: boolean
}

/**
 * Convert Archetype to PickableType
 */
function archetypeToPickable(arch: Archetype): PickableType {
  return {
    name: arch.name,
    description: arch.description || arch.title,
    icon: arch.icon,
  }
}

export default function ArchetypePicker({
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
}: ArchetypePickerProps) {
  const { repo, branch } = useParams<{ repo: string; branch?: string }>()
  const activeBranch = branch || 'main'

  const [archetypes, setArchetypes] = useState<Archetype[]>([])
  const [loading, setLoading] = useState(true)
  const [loadError, setLoadError] = useState<string | null>(null)

  useEffect(() => {
    async function loadArchetypes() {
      if (!repo) {
        setLoading(false)
        return
      }

      try {
        setLoading(true)
        setLoadError(null)
        // Use list (all) or listPublished based on publishedOnly prop
        const data = publishedOnly
          ? await archetypesApi.listPublished(repo, activeBranch)
          : await archetypesApi.list(repo, activeBranch)
        setArchetypes(data)
      } catch (err) {
        console.error('Failed to load archetypes:', err)
        setLoadError('Failed to load archetypes')
      } finally {
        setLoading(false)
      }
    }

    loadArchetypes()
  }, [repo, activeBranch, publishedOnly])

  // Filter out excluded names and convert to pickable items
  const items: PickableType[] = archetypes
    .filter((arch) => !excludeNames.includes(arch.name))
    .map(archetypeToPickable)

  return (
    <TypePicker
      items={items}
      loading={loading}
      mode={mode}
      value={value}
      onChange={onChange}
      allowNone={allowNone}
      noneLabel={noneLabel}
      placeholder={placeholder || (mode === 'single' ? 'Select archetype...' : 'Select archetypes...')}
      disabled={disabled}
      className={className}
      itemIcon={Layers}
      itemIconColor="text-purple-400"
      error={error || loadError || undefined}
    />
  )
}
