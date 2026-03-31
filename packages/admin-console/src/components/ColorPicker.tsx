import { useState, useRef, useEffect } from 'react'
import { createPortal } from 'react-dom'
import { ChevronDown, Palette } from 'lucide-react'

interface ColorPickerProps {
  value: string
  onChange: (color: string) => void
  className?: string
}

// Curated color palette organized by hue
const COLOR_PALETTE = {
  Blues: ['#3b82f6', '#2563eb', '#1d4ed8', '#1e40af', '#60a5fa', '#93c5fd'],
  Purples: ['#8b5cf6', '#7c3aed', '#6d28d9', '#5b21b6', '#a78bfa', '#c4b5fd'],
  Greens: ['#10b981', '#059669', '#047857', '#065f46', '#34d399', '#6ee7b7'],
  Emeralds: ['#14b8a6', '#0d9488', '#0f766e', '#115e59', '#2dd4bf', '#5eead4'],
  Yellows: ['#f59e0b', '#d97706', '#b45309', '#92400e', '#fbbf24', '#fcd34d'],
  Oranges: ['#f97316', '#ea580c', '#c2410c', '#9a3412', '#fb923c', '#fdba74'],
  Reds: ['#ef4444', '#dc2626', '#b91c1c', '#991b1b', '#f87171', '#fca5a5'],
  Pinks: ['#ec4899', '#db2777', '#be185d', '#9f1239', '#f472b6', '#f9a8d4'],
  Grays: ['#6b7280', '#4b5563', '#374151', '#1f2937', '#9ca3af', '#d1d5db'],
}

