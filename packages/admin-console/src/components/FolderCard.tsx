import { useState, useRef, useEffect } from 'react'
import * as LucideIcons from 'lucide-react'
import { Folder as FolderIcon, MoreVertical, Edit, Trash2, MoveRight } from 'lucide-react'
import { dropTargetForElements } from '@atlaskit/pragmatic-drag-and-drop/element/adapter'
import type { Node } from '../api/nodes'

interface FolderCardProps {
  folder: Node
  onClick: () => void
  onEdit?: (folder: Node) => void
  onDelete?: (folder: Node) => void
  onMove?: (folder: Node) => void
  /** Called when an item is dropped onto this folder */
  onDropInto?: (sourcePath: string, sourceType: string) => void
  className?: string
}

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
  return (LucideIcons as any)[pascalName] || FolderIcon
}

export default function FolderCard({ folder, onClick, onEdit, onDelete, onMove, onDropInto, className = '' }: FolderCardProps) {
  const [showMenu, setShowMenu] = useState(false)
  const [isDraggedOver, setIsDraggedOver] = useState(false)
  const menuRef = useRef<HTMLDivElement>(null)
  const cardRef = useRef<HTMLDivElement>(null)
  const icon = (folder.properties?.icon as string) || 'folder'
  const color = (folder.properties?.color as string) || '#3b82f6'
  const description = folder.properties?.description as string | undefined

  const IconComponent = getIconComponent(icon)

  // Set up drop target for moving items into this folder
  useEffect(() => {
    const el = cardRef.current
    if (!el || !onDropInto) return

    return dropTargetForElements({
      element: el,
      canDrop: ({ source }) => {
        // Don't allow dropping folder onto itself
        const sourcePath = source.data.path as string
        if (sourcePath === folder.path) return false
        // Don't allow dropping parent folder into child
        if (folder.path.startsWith(sourcePath + '/')) return false
        return true
      },
      onDragEnter: () => setIsDraggedOver(true),
      onDragLeave: () => setIsDraggedOver(false),
      onDrop: ({ source }) => {
        setIsDraggedOver(false)
        const sourcePath = source.data.path as string
        const sourceType = source.data.type as string
        onDropInto(sourcePath, sourceType)
      },
    })
  }, [folder.path, onDropInto])

  // Close menu on outside click
  useEffect(() => {
    if (!showMenu) return

    const handleClickOutside = (event: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(event.target as HTMLElement)) {
        setShowMenu(false)
      }
    }

    document.addEventListener('mousedown', handleClickOutside)
    return () => document.removeEventListener('mousedown', handleClickOutside)
  }, [showMenu])

  // Lighten the color for the gradient
  const lightenColor = (hex: string, percent: number = 20): string => {
    const num = parseInt(hex.replace('#', ''), 16)
    const r = Math.min(255, ((num >> 16) & 255) + percent)
    const g = Math.min(255, ((num >> 8) & 255) + percent)
    const b = Math.min(255, (num & 255) + percent)
    return `#${((r << 16) | (g << 8) | b).toString(16).padStart(6, '0')}`
  }

  const lightColor = lightenColor(color, 30)

  return (
    <div
      ref={cardRef}
      className={`group relative overflow-hidden rounded-xl transition-all hover:scale-105 hover:shadow-xl ${isDraggedOver ? 'ring-4 ring-white/50 scale-110' : ''} ${className}`}
      style={{
        background: `linear-gradient(135deg, ${color} 0%, ${lightColor} 100%)`,
      }}
    >
      {/* Context menu button */}
      {(onEdit || onDelete || onMove) && (
        <div className="absolute top-2 right-2 z-10" ref={menuRef}>
          <button
            type="button"
            onClick={(e) => {
              e.stopPropagation()
              setShowMenu(!showMenu)
            }}
            className="p-2 bg-black/20 hover:bg-black/40 rounded-lg transition-colors opacity-0 group-hover:opacity-100"
          >
            <MoreVertical className="w-4 h-4 text-white" />
          </button>

          {/* Dropdown menu */}
          {showMenu && (
            <div className="absolute right-0 mt-2 w-48 bg-zinc-800 border border-white/10 rounded-lg shadow-xl overflow-hidden">
              {onEdit && (
                <button
                  type="button"
                  onClick={(e) => {
                    e.stopPropagation()
                    setShowMenu(false)
                    onEdit(folder)
                  }}
                  className="w-full px-4 py-2 text-left text-white hover:bg-white/10 flex items-center gap-2 transition-colors"
                >
                  <Edit className="w-4 h-4" />
                  <span>Edit Folder</span>
                </button>
              )}
              {onMove && (
                <button
                  type="button"
                  onClick={(e) => {
                    e.stopPropagation()
                    setShowMenu(false)
                    onMove(folder)
                  }}
                  className="w-full px-4 py-2 text-left text-white hover:bg-white/10 flex items-center gap-2 transition-colors"
                >
                  <MoveRight className="w-4 h-4" />
                  <span>Move Folder</span>
                </button>
              )}
              {onDelete && (
                <button
                  type="button"
                  onClick={(e) => {
                    e.stopPropagation()
                    setShowMenu(false)
                    onDelete(folder)
                  }}
                  className="w-full px-4 py-2 text-left text-red-400 hover:bg-red-500/10 flex items-center gap-2 transition-colors"
                >
                  <Trash2 className="w-4 h-4" />
                  <span>Delete Folder</span>
                </button>
              )}
            </div>
          )}
        </div>
      )}

      {/* Main clickable area */}
      <button
        type="button"
        onClick={onClick}
        className="w-full text-left focus:outline-none focus:ring-2 focus:ring-primary-500 focus:ring-offset-2 focus:ring-offset-zinc-900 rounded-xl"
      >
        {/* Content */}
        <div className="relative p-6 md:p-8">
          {/* Icon */}
          <div className="mb-4">
            <div className="w-12 h-12 md:w-16 md:h-16 rounded-lg bg-white/20 backdrop-blur-sm flex items-center justify-center group-hover:bg-white/30 transition-colors">
              <IconComponent className="w-6 h-6 md:w-8 md:h-8 text-white" />
            </div>
          </div>

          {/* Name */}
          <h3 className="text-lg md:text-xl font-semibold text-white mb-1">
            {folder.name}
          </h3>

          {/* Description */}
          {description && (
            <p className="text-sm text-white/80 line-clamp-2 whitespace-pre-line">{description}</p>
          )}
        </div>

        {/* Hover overlay */}
        <div className="absolute inset-0 bg-white/0 group-hover:bg-white/10 transition-colors pointer-events-none" />

        {/* Decorative elements */}
        <div className="absolute top-0 right-0 w-32 h-32 bg-white/5 rounded-full -translate-y-16 translate-x-16 group-hover:scale-150 transition-transform duration-500" />
        <div className="absolute bottom-0 left-0 w-24 h-24 bg-black/10 rounded-full translate-y-12 -translate-x-12 group-hover:scale-150 transition-transform duration-500" />
      </button>
    </div>
  )
}
