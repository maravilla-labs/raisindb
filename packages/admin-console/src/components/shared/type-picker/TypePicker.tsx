/**
 * TypePicker - Generic type picker component with namespace tree grouping
 *
 * A dropdown component for selecting types (NodeTypes or Archetypes)
 * with virtual namespace tree grouping and search functionality.
 */

import {
  useState,
  useRef,
  useEffect,
  useLayoutEffect,
  useCallback,
  useMemo,
  useId,
  type ComponentType,
  type CSSProperties,
} from 'react'
import { createPortal } from 'react-dom'
import { ChevronDown, X, Search, FileType, Check, Asterisk } from 'lucide-react'
import type { PickableType, SelectionMode } from './types'
import { useTypePickerTree, flattenTreePaths } from './useTypePickerTree'
import TypePickerTree from './TypePickerTree'

interface TypePickerProps<T extends PickableType = PickableType> {
  // Data
  items: T[]
  loading?: boolean

  // Selection
  mode: SelectionMode
  value: string | string[]
  onChange: (value: string | string[]) => void

  // Special options
  allowWildcard?: boolean
  allowNone?: boolean
  noneLabel?: string
  wildcardLabel?: string

  // UI
  placeholder?: string
  disabled?: boolean
  className?: string
  maxHeight?: number

  // Item rendering
  itemIcon?: ComponentType<{ className?: string }>
  itemIconColor?: string

  // Validation
  error?: string
}

/**
 * Normalize value to Set for consistent comparison
 */
function normalizeToSet(value: string | string[]): Set<string> {
  if (Array.isArray(value)) {
    return new Set(value.filter(Boolean))
  }
  return value ? new Set([value]) : new Set()
}

