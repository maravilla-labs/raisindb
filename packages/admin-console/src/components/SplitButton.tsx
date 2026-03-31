import { useState, useRef, useEffect, ReactNode } from 'react'
import { createPortal } from 'react-dom'
import { ChevronDown, Loader2 } from 'lucide-react'

export interface SplitButtonOption<T extends string> {
  value: T
  label: string
  description?: string
  icon?: ReactNode
}

interface SplitButtonProps<T extends string> {
  /** Available options for the dropdown */
  options: SplitButtonOption<T>[]
  /** Currently selected/default value */
  defaultValue: T
  /** Called when an option is selected (triggers action) */
  onSelect: (value: T) => void
  /** Loading state */
  loading?: boolean
  /** Loading label (replaces button text when loading) */
  loadingLabel?: string
  /** Loading progress percentage (0-100) */
  loadingProgress?: number | null
  /** Loading status sub-text */
  loadingStatus?: string | null
  /** Disabled state */
  disabled?: boolean
  /** Button color variant */
  variant?: 'primary' | 'secondary' | 'success' | 'danger'
  /** Icon to show before the label */
  icon?: ReactNode
  /** Additional className */
  className?: string
}

const variantStyles = {
  primary: {
    button: 'bg-primary-500/20 hover:bg-primary-500/30 text-primary-400 border-primary-500/30',
    divider: 'bg-primary-400/30',
    active: 'bg-primary-500/30',
  },
  secondary: {
    button: 'bg-white/10 hover:bg-white/20 text-white border-white/20',
    divider: 'bg-white/20',
    active: 'bg-white/20',
  },
  success: {
    button: 'bg-green-500/20 hover:bg-green-500/30 text-green-400 border-green-500/30',
    divider: 'bg-green-400/30',
    active: 'bg-green-500/30',
  },
  danger: {
    button: 'bg-red-500/20 hover:bg-red-500/30 text-red-400 border-red-500/30',
    divider: 'bg-red-400/30',
    active: 'bg-red-500/30',
  },
}

export default function SplitButton<T extends string>({
  options,
  defaultValue,
  onSelect,
  loading = false,
  loadingLabel,
  loadingProgress,
  loadingStatus,
  disabled = false,
  variant = 'primary',
  icon,
  className = '',
}: SplitButtonProps<T>) {
  const [isOpen, setIsOpen] = useState(false)
  const [buttonRect, setButtonRect] = useState<DOMRect | null>(null)
  const buttonRef = useRef<HTMLButtonElement>(null)
  const dropdownRef = useRef<HTMLDivElement>(null)

  const selectedOption = options.find((o) => o.value === defaultValue) || options[0]
  const styles = variantStyles[variant]

  // Close dropdown when clicking outside
  useEffect(() => {
    function handleClickOutside(event: MouseEvent) {
      if (dropdownRef.current && !dropdownRef.current.contains(event.target as Node)) {
        setIsOpen(false)
      }
    }
    if (isOpen) {
      if (buttonRef.current) {
        setButtonRect(buttonRef.current.getBoundingClientRect())
      }
      document.addEventListener('mousedown', handleClickOutside)
      return () => document.removeEventListener('mousedown', handleClickOutside)
    }
  }, [isOpen])

  // Close dropdown on escape
  useEffect(() => {
    function handleEscape(event: KeyboardEvent) {
      if (event.key === 'Escape') {
        setIsOpen(false)
      }
    }
    if (isOpen) {
      document.addEventListener('keydown', handleEscape)
      return () => document.removeEventListener('keydown', handleEscape)
    }
  }, [isOpen])

  return (
    <div className={`relative inline-flex ${className}`}>
      {/* Main action button */}
      <button
        onClick={() => onSelect(defaultValue)}
        disabled={disabled || loading}
        className={`
          flex items-center gap-2 px-4 py-2 rounded-l-lg transition-colors
          ${styles.button}
          disabled:opacity-50 disabled:cursor-not-allowed
        `}
      >
        {loading ? (
          <>
            <Loader2 className="w-5 h-5 animate-spin" />
            <span className="flex flex-col items-start">
              <span>
                {loadingLabel || selectedOption.label}
                {loadingProgress !== null && loadingProgress !== undefined && ` ${loadingProgress}%`}
              </span>
              {loadingStatus && <span className="text-xs opacity-70">{loadingStatus}</span>}
            </span>
          </>
        ) : (
          <>
            {icon || selectedOption.icon}
            {selectedOption.label}
          </>
        )}
      </button>

      {/* Divider */}
      <div className={`w-px ${styles.divider}`} />

      {/* Dropdown toggle */}
      <button
        ref={buttonRef}
        onClick={() => setIsOpen(!isOpen)}
        disabled={disabled || loading}
        className={`
          flex items-center px-2 py-2 rounded-r-lg transition-colors
          ${styles.button}
          disabled:opacity-50 disabled:cursor-not-allowed
        `}
      >
        <ChevronDown
          className={`w-4 h-4 transition-transform ${isOpen ? 'rotate-180' : ''}`}
        />
      </button>

      {/* Dropdown menu (portal) */}
      {isOpen &&
        buttonRect &&
        createPortal(
          <div
            ref={dropdownRef}
            className="fixed w-80 bg-zinc-900 border border-white/20 rounded-lg shadow-2xl overflow-hidden"
            style={{
              top: `${buttonRect.bottom + 8}px`,
              left: `${Math.max(8, buttonRect.right - 320)}px`,
              zIndex: 9999,
            }}
          >
            <div className="p-2">
              <div className="text-xs text-zinc-500 px-3 py-1.5 font-medium uppercase tracking-wider">
                Choose install mode
              </div>
              {options.map((option) => (
                <button
                  key={option.value}
                  onClick={() => {
                    onSelect(option.value)
                    setIsOpen(false)
                  }}
                  className={`
                    w-full flex flex-col items-start gap-1 p-3 rounded-lg text-left transition-colors
                    hover:bg-white/10
                    ${option.value === defaultValue ? styles.active : ''}
                  `}
                >
                  <div className="flex items-center gap-2 text-white font-medium">
                    {option.icon}
                    {option.label}
                  </div>
                  {option.description && (
                    <p className="text-xs text-zinc-400 leading-relaxed">{option.description}</p>
                  )}
                </button>
              ))}
            </div>
          </div>,
          document.body
        )}
    </div>
  )
}
