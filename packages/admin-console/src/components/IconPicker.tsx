import { useState, useRef, useEffect } from 'react'
import { createPortal } from 'react-dom'
import * as LucideIcons from 'lucide-react'
import { Search, ChevronDown } from 'lucide-react'

interface IconPickerProps {
  value: string
  onChange: (icon: string) => void
  className?: string
}

// Curated list of ~50 commonly used icons
const CURATED_ICONS = [
  'folder',
  'folder-open',
  'folder-plus',
  'folder-archive',
  'user',
  'users',
  'user-plus',
  'user-check',
  'shield',
  'shield-check',
  'lock',
  'unlock',
  'key',
  'star',
  'bookmark',
  'tag',
  'tags',
  'file',
  'file-text',
  'files',
  'database',
  'server',
  'settings',
  'sliders',
  'grid',
  'layout',
  'layers',
  'package',
  'inbox',
  'mail',
  'bell',
  'heart',
  'home',
  'building',
  'briefcase',
  'calendar',
  'clock',
  'flag',
  'bookmark',
  'archive',
  'trash',
  'edit',
  'eye',
  'download',
  'upload',
  'link',
  'external-link',
  'check',
  'x',
  'plus',
  'minus',
]

// Get all available Lucide icons
const getAllIcons = () => {
  return Object.keys(LucideIcons)
    .filter((key) => {
      const component = (LucideIcons as any)[key]
      // Lucide icons are React component objects with a render function
      return (
        component &&
        typeof component === 'object' &&
        typeof component.render === 'function' &&
        !key.endsWith('Icon') && // Exclude duplicate "Icon" suffix exports
        key !== 'createLucideIcon'
      )
    })
    .map((name) => {
      // Convert PascalCase to kebab-case
      return name
        .replace(/([A-Z])/g, '-$1')
        .toLowerCase()
        .replace(/^-/, '')
    })
    .sort()
}

// Cache all icons at module level for performance
const ALL_ICONS = getAllIcons()

// Convert kebab-case to PascalCase for Lucide component lookup
const iconNameToPascalCase = (name: string): string => {
  return name
    .split('-')
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join('')
}

// Get Lucide icon component by name
const getIconComponent = (name: string) => {
  const pascalName = iconNameToPascalCase(name)
  return (LucideIcons as any)[pascalName] || LucideIcons.Folder
}

export default function IconPicker({ value, onChange, className = '' }: IconPickerProps) {
  const [isOpen, setIsOpen] = useState(false)
  const [searchTerm, setSearchTerm] = useState('')
  const [showAll, setShowAll] = useState(true)
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
        setSearchTerm('')
        setShowAll(false)
      }
    }

    document.addEventListener('mousedown', handleClickOutside)
    return () => document.removeEventListener('mousedown', handleClickOutside)
  }, [isOpen])

  // Update dropdown position dynamically
  useEffect(() => {
    if (!isOpen || !dropdownRef.current || !buttonRef.current) return

    const updatePosition = () => {
      if (!dropdownRef.current || !buttonRef.current) return
      const style = getDropdownStyle()
      Object.assign(dropdownRef.current.style, style)
    }

    // Initial position
    updatePosition()

    // Update on scroll/resize
    window.addEventListener('scroll', updatePosition, true)
    window.addEventListener('resize', updatePosition)

    return () => {
      window.removeEventListener('scroll', updatePosition, true)
      window.removeEventListener('resize', updatePosition)
    }
  }, [isOpen, isMobile])

  // Get available icons based on showAll state
  const availableIcons = showAll ? ALL_ICONS : CURATED_ICONS

  // Filter icons based on search term
  const filteredIcons = availableIcons.filter((icon) =>
    icon.toLowerCase().includes(searchTerm.toLowerCase())
  )

  const handleIconSelect = (icon: string) => {
    onChange(icon)
    setIsOpen(false)
    setSearchTerm('')
    setShowAll(false)
  }

  const CurrentIcon = getIconComponent(value)

  // Calculate dropdown position
  const getDropdownStyle = (): React.CSSProperties => {
    if (!buttonRef.current) return {}

    const rect = buttonRef.current.getBoundingClientRect()
    const spaceBelow = window.innerHeight - rect.bottom
    const spaceAbove = rect.top
    const dropdownHeight = isMobile ? 400 : 480

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
        maxWidth: '400px',
      }
    }

    return {
      position: 'fixed',
      left: `${rect.left}px`,
      top: shouldPositionAbove ? `${rect.top - dropdownHeight - 8}px` : `${rect.bottom + 8}px`,
      width: `${Math.max(rect.width, 320)}px`,
      maxWidth: '400px',
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
        <CurrentIcon className="w-5 h-5 text-white" />
        <span className="text-white text-sm">{value}</span>
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
              className="bg-zinc-900 border border-white/10 rounded-lg shadow-2xl z-[9999] flex flex-col max-h-[480px]"
            >
              {/* Search */}
              <div className="p-3 border-b border-white/10">
                <div className="relative">
                  <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-white/40" />
                  <input
                    type="text"
                    value={searchTerm}
                    onChange={(e) => setSearchTerm(e.target.value)}
                    placeholder="Search icons..."
                    className="w-full pl-10 pr-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-white/40 focus:outline-none focus:ring-2 focus:ring-primary-500"
                    autoFocus={!isMobile}
                  />
                </div>
              </div>

              {/* Icon grid */}
              <div className="flex-1 overflow-y-auto p-3">
                <div className="grid grid-cols-6 md:grid-cols-8 gap-2">
                  {filteredIcons.map((iconName) => {
                    const IconComponent = getIconComponent(iconName)
                    const isSelected = iconName === value

                    return (
                      <button
                        key={iconName}
                        type="button"
                        onClick={() => handleIconSelect(iconName)}
                        className={`aspect-square flex items-center justify-center rounded-lg transition-all hover:bg-primary-500 hover:scale-110 ${
                          isSelected
                            ? 'bg-primary-500 text-white'
                            : 'bg-white/5 text-white/70 hover:text-white'
                        }`}
                        title={iconName}
                      >
                        <IconComponent className="w-5 h-5" />
                      </button>
                    )
                  })}
                </div>

                {filteredIcons.length === 0 && (
                  <div className="text-center py-8 text-white/40">No icons found</div>
                )}
              </div>

              {/* Show curated button */}
              {showAll && searchTerm === '' && (
                <div className="p-3 border-t border-white/10">
                  <button
                    type="button"
                    onClick={() => {
                      setShowAll(false)
                      setSearchTerm('')
                    }}
                    className="w-full py-2 px-4 bg-white/5 hover:bg-white/10 text-white rounded-lg transition-colors text-sm font-medium"
                  >
                    Show curated icons ({CURATED_ICONS.length})
                  </button>
                </div>
              )}

              {/* Show all button */}
              {!showAll && (
                <div className="p-3 border-t border-white/10">
                  <button
                    type="button"
                    onClick={() => setShowAll(true)}
                    className="w-full py-2 px-4 bg-primary-500 hover:bg-primary-600 text-white rounded-lg transition-colors text-sm font-medium"
                  >
                    Show all icons ({ALL_ICONS.length})
                  </button>
                </div>
              )}
            </div>
          </>,
          document.body
        )}
    </>
  )
}
