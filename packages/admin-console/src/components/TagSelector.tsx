import { useEffect, useMemo, useState, useRef, useLayoutEffect, KeyboardEvent } from 'react'
import { createPortal } from 'react-dom'
import { X } from 'lucide-react'

interface TagSelectorProps {
  value: string[]
  onChange: (value: string[]) => void
  placeholder?: string
  label?: string
  suggestions?: string[]
  allowCustom?: boolean
  invalidValues?: string[]
  helperText?: string
  error?: string
}

export default function TagSelector({
  value,
  onChange,
  placeholder = 'Add item...',
  label,
  suggestions = [],
  allowCustom = true,
  invalidValues = [],
  helperText = 'Press Enter to add, Backspace to remove',
  error,
}: TagSelectorProps) {
  const [inputValue, setInputValue] = useState('')
  const [showSuggestions, setShowSuggestions] = useState(false)
  const [internalError, setInternalError] = useState<string | null>(null)
  const [activeSuggestionIndex, setActiveSuggestionIndex] = useState<number>(-1)
  const containerRef = useRef<HTMLDivElement>(null)
  const [dropdownStyle, setDropdownStyle] = useState<React.CSSProperties>({})

  const normalizedSuggestions = useMemo(() => new Set(suggestions.map((s) => s.toLowerCase())), [
    suggestions,
  ])
  const invalidValueSet = useMemo(() => new Set(invalidValues), [invalidValues])

  const filteredSuggestions = useMemo(
    () =>
      suggestions.filter(
        (s) => !value.includes(s) && s.toLowerCase().includes(inputValue.toLowerCase())
      ),
    [suggestions, value, inputValue]
  )

  useEffect(() => {
    setActiveSuggestionIndex(-1)
  }, [inputValue, suggestions])

  // Calculate dropdown position when showing suggestions
  useLayoutEffect(() => {
    if (showSuggestions && filteredSuggestions.length > 0 && containerRef.current) {
      const rect = containerRef.current.getBoundingClientRect()
      const viewportHeight = window.innerHeight
      const spaceBelow = viewportHeight - rect.bottom
      const spaceAbove = rect.top
      const dropdownHeight = Math.min(filteredSuggestions.length * 40, 192) // max-h-48 = 192px

      // Prefer showing below, but flip to above if not enough space
      const showAbove = spaceBelow < dropdownHeight && spaceAbove > spaceBelow

      setDropdownStyle({
        position: 'fixed',
        left: rect.left,
        width: rect.width,
        ...(showAbove
          ? { bottom: viewportHeight - rect.top + 4 }
          : { top: rect.bottom + 4 }),
        zIndex: 9999,
      })
    }
  }, [showSuggestions, filteredSuggestions.length])

  function canAdd(tag: string) {
    if (allowCustom) return true
    if (tag.length === 0) return false
    return normalizedSuggestions.has(tag.toLowerCase())
  }

  function handleKeyDown(e: KeyboardEvent<HTMLInputElement>) {
    if (e.key === 'ArrowDown') {
      if (filteredSuggestions.length === 0) return
      e.preventDefault()
      setShowSuggestions(true)
      setActiveSuggestionIndex((prev) => {
        const next = prev + 1
        if (next >= filteredSuggestions.length) return 0
        return next
      })
      return
    }

    if (e.key === 'ArrowUp') {
      if (filteredSuggestions.length === 0) return
      e.preventDefault()
      setShowSuggestions(true)
      setActiveSuggestionIndex((prev) => {
        if (prev <= 0) return filteredSuggestions.length - 1
        return prev - 1
      })
      return
    }

    if (e.key === 'Escape') {
      setShowSuggestions(false)
      setActiveSuggestionIndex(-1)
      setInternalError(null)
      return
    }

    if (e.key === 'Enter') {
      e.preventDefault()
      if (
        activeSuggestionIndex >= 0 &&
        activeSuggestionIndex < filteredSuggestions.length
      ) {
        handleSuggestionClick(filteredSuggestions[activeSuggestionIndex])
        return
      }
      const trimmed = inputValue.trim()
      if (!value.includes(trimmed)) {
        if (!canAdd(trimmed)) {
          setInternalError('Select an existing option from the list.')
          setShowSuggestions(true)
          return
        }
        onChange([...value, trimmed])
      }
      setInputValue('')
      setShowSuggestions(false)
      setActiveSuggestionIndex(-1)
      setInternalError(null)
    } else if (e.key === 'Backspace' && !inputValue && value.length > 0) {
      onChange(value.slice(0, -1))
      setInternalError(null)
      setActiveSuggestionIndex(-1)
    }
  }

  function handleRemove(item: string) {
    onChange(value.filter((v) => v !== item))
    setInternalError(null)
    setActiveSuggestionIndex(-1)
  }

  function handleSuggestionClick(suggestion: string) {
    if (!canAdd(suggestion)) {
      setInternalError('Select an existing option from the list.')
      return
    }
    onChange([...value, suggestion])
    setInputValue('')
    setShowSuggestions(false)
    setActiveSuggestionIndex(-1)
    setInternalError(null)
  }

  const feedbackMessage = error || internalError

  const dropdownContent =
    showSuggestions && filteredSuggestions.length > 0
      ? createPortal(
          <div
            style={dropdownStyle}
            className="bg-zinc-900 border border-white/10 rounded-lg shadow-xl max-h-48 overflow-y-auto"
          >
            {filteredSuggestions.map((suggestion, index) => (
              <button
                key={suggestion}
                type="button"
                onClick={() => handleSuggestionClick(suggestion)}
                onMouseEnter={() => setActiveSuggestionIndex(index)}
                className={`w-full text-left px-4 py-2 text-zinc-300 transition-colors ${
                  activeSuggestionIndex === index
                    ? 'bg-primary-500/30 text-primary-100'
                    : 'hover:bg-primary-500/20'
                }`}
              >
                {suggestion}
              </button>
            ))}
          </div>,
          document.body
        )
      : null

  return (
    <div className="space-y-2">
      {label && <label className="block text-sm font-medium text-zinc-300">{label}</label>}
      <div ref={containerRef}>
        <div className="flex flex-wrap gap-2 p-3 bg-white/5 border border-white/10 rounded-lg focus-within:border-primary-500 transition-colors">
          {value.map((item) => (
            <span
              key={item}
              className={`flex items-center gap-1 px-2 py-1 text-sm rounded-full ${
                invalidValueSet.has(item)
                  ? 'bg-red-500/20 text-red-200 border border-red-500/40'
                  : 'bg-primary-500/20 text-primary-300'
              }`}
              title={invalidValueSet.has(item) ? 'Value not found in available options' : undefined}
            >
              {item}
              <button
                type="button"
                onClick={() => handleRemove(item)}
                className="hover:text-primary-100 transition-colors"
              >
                <X className="w-3 h-3" />
              </button>
            </span>
          ))}
          <input
            type="text"
            value={inputValue}
            onChange={(e) => {
              setInputValue(e.target.value)
              setShowSuggestions(true)
              if (internalError) {
                setInternalError(null)
              }
            }}
            onKeyDown={handleKeyDown}
            onFocus={() => setShowSuggestions(true)}
            onBlur={() => setTimeout(() => setShowSuggestions(false), 200)}
            placeholder={value.length === 0 ? placeholder : ''}
            className="flex-1 min-w-[120px] bg-transparent text-white placeholder-zinc-500 outline-none"
          />
        </div>
        {dropdownContent}
      </div>
      {feedbackMessage ? (
        <p className="text-xs text-red-400">{feedbackMessage}</p>
      ) : helperText ? (
        <p className="text-xs text-zinc-500">{helperText}</p>
      ) : null}
    </div>
  )
}
