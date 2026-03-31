/**
 * Searchable Multi-Select Dropdown Component
 *
 * A dropdown with search input that allows selecting multiple items.
 * Displays selected items as tags.
 */

import { useState, useRef, useEffect } from 'react'
import { ChevronDown, X, Search, Check } from 'lucide-react'

export interface SelectOption {
  value: string
  label: string
  description?: string
}

interface SearchableMultiSelectProps {
  /** Available options to select from */
  options: SelectOption[]
  /** Currently selected values */
  selected: string[]
  /** Callback when selection changes */
  onChange: (selected: string[]) => void
  /** Placeholder text when nothing selected */
  placeholder?: string
  /** Maximum height of dropdown in pixels */
  maxHeight?: number
  /** Whether the component is disabled */
  disabled?: boolean
  /** Allow creating new options */
  allowCreate?: boolean
  /** Callback when creating new option */
  onCreate?: (value: string) => void
}

export function SearchableMultiSelect({
  options,
  selected,
  onChange,
  placeholder = 'Select items...',
  maxHeight = 200,
  disabled = false,
  allowCreate = false,
  onCreate,
}: SearchableMultiSelectProps) {
  const [isOpen, setIsOpen] = useState(false)
  const [searchQuery, setSearchQuery] = useState('')
  const containerRef = useRef<HTMLDivElement>(null)
  const inputRef = useRef<HTMLInputElement>(null)

  // Close dropdown when clicking outside
  useEffect(() => {
    function handleClickOutside(event: MouseEvent) {
      if (containerRef.current && !containerRef.current.contains(event.target as Node)) {
        setIsOpen(false)
      }
    }
    document.addEventListener('mousedown', handleClickOutside)
    return () => document.removeEventListener('mousedown', handleClickOutside)
  }, [])

  // Focus input when dropdown opens
  useEffect(() => {
    if (isOpen && inputRef.current) {
      inputRef.current.focus()
    }
  }, [isOpen])

  const filteredOptions = options.filter(
    (option) =>
      option.label.toLowerCase().includes(searchQuery.toLowerCase()) ||
      option.value.toLowerCase().includes(searchQuery.toLowerCase())
  )

  const toggleOption = (value: string) => {
    if (selected.includes(value)) {
      onChange(selected.filter((v) => v !== value))
    } else {
      onChange([...selected, value])
    }
  }

  const removeOption = (value: string) => {
    onChange(selected.filter((v) => v !== value))
  }

  const handleCreateNew = () => {
    if (allowCreate && onCreate && searchQuery.trim()) {
      onCreate(searchQuery.trim())
      setSearchQuery('')
    }
  }

  const selectedOptions = options.filter((o) => selected.includes(o.value))
  const showCreateOption =
    allowCreate &&
    searchQuery.trim() &&
    !options.some((o) => o.value.toLowerCase() === searchQuery.toLowerCase().trim())

  return (
    <div ref={containerRef} className="relative">
      {/* Selected tags display */}
      <div
        onClick={() => !disabled && setIsOpen(!isOpen)}
        className={`
          min-h-[40px] px-3 py-2 bg-white/5 border border-white/20 rounded-lg
          flex flex-wrap gap-1.5 items-center cursor-pointer
          ${disabled ? 'opacity-50 cursor-not-allowed' : 'hover:border-white/30'}
          ${isOpen ? 'ring-2 ring-primary-500/50 border-primary-500' : ''}
        `}
      >
        {selectedOptions.length === 0 ? (
          <span className="text-sm text-zinc-500">{placeholder}</span>
        ) : (
          selectedOptions.map((option) => (
            <span
              key={option.value}
              className="inline-flex items-center gap-1 px-2 py-0.5 bg-primary-500/20 text-primary-300 text-xs rounded-full"
            >
              {option.label}
              <button
                onClick={(e) => {
                  e.stopPropagation()
                  removeOption(option.value)
                }}
                className="hover:bg-primary-500/30 rounded-full p-0.5"
              >
                <X className="w-3 h-3" />
              </button>
            </span>
          ))
        )}
        <ChevronDown
          className={`w-4 h-4 text-zinc-400 ml-auto transition-transform ${isOpen ? 'rotate-180' : ''}`}
        />
      </div>

      {/* Dropdown */}
      {isOpen && (
        <div className="absolute z-50 mt-1 w-full bg-zinc-800 border border-white/20 rounded-lg shadow-xl overflow-hidden">
          {/* Search input */}
          <div className="p-2 border-b border-white/10">
            <div className="relative">
              <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 w-4 h-4 text-zinc-400" />
              <input
                ref={inputRef}
                type="text"
                value={searchQuery}
                onChange={(e) => setSearchQuery(e.target.value)}
                placeholder="Search..."
                className="w-full pl-8 pr-3 py-1.5 bg-white/5 border border-white/10 rounded text-sm text-white placeholder-zinc-500 focus:outline-none focus:border-primary-400"
                onKeyDown={(e) => {
                  if (e.key === 'Enter' && showCreateOption) {
                    handleCreateNew()
                  }
                }}
              />
            </div>
          </div>

          {/* Options list */}
          <div className="overflow-y-auto" style={{ maxHeight }}>
            {filteredOptions.length === 0 && !showCreateOption ? (
              <div className="px-3 py-4 text-sm text-zinc-500 text-center">
                No options found
              </div>
            ) : (
              <>
                {filteredOptions.map((option) => {
                  const isSelected = selected.includes(option.value)
                  return (
                    <button
                      key={option.value}
                      onClick={() => toggleOption(option.value)}
                      className={`
                        w-full px-3 py-2 text-left flex items-start gap-2 transition-colors
                        ${isSelected ? 'bg-primary-500/20' : 'hover:bg-white/5'}
                      `}
                    >
                      <div
                        className={`
                        mt-0.5 w-4 h-4 rounded border flex items-center justify-center flex-shrink-0
                        ${isSelected ? 'bg-primary-500 border-primary-500' : 'border-white/30'}
                      `}
                      >
                        {isSelected && <Check className="w-3 h-3 text-white" />}
                      </div>
                      <div className="flex-1 min-w-0">
                        <div className="text-sm text-white truncate">{option.label}</div>
                        {option.description && (
                          <div className="text-xs text-zinc-500 truncate">{option.description}</div>
                        )}
                      </div>
                    </button>
                  )
                })}
                {showCreateOption && (
                  <button
                    onClick={handleCreateNew}
                    className="w-full px-3 py-2 text-left flex items-center gap-2 hover:bg-white/5 text-primary-400"
                  >
                    <span className="text-sm">Create "{searchQuery.trim()}"</span>
                  </button>
                )}
              </>
            )}
          </div>
        </div>
      )}
    </div>
  )
}
