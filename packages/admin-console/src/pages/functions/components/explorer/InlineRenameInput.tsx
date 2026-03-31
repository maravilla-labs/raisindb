/**
 * Inline Rename Input Component
 *
 * A text input for inline renaming of tree nodes.
 * - Auto-focus and auto-select text on mount
 * - Enter to save, Escape to cancel
 * - Click-outside to cancel
 * - Validates: starts with letter, alphanumeric + underscore/hyphen (+ dots for files)
 */

import { useState, useRef, useEffect, useCallback } from 'react'

// Regex for folders/functions (no dots allowed)
const NAME_REGEX = /^[a-zA-Z][a-zA-Z0-9_-]*$/
// Regex for files (dots allowed for extensions)
const FILE_NAME_REGEX = /^[a-zA-Z][a-zA-Z0-9_.-]*$/

export interface InlineRenameInputProps {
  initialValue: string
  onSave: (newName: string) => void
  onCancel: () => void
  /** Whether this is a file (allows dots in name for extensions) */
  isFile?: boolean
}

export function InlineRenameInput({ initialValue, onSave, onCancel, isFile = false }: InlineRenameInputProps) {
  const [value, setValue] = useState(initialValue)
  const [error, setError] = useState<string | null>(null)
  const inputRef = useRef<HTMLInputElement>(null)

  // Auto-focus and select all text on mount
  useEffect(() => {
    if (inputRef.current) {
      inputRef.current.focus()
      inputRef.current.select()
    }
  }, [])

  // Handle click outside to cancel
  useEffect(() => {
    const handleClickOutside = (e: MouseEvent) => {
      if (inputRef.current && !inputRef.current.contains(e.target as Node)) {
        onCancel()
      }
    }

    // Use capture phase to intercept before other handlers
    document.addEventListener('mousedown', handleClickOutside, true)
    return () => {
      document.removeEventListener('mousedown', handleClickOutside, true)
    }
  }, [onCancel])

  const validate = useCallback((name: string): string | null => {
    if (!name.trim()) {
      return 'Name cannot be empty'
    }
    // Use different regex for files vs folders/functions
    const regex = isFile ? FILE_NAME_REGEX : NAME_REGEX
    if (!regex.test(name)) {
      return isFile
        ? 'Must start with a letter, use only letters, numbers, underscore, hyphen, or dot'
        : 'Must start with a letter, use only letters, numbers, underscore, or hyphen'
    }
    return null
  }, [isFile])

  const handleChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const newValue = e.target.value
    setValue(newValue)
    setError(validate(newValue))
  }

  const handleKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'Enter') {
      e.preventDefault()
      const validationError = validate(value)
      if (validationError) {
        setError(validationError)
        return
      }
      if (value !== initialValue) {
        onSave(value)
      } else {
        onCancel()
      }
    } else if (e.key === 'Escape') {
      e.preventDefault()
      onCancel()
    }
  }

  const handleBlur = () => {
    // Let click-outside handler manage this
  }

  return (
    <div className="flex-1 relative">
      <input
        ref={inputRef}
        type="text"
        value={value}
        onChange={handleChange}
        onKeyDown={handleKeyDown}
        onBlur={handleBlur}
        className={`
          w-full px-1 py-0 text-sm bg-white/10 border rounded
          text-white outline-none
          ${error ? 'border-red-500' : 'border-primary-500'}
        `}
        onClick={(e) => e.stopPropagation()}
      />
      {error && (
        <div className="absolute left-0 top-full mt-1 px-2 py-1 text-xs bg-red-900 text-red-200 rounded shadow-lg whitespace-nowrap z-50">
          {error}
        </div>
      )}
    </div>
  )
}