export default function ColorPicker({ value, onChange, className = '' }: ColorPickerProps) {
  const [isOpen, setIsOpen] = useState(false)
  const [customColor, setCustomColor] = useState(value)
  const [isMobile, setIsMobile] = useState(false)
  const buttonRef = useRef<HTMLButtonElement>(null)
  const dropdownRef = useRef<HTMLDivElement>(null)

  // Check for mobile on mount and resize
  useEffect(() => {
    const checkMobile = () => {
      setIsMobile(window.innerWidth < 768)
    }
    checkMobile()
    window.addEventListener('resize', checkMobile)
    return () => window.removeEventListener('resize', checkMobile)
  }, [])

  // Update custom color when value changes externally
  useEffect(() => {
    setCustomColor(value)
  }, [value])

  // Close dropdown on outside click
  useEffect(() => {
    if (!isOpen) return

    const handleClickOutside = (event: MouseEvent) => {
      if (
        dropdownRef.current &&
        !dropdownRef.current.contains(event.target as Node) &&
        buttonRef.current &&
        !buttonRef.current.contains(event.target as Node)
      ) {
        setIsOpen(false)
      }
    }

    document.addEventListener('mousedown', handleClickOutside)
    return () => document.removeEventListener('mousedown', handleClickOutside)
  }, [isOpen])

  const handleColorSelect = (color: string) => {
    onChange(color)
    setCustomColor(color)
  }

  const handleCustomColorChange = (color: string) => {
    setCustomColor(color)
    // Validate hex color
    if (/^#[0-9A-F]{6}$/i.test(color)) {
      onChange(color)
    }
  }

  // Calculate dropdown position
  const getDropdownStyle = (): React.CSSProperties => {
    if (!buttonRef.current) return {}

    const rect = buttonRef.current.getBoundingClientRect()
    const spaceBelow = window.innerHeight - rect.bottom
    const spaceAbove = rect.top
    const dropdownHeight = isMobile ? 400 : 460

    // Position above if not enough space below
    const shouldPositionAbove = spaceBelow < dropdownHeight && spaceAbove > spaceBelow

    if (isMobile) {
      // Center on mobile
      return {
        position: 'fixed',
        top: '50%',
        left: '50%',
        transform: 'translate(-50%, -50%)',
        width: '90vw',
        maxWidth: '360px',
      }
    }

    return {
      position: 'fixed',
      left: `${rect.left}px`,
      top: shouldPositionAbove ? `${rect.top - dropdownHeight - 8}px` : `${rect.bottom + 8}px`,
      width: `${Math.max(rect.width, 320)}px`,
      maxWidth: '360px',
    }
  }

  return (
    <>
      <button
        ref={buttonRef}
        type="button"
        onClick={() => setIsOpen(!isOpen)}
        className={`flex items-center gap-2 px-3 py-2 bg-white/5 border border-white/10 rounded-lg hover:bg-white/10 transition-colors ${className}`}
      >
        <div
          className="w-5 h-5 rounded border-2 border-white/20"
          style={{ backgroundColor: value }}
        />
        <span className="text-white text-sm font-mono">{value}</span>
        <ChevronDown className="w-4 h-4 text-white/60 ml-auto" />
      </button>

      {isOpen &&
        createPortal(
          <>
            {/* Mobile overlay */}
            {isMobile && (
              <div className="fixed inset-0 bg-black/50 backdrop-blur-sm z-[9998]" />
            )}

            {/* Dropdown */}
            <div
              ref={dropdownRef}
              style={getDropdownStyle()}
              className="bg-zinc-900 border border-white/10 rounded-lg shadow-2xl z-[9999] flex flex-col"
            >
              {/* Header */}
              <div className="p-3 border-b border-white/10 flex items-center gap-2">
                <Palette className="w-4 h-4 text-white/60" />
                <span className="text-white text-sm font-medium">Choose Color</span>
              </div>

              {/* Color palette */}
              <div className="flex-1 overflow-y-auto p-4 space-y-4">
                {Object.entries(COLOR_PALETTE).map(([category, colors]) => (
                  <div key={category}>
                    <div className="text-white/40 text-xs font-medium mb-2">{category}</div>
                    <div className="grid grid-cols-6 gap-2">
                      {colors.map((color) => {
                        const isSelected = color.toLowerCase() === value.toLowerCase()

                        return (
                          <button
                            key={color}
                            type="button"
                            onClick={() => handleColorSelect(color)}
                            className={`aspect-square rounded-lg transition-all hover:scale-110 relative ${
                              isSelected ? 'ring-2 ring-white ring-offset-2 ring-offset-zinc-900' : ''
                            }`}
                            style={{ backgroundColor: color }}
                            title={color}
                          >
                            {isSelected && (
                              <div className="absolute inset-0 flex items-center justify-center">
                                <div className="w-2 h-2 bg-white rounded-full shadow-lg" />
                              </div>
                            )}
                          </button>
                        )
                      })}
                    </div>
                  </div>
                ))}
              </div>

              {/* Custom color input */}
              <div className="p-3 border-t border-white/10">
                <div className="text-white/40 text-xs font-medium mb-2">Custom Color</div>
                <div className="flex gap-2">
                  <div className="relative flex-1">
                    <span className="absolute left-3 top-1/2 -translate-y-1/2 text-white/60 text-sm">
                      #
                    </span>
                    <input
                      type="text"
                      value={customColor.replace('#', '')}
                      onChange={(e) => {
                        const hex = e.target.value.replace(/[^0-9A-Fa-f]/g, '').slice(0, 6)
                        handleCustomColorChange(`#${hex}`)
                      }}
                      placeholder="3b82f6"
                      className="w-full pl-7 pr-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-white/40 focus:outline-none focus:ring-2 focus:ring-primary-500 font-mono text-sm"
                    />
                  </div>
                  <input
                    type="color"
                    value={value}
                    onChange={(e) => handleColorSelect(e.target.value)}
                    className="w-12 h-10 rounded-lg cursor-pointer bg-white/5 border border-white/10"
                    title="Pick custom color"
                  />
                </div>
              </div>
            </div>
          </>,
          document.body
        )}
    </>
  )
}
