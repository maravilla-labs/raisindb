/**
 * TypePickerTree - Tree view component for type picker
 *
 * Renders the hierarchical tree of namespaces and type items.
 */

import { memo, useCallback, type ComponentType } from 'react'
import { ChevronRight, ChevronDown, Folder, Check, FileType } from 'lucide-react'
import type { TypeTreeNode, SelectionMode } from './types'
import { countItems } from './useTypePickerTree'

interface TypePickerNamespaceGroupProps {
  node: TypeTreeNode
  isExpanded: boolean
  onToggle: () => void
  children: React.ReactNode
}

const TypePickerNamespaceGroup = memo(function TypePickerNamespaceGroup({
  node,
  isExpanded,
  onToggle,
  children,
}: TypePickerNamespaceGroupProps) {
  const itemCount = countItems(node)

  return (
    <div role="group" aria-label={`${node.name} namespace`}>
      <button
        type="button"
        onClick={onToggle}
        aria-expanded={isExpanded}
        className="w-full flex items-center gap-2 px-3 py-1.5 hover:bg-white/5 transition-colors text-left"
        style={{ paddingLeft: `${12 + node.depth * 16}px` }}
      >
        {isExpanded ? (
          <ChevronDown className="w-4 h-4 text-zinc-500 flex-shrink-0" />
        ) : (
          <ChevronRight className="w-4 h-4 text-zinc-500 flex-shrink-0" />
        )}
        <Folder className="w-4 h-4 text-amber-400 flex-shrink-0" />
        <span className="text-sm text-zinc-300 flex-1 truncate">{node.name}</span>
        <span className="text-xs text-zinc-500">{itemCount}</span>
      </button>

      {isExpanded && <div>{children}</div>}
    </div>
  )
})

interface TypePickerItemProps {
  node: TypeTreeNode
  isSelected: boolean
  onSelect: () => void
  mode: SelectionMode
  icon?: ComponentType<{ className?: string }>
  iconColor?: string
  isFocused?: boolean
}

const TypePickerItem = memo(function TypePickerItem({
  node,
  isSelected,
  onSelect,
  mode,
  icon: Icon = FileType,
  iconColor = 'text-primary-400',
  isFocused,
}: TypePickerItemProps) {
  return (
    <button
      type="button"
      onClick={onSelect}
      role="option"
      aria-selected={isSelected}
      className={`
        w-full flex items-center gap-2 px-3 py-1.5 transition-colors text-sm text-left
        ${isSelected ? 'bg-primary-500/20 text-primary-300' : 'text-zinc-300 hover:bg-white/5'}
        ${isFocused ? 'ring-2 ring-inset ring-primary-500/50' : ''}
      `}
      style={{ paddingLeft: `${12 + node.depth * 16}px` }}
    >
      {mode === 'multi' && (
        <div
          className={`
            w-4 h-4 rounded border flex items-center justify-center flex-shrink-0
            ${isSelected ? 'bg-primary-500 border-primary-500' : 'border-white/30 bg-transparent'}
          `}
        >
          {isSelected && <Check className="w-3 h-3 text-white" />}
        </div>
      )}

      <Icon className={`w-4 h-4 ${iconColor} flex-shrink-0`} />

      <span className="flex-1 truncate">{node.name}</span>

      {isSelected && mode === 'single' && (
        <Check className="w-4 h-4 text-primary-400 flex-shrink-0" />
      )}
    </button>
  )
})

interface TypePickerTreeProps {
  tree: TypeTreeNode[]
  expandedPaths: Set<string>
  onToggleExpand: (path: string) => void
  selectedValues: Set<string>
  onSelect: (name: string) => void
  mode: SelectionMode
  itemIcon?: ComponentType<{ className?: string }>
  itemIconColor?: string
  focusedPath?: string | null
}

export default function TypePickerTree({
  tree,
  expandedPaths,
  onToggleExpand,
  selectedValues,
  onSelect,
  mode,
  itemIcon,
  itemIconColor,
  focusedPath,
}: TypePickerTreeProps) {
  const renderNode = useCallback(
    (node: TypeTreeNode): React.ReactNode => {
      if (node.type === 'namespace') {
        const isExpanded = expandedPaths.has(node.fullPath)
        return (
          <TypePickerNamespaceGroup
            key={node.id}
            node={node}
            isExpanded={isExpanded}
            onToggle={() => onToggleExpand(node.fullPath)}
          >
            {node.children?.map((child) => renderNode(child))}
          </TypePickerNamespaceGroup>
        )
      }

      return (
        <TypePickerItem
          key={node.id}
          node={node}
          isSelected={selectedValues.has(node.fullPath)}
          onSelect={() => onSelect(node.fullPath)}
          mode={mode}
          icon={itemIcon}
          iconColor={itemIconColor}
          isFocused={focusedPath === node.fullPath}
        />
      )
    },
    [expandedPaths, onToggleExpand, selectedValues, onSelect, mode, itemIcon, itemIconColor, focusedPath]
  )

  if (tree.length === 0) {
    return (
      <div className="px-3 py-4 text-sm text-zinc-500 text-center">
        No types found
      </div>
    )
  }

  return <div className="py-1">{tree.map((node) => renderNode(node))}</div>
}