export default function TypePicker<T extends PickableType>({
  items,
  loading = false,
  mode,
  value,
  onChange,
  allowWildcard = false,
  allowNone = false,
  noneLabel = 'None',
  wildcardLabel = 'Allow All (*)',
  placeholder,
  disabled = false,
  className = '',
  maxHeight = 300,
  itemIcon = FileType,
  itemIconColor = 'text-primary-400',
  error,
}: TypePickerProps<T>) {
  const id = useId()
  const containerRef = useRef<HTMLDivElement>(null)
  const triggerRef = useRef<HTMLButtonElement>(null)
  const dropdownRef = useRef<HTMLDivElement>(null)
  const searchInputRef = useRef<HTMLInputElement>(null)

  // State
  const [isOpen, setIsOpen] = useState(false)
  const [searchQuery, setSearchQuery] = useState('')
  const [expandedPaths, setExpandedPaths] = useState<Set<string>>(new Set())
  const [dropdownStyle, setDropdownStyle] = useState<CSSProperties>({})
  const [focusedPath, setFocusedPath] = useState<string | null>(null)

  // Build tree
  const { tree, filteredTree, searchExpandedPaths } = useTypePickerTree(items, searchQuery)

  // Merge search expanded paths with manually expanded paths
  const effectiveExpandedPaths = useMemo(
    () => new Set([...expandedPaths, ...searchExpandedPaths]),
    [expandedPaths, searchExpandedPaths]
  )

  // Flatten tree for keyboard navigation
  const flatPaths = useMemo(() => {
    const currentTree = searchQuery ? filteredTree : tree
    return flattenTreePaths(currentTree, effectiveExpandedPaths)
  }, [searchQuery, filteredTree, tree, effectiveExpandedPaths])

  // Selection as Set (memoized)
  const selectedValues = useMemo(() => normalizeToSet(value), [value])
  const hasWildcard = selectedValues.has('*')

  // Reset expanded paths when items change
  useEffect(() => {
    setExpandedPaths(new Set())
  }, [items])

  // Update dropdown position
  const updateDropdownPosition = useCallback(() => {
    if (containerRef.current) {
      const rect = containerRef.current.getBoundingClientRect()
      const viewportHeight = window.innerHeight
      const spaceBelow = viewportHeight - rect.bottom
      const spaceAbove = rect.top

      // Prefer showing below, but switch to above if not enough space
      const showAbove = spaceBelow < maxHeight && spaceAbove > spaceBelow

      setDropdownStyle({
        position: 'fixed',
        left: rect.left,
        width: rect.width,
        maxHeight: Math.min(maxHeight, showAbove ? spaceAbove - 8 : spaceBelow - 8),
        ...(showAbove ? { bottom: viewportHeight - rect.top + 4 } : { top: rect.bottom + 4 }),
        zIndex: 9999,
      })
    }
  }, [maxHeight])

  // Calculate dropdown position and add scroll/resize listeners
  useLayoutEffect(() => {
    if (!isOpen) return

    updateDropdownPosition()

    window.addEventListener('resize', updateDropdownPosition)
    window.addEventListener('scroll', updateDropdownPosition, true)
    return () => {
      window.removeEventListener('resize', updateDropdownPosition)
      window.removeEventListener('scroll', updateDropdownPosition, true)
    }
  }, [isOpen, updateDropdownPosition])

  // Focus search input when dropdown opens
  useEffect(() => {
    if (isOpen && searchInputRef.current) {
      searchInputRef.current.focus()
      setFocusedPath(null)
    }
  }, [isOpen])

  // Reset search and focus when closing
  useEffect(() => {
    if (!isOpen) {
      setSearchQuery('')
      setFocusedPath(null)
    }
  }, [isOpen])

  // Return focus to trigger when dropdown closes
  useEffect(() => {
    if (!isOpen && triggerRef.current) {
      // Only focus if dropdown was previously open and focused
      const wasDropdownFocused = dropdownRef.current?.contains(document.activeElement as Node)
      if (wasDropdownFocused) {
        triggerRef.current.focus()
      }
    }
  }, [isOpen])

  // Click outside to close
  useEffect(() => {
    if (!isOpen) return

    function handleClickOutside(e: MouseEvent) {
      if (
        containerRef.current &&
        !containerRef.current.contains(e.target as Node) &&
        dropdownRef.current &&
        !dropdownRef.current.contains(e.target as Node)
      ) {
        setIsOpen(false)
      }
    }

    document.addEventListener('mousedown', handleClickOutside)
    return () => document.removeEventListener('mousedown', handleClickOutside)
  }, [isOpen])

  // Handle selection
  const handleSelect = useCallback(
    (name: string) => {
      if (mode === 'single') {
        onChange(name)
        setIsOpen(false)
      } else {
        // Multi-select
        if (name === '*') {
          // Selecting wildcard clears everything else
          onChange(['*'])
        } else {
          const newValues = new Set(selectedValues)
          // Remove wildcard if selecting a specific type
          newValues.delete('*')

          if (newValues.has(name)) {
            newValues.delete(name)
          } else {
            newValues.add(name)
          }
          onChange(Array.from(newValues))
        }
      }
    },
    [mode, onChange, selectedValues]
  )

  // Handle none selection
  const handleSelectNone = useCallback(() => {
    if (mode === 'single') {
      onChange('')
      setIsOpen(false)
    }
  }, [mode, onChange])

  // Handle wildcard selection
  const handleSelectWildcard = useCallback(() => {
    if (mode === 'multi') {
      if (hasWildcard) {
        onChange([])
      } else {
        onChange(['*'])
      }
    }
  }, [mode, onChange, hasWildcard])

  // Clear selection
  const handleClear = useCallback(
    (e: React.MouseEvent) => {
      e.stopPropagation()
      if (mode === 'single') {
        onChange('')
      } else {
        onChange([])
      }
    },
    [mode, onChange]
  )

  // Remove a single item from multi-select
  const handleRemoveItem = useCallback(
    (e: React.MouseEvent, name: string) => {
      e.stopPropagation()
      if (mode === 'multi') {
        const newValues = Array.from(selectedValues).filter((v) => v !== name)
        onChange(newValues)
      }
    },
    [mode, onChange, selectedValues]
  )

  // Toggle expanded path
  const toggleExpanded = useCallback((path: string) => {
    setExpandedPaths((prev) => {
      const next = new Set(prev)
      if (next.has(path)) {
        next.delete(path)
      } else {
        next.add(path)
      }
      return next
    })
  }, [])

  // Keyboard navigation
  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (!isOpen) return

      switch (e.key) {
        case 'ArrowDown': {
          e.preventDefault()
          const currentIndex = focusedPath ? flatPaths.indexOf(focusedPath) : -1
          const nextIndex = Math.min(currentIndex + 1, flatPaths.length - 1)
          if (flatPaths[nextIndex]) {
            setFocusedPath(flatPaths[nextIndex])
          }
          break
        }
        case 'ArrowUp': {
          e.preventDefault()
          const currentIndex = focusedPath ? flatPaths.indexOf(focusedPath) : flatPaths.length
          const prevIndex = Math.max(currentIndex - 1, 0)
          if (flatPaths[prevIndex]) {
            setFocusedPath(flatPaths[prevIndex])
          }
          break
        }
        case 'Enter': {
          e.preventDefault()
          if (focusedPath) {
            handleSelect(focusedPath)
          }
          break
        }
        case 'Escape': {
          e.preventDefault()
          setIsOpen(false)
          break
        }
      }
    },
    [isOpen, focusedPath, flatPaths, handleSelect]
  )

  // Get display value for single select
  const getSingleDisplayValue = (): string => {
    if (!value || (typeof value === 'string' && !value)) {
      return ''
    }
    const v = typeof value === 'string' ? value : value[0]
    return v || ''
  }

  // Render trigger content
  const renderTriggerContent = () => {
    if (loading) {
      return (
        <span className="flex items-center gap-2 text-zinc-500">
          <span className="w-4 h-4 border-2 border-zinc-600 border-t-primary-500 rounded-full animate-spin" />
          Loading...
        </span>
      )
    }

    if (mode === 'single') {
      const displayValue = getSingleDisplayValue()
      if (!displayValue) {
        return <span className="text-zinc-500">{placeholder || 'Select...'}</span>
      }
      return <span className="truncate">{displayValue}</span>
    }

    // Multi-select
    if (selectedValues.size === 0) {
      return <span className="text-zinc-500">{placeholder || 'Select types...'}</span>
    }

    if (hasWildcard) {
      return (
        <span className="inline-flex items-center gap-1 px-2 py-0.5 bg-amber-500/20 text-amber-300 rounded text-xs">
          <Asterisk className="w-3 h-3" />
          All Types
        </span>
      )
    }

    // Show pills for selected items (max 3)
    const selectedArray = Array.from(selectedValues)
    const displayItems = selectedArray.slice(0, 3)
    const remainingCount = selectedArray.length - 3

    return (
      <div className="flex flex-wrap gap-1">
        {displayItems.map((name) => (
          <span
            key={name}
            className="inline-flex items-center gap-1 px-2 py-0.5 bg-primary-500/20 text-primary-300 rounded text-xs"
          >
            {name}
            <button
              type="button"
              onClick={(e) => handleRemoveItem(e, name)}
              className="hover:bg-white/10 rounded p-0.5"
              aria-label={`Remove ${name}`}
            >
              <X className="w-3 h-3" />
            </button>
          </span>
        ))}
        {remainingCount > 0 && (
          <span className="px-2 py-0.5 bg-zinc-700 text-zinc-300 rounded text-xs">
            +{remainingCount} more
          </span>
        )}
      </div>
    )
  }

  const listboxId = `${id}-listbox`
  const errorId = `${id}-error`

  return (
    <div ref={containerRef} className={`relative ${className}`}>
      {/* Trigger Button */}
      <button
        ref={triggerRef}
        type="button"
        onClick={() => !disabled && setIsOpen(!isOpen)}
        onKeyDown={(e) => {
          if (e.key === 'ArrowDown' && !isOpen) {
            e.preventDefault()
            setIsOpen(true)
          }
        }}
        disabled={disabled}
        aria-haspopup="listbox"
        aria-expanded={isOpen}
        aria-controls={isOpen ? listboxId : undefined}
        aria-invalid={!!error}
        aria-describedby={error ? errorId : undefined}
        className={`
          relative w-full flex items-center gap-2 px-3 py-2 min-h-[38px]
          bg-white/5 border rounded-lg
          text-left text-sm text-white
          transition-colors
          ${disabled ? 'opacity-50 cursor-not-allowed' : 'hover:bg-white/10'}
          ${error ? 'border-red-500/50' : 'border-white/20'}
          ${isOpen ? 'ring-2 ring-primary-500/50' : ''}
        `}
      >
        <div className="flex-1 min-w-0">{renderTriggerContent()}</div>

        {/* Clear button for single select with value */}
        {mode === 'single' && getSingleDisplayValue() && !disabled && (
          <button
            type="button"
            onClick={handleClear}
            className="p-1 hover:bg-white/20 rounded transition-colors"
            aria-label="Clear selection"
          >
            <X className="w-3 h-3" />
          </button>
        )}

        {/* Clear all for multi select */}
        {mode === 'multi' && selectedValues.size > 0 && !disabled && (
          <button
            type="button"
            onClick={handleClear}
            className="p-1 hover:bg-white/20 rounded transition-colors"
            aria-label="Clear all selections"
          >
            <X className="w-3 h-3" />
          </button>
        )}

        <ChevronDown
          className={`w-4 h-4 text-zinc-400 transition-transform flex-shrink-0 ${
            isOpen ? 'rotate-180' : ''
          }`}
        />
      </button>

      {/* Error message */}
      {error && (
        <p id={errorId} className="text-xs text-red-400 mt-1" role="alert">
          {error}
        </p>
      )}

      {/* Dropdown */}
      {isOpen &&
        createPortal(
          <>
            {/* Backdrop */}
            <div
              className="fixed inset-0 z-[9998]"
              onClick={() => setIsOpen(false)}
              aria-hidden="true"
            />

            {/* Dropdown panel */}
            <div
              ref={dropdownRef}
              id={listboxId}
              role="listbox"
              aria-multiselectable={mode === 'multi'}
              style={dropdownStyle}
              onKeyDown={handleKeyDown}
              className="bg-zinc-900 border border-white/20 rounded-lg shadow-xl overflow-hidden flex flex-col"
            >
              {/* Search input */}
              <div className="p-2 border-b border-white/10">
                <div className="relative">
                  <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 w-4 h-4 text-zinc-500" />
                  <input
                    ref={searchInputRef}
                    type="text"
                    value={searchQuery}
                    onChange={(e) => setSearchQuery(e.target.value)}
                    placeholder="Search types..."
                    aria-label="Search types"
                    className="w-full pl-8 pr-3 py-1.5 bg-white/5 border border-white/10 rounded text-sm text-white placeholder-zinc-500 focus:outline-none focus:ring-1 focus:ring-primary-500/50"
                  />
                  {searchQuery && (
                    <button
                      type="button"
                      onClick={() => setSearchQuery('')}
                      className="absolute right-2 top-1/2 -translate-y-1/2 p-0.5 hover:bg-white/10 rounded"
                      aria-label="Clear search"
                    >
                      <X className="w-3 h-3 text-zinc-400" />
                    </button>
                  )}
                </div>
              </div>

              {/* Options list */}
              <div className="flex-1 overflow-y-auto">
                {/* None option (single select only) */}
                {allowNone && mode === 'single' && (
                  <button
                    type="button"
                    onClick={handleSelectNone}
                    role="option"
                    aria-selected={!getSingleDisplayValue()}
                    className={`
                      w-full flex items-center gap-2 px-3 py-2 text-sm text-left transition-colors
                      ${!getSingleDisplayValue() ? 'bg-primary-500/20 text-primary-300' : 'text-zinc-300 hover:bg-white/5'}
                    `}
                  >
                    <div className="w-4 h-4" /> {/* Spacer for alignment */}
                    <span className="flex-1">{noneLabel}</span>
                    {!getSingleDisplayValue() && <Check className="w-4 h-4 text-primary-400" />}
                  </button>
                )}

                {/* Wildcard option (multi select only) */}
                {allowWildcard && mode === 'multi' && (
                  <button
                    type="button"
                    onClick={handleSelectWildcard}
                    role="option"
                    aria-selected={hasWildcard}
                    className={`
                      w-full flex items-center gap-2 px-3 py-2 text-sm text-left transition-colors border-b border-white/10
                      ${hasWildcard ? 'bg-amber-500/20 text-amber-300' : 'text-zinc-300 hover:bg-white/5'}
                    `}
                  >
                    <div
                      className={`
                        w-4 h-4 rounded border flex items-center justify-center flex-shrink-0
                        ${hasWildcard ? 'bg-amber-500 border-amber-500' : 'border-white/30'}
                      `}
                    >
                      {hasWildcard && <Check className="w-3 h-3 text-white" />}
                    </div>
                    <Asterisk className="w-4 h-4 text-amber-400" />
                    <span className="flex-1">{wildcardLabel}</span>
                  </button>
                )}

                {/* Tree */}
                <TypePickerTree
                  tree={searchQuery ? filteredTree : tree}
                  expandedPaths={effectiveExpandedPaths}
                  onToggleExpand={toggleExpanded}
                  selectedValues={selectedValues}
                  onSelect={handleSelect}
                  mode={mode}
                  itemIcon={itemIcon}
                  itemIconColor={itemIconColor}
                  focusedPath={focusedPath}
                />

                {/* Loading state */}
                {loading && (
                  <div className="px-3 py-4 text-sm text-zinc-500 text-center flex items-center justify-center gap-2">
                    <span className="w-4 h-4 border-2 border-zinc-600 border-t-primary-500 rounded-full animate-spin" />
                    Loading types...
                  </div>
                )}

                {/* Empty state */}
                {!loading && items.length === 0 && (
                  <div className="px-3 py-4 text-sm text-zinc-500 text-center">
                    No types available
                  </div>
                )}
              </div>
            </div>
          </>,
          document.body
        )}
    </div>
  )
}
