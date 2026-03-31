/**
 * Type Picker - Shared type definitions
 *
 * Generic types for the namespace-grouped type picker component.
 */

import type { ComponentType } from 'react'

/**
 * Base interface for any pickable type item.
 * Both NodeTypes and Archetypes conform to this.
 */
export interface PickableType {
  name: string // Full qualified name (e.g., "news:Article")
  description?: string
  icon?: string
}

/**
 * Selection mode for the picker
 */
export type SelectionMode = 'single' | 'multi'

/**
 * Tree node representing either a namespace folder or a type item
 */
export interface TypeTreeNode {
  id: string // Unique ID for React keys
  type: 'namespace' | 'item'
  name: string // Display name (segment for namespace, type name for item)
  fullPath: string // Full path (namespace path or full type name)
  depth: number // Nesting level (0 = root)
  children?: TypeTreeNode[] // Child nodes (for namespace type)
  item?: PickableType // Original item (for item type)
}

/**
 * Props for the main TypePicker component
 */
export interface TypePickerProps<T extends PickableType = PickableType> {
  // Data
  items: T[]
  loading?: boolean

  // Selection
  mode: SelectionMode
  value: string | string[] // Single value or array for multi
  onChange: (value: string | string[]) => void

  // Special options
  allowWildcard?: boolean // Show "*" option for "Allow All"
  allowNone?: boolean // Show "None" option
  noneLabel?: string // Label for none option (default: "None")
  wildcardLabel?: string // Label for wildcard (default: "Allow All (*)")

  // UI Customization
  placeholder?: string
  disabled?: boolean
  className?: string
  maxHeight?: number // Dropdown max height (default: 300)

  // Item rendering
  itemIcon?: ComponentType<{ className?: string }>
  itemIconColor?: string // Tailwind color class

  // Validation
  error?: string
}

/**
 * Props for the TypePickerTree component
 */
export interface TypePickerTreeProps {
  tree: TypeTreeNode[]
  expandedPaths: Set<string>
  onToggleExpand: (path: string) => void
  selectedValues: Set<string>
  onSelect: (name: string) => void
  mode: SelectionMode
  itemIcon?: ComponentType<{ className?: string }>
  itemIconColor?: string
  focusedPath?: string
}

/**
 * Props for TypePickerNamespaceGroup
 */
export interface TypePickerNamespaceGroupProps {
  node: TypeTreeNode
  isExpanded: boolean
  onToggle: () => void
  children: React.ReactNode
  itemCount: number
}

/**
 * Props for TypePickerItem
 */
export interface TypePickerItemProps {
  node: TypeTreeNode
  isSelected: boolean
  onSelect: () => void
  mode: SelectionMode
  icon?: ComponentType<{ className?: string }>
  iconColor?: string
  isFocused?: boolean
}

/**
 * State returned by useTypePickerState hook
 */
export interface TypePickerState {
  // Search
  searchQuery: string
  setSearchQuery: (query: string) => void

  // Dropdown
  isOpen: boolean
  setIsOpen: (open: boolean) => void

  // Tree expansion
  expandedPaths: Set<string>
  toggleExpanded: (path: string) => void
  setExpandedPaths: (paths: Set<string>) => void

  // Keyboard navigation
  focusedPath: string | null
  setFocusedPath: (path: string | null) => void
}

/**
 * Dropdown positioning style
 */
export interface DropdownPosition {
  position: 'fixed'
  left: number
  width: number
  top?: number
  bottom?: number
  zIndex: number
}
