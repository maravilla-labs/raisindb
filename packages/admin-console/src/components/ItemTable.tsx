import { useRef, useState, useEffect } from 'react'
import { Pencil, Trash2, MoveRight, GripVertical } from 'lucide-react'
import { Link } from 'react-router-dom'
import { draggable, dropTargetForElements } from '@atlaskit/pragmatic-drag-and-drop/element/adapter'
import { combine } from '@atlaskit/pragmatic-drag-and-drop/combine'

export interface TableColumn<T> {
  key: string
  header: string
  width?: string
  render: (item: T) => React.ReactNode
}

export interface ItemTableProps<T> {
  items: T[]
  columns: TableColumn<T>[]
  getItemId: (item: T) => string
  getItemPath: (item: T) => string
  getItemName: (item: T) => string
  itemType: string
  editPath?: (item: T) => string
  onEdit?: (item: T) => void
  onDelete?: (item: T) => void
  onMove?: (item: T) => void
  onReorder?: (sourcePath: string, targetPath: string, position: 'before' | 'after') => void
}

interface DraggableRowProps<T> {
  item: T
  columns: TableColumn<T>[]
  getItemId: (item: T) => string
  getItemPath: (item: T) => string
  getItemName: (item: T) => string
  itemType: string
  editPath?: string
  onEdit?: () => void
  onDelete?: () => void
  onMove?: () => void
  isDraggable: boolean
}

function DraggableRow<T>({
  item,
  columns,
  getItemId,
  getItemPath,
  getItemName,
  itemType,
  editPath,
  onEdit,
  onDelete,
  onMove,
  isDraggable,
}: DraggableRowProps<T>) {
  const rowRef = useRef<HTMLTableRowElement>(null)
  const [isDragging, setIsDragging] = useState(false)
  const [dropPosition, setDropPosition] = useState<'before' | 'after' | null>(null)

  useEffect(() => {
    const el = rowRef.current
    if (!el || !isDraggable) return

    return combine(
      draggable({
        element: el,
        getInitialData: () => ({
          id: getItemId(item),
          path: getItemPath(item),
          name: getItemName(item),
          type: itemType,
        }),
        onDragStart: () => setIsDragging(true),
        onDrop: () => setIsDragging(false),
      }),
      dropTargetForElements({
        element: el,
        canDrop: ({ source }) => {
          const sourcePath = source.data.path as string
          const targetPath = getItemPath(item)
          if (sourcePath === targetPath) return false
          return true
        },
        getData: () => ({
          id: getItemId(item),
          path: getItemPath(item),
          type: itemType,
        }),
        onDrag: ({ source, location }) => {
          if (source.data.path === getItemPath(item)) {
            setDropPosition(null)
            return
          }
          const element = rowRef.current
          if (!element) return
          const rect = element.getBoundingClientRect()
          const mouseY = location.current.input.clientY
          const midpoint = rect.top + rect.height / 2
          setDropPosition(mouseY < midpoint ? 'before' : 'after')
        },
        onDragLeave: () => setDropPosition(null),
        onDrop: () => setDropPosition(null),
      })
    )
  }, [item, getItemId, getItemPath, getItemName, itemType, isDraggable])

  return (
    <tr
      ref={rowRef}
      className={`
        border-b border-white/5 hover:bg-white/5 transition-colors
        ${isDragging ? 'opacity-50' : ''}
        ${dropPosition === 'before' ? 'border-t-2 border-t-primary-400' : ''}
        ${dropPosition === 'after' ? 'border-b-2 border-b-primary-400' : ''}
      `}
    >
      {isDraggable && (
        <td className="px-2 py-3 w-8 select-none">
          <GripVertical className="w-4 h-4 text-zinc-500 cursor-grab" />
        </td>
      )}
      {columns.map((col) => (
        <td key={col.key} className="px-4 py-3" style={{ width: col.width }}>
          {col.render(item)}
        </td>
      ))}
      <td className="px-4 py-3 text-right">
        <div className="flex items-center justify-end gap-1">
          {editPath ? (
            <Link
              to={editPath}
              className="p-1.5 hover:bg-white/10 text-zinc-400 hover:text-primary-300 rounded transition-colors"
              title="Edit"
            >
              <Pencil className="w-4 h-4" />
            </Link>
          ) : onEdit ? (
            <button
              onClick={onEdit}
              className="p-1.5 hover:bg-white/10 text-zinc-400 hover:text-primary-300 rounded transition-colors"
              title="Edit"
            >
              <Pencil className="w-4 h-4" />
            </button>
          ) : null}
          {onMove && (
            <button
              onClick={onMove}
              className="p-1.5 hover:bg-white/10 text-zinc-400 hover:text-zinc-300 rounded transition-colors"
              title="Move"
            >
              <MoveRight className="w-4 h-4" />
            </button>
          )}
          {onDelete && (
            <button
              onClick={onDelete}
              className="p-1.5 hover:bg-red-500/20 text-zinc-400 hover:text-red-400 rounded transition-colors"
              title="Delete"
            >
              <Trash2 className="w-4 h-4" />
            </button>
          )}
        </div>
      </td>
    </tr>
  )
}

export function ItemTable<T>({
  items,
  columns,
  getItemId,
  getItemPath,
  getItemName,
  itemType,
  editPath,
  onEdit,
  onDelete,
  onMove,
  onReorder,
}: ItemTableProps<T>) {
  const isDraggable = !!onReorder

  return (
    <div className="h-full flex flex-col overflow-hidden">
      <table className="w-full">
        <thead className="sticky top-0 bg-black/40 backdrop-blur-sm z-10 select-none">
          <tr className="border-b border-white/10">
            {isDraggable && <th className="w-8" />}
            {columns.map((col) => (
              <th
                key={col.key}
                className="px-4 py-3 text-left text-xs font-medium text-zinc-400 uppercase tracking-wider"
                style={{ width: col.width }}
              >
                {col.header}
              </th>
            ))}
            <th className="px-4 py-3 text-right text-xs font-medium text-zinc-400 uppercase tracking-wider w-28">
              Actions
            </th>
          </tr>
        </thead>
      </table>
      <div className="flex-1 overflow-y-auto overflow-x-hidden">
        <table className="w-full">
          <tbody>
            {items.map((item) => (
              <DraggableRow
                key={getItemId(item)}
                item={item}
                columns={columns}
                getItemId={getItemId}
                getItemPath={getItemPath}
                getItemName={getItemName}
                itemType={itemType}
                editPath={editPath ? editPath(item) : undefined}
                onEdit={onEdit ? () => onEdit(item) : undefined}
                onDelete={onDelete ? () => onDelete(item) : undefined}
                onMove={onMove ? () => onMove(item) : undefined}
                isDraggable={isDraggable}
              />
            ))}
          </tbody>
        </table>
      </div>
    </div>
  )
}
