import { Menu } from '@headlessui/react'
import {
  Edit,
  Plus,
  Copy,
  MoveHorizontal,
  Eye,
  EyeOff,
  Trash2,
  MoreVertical,
  AlertCircle
} from 'lucide-react'
import type { Node } from '../api/nodes'

interface ContextMenuProps {
  node: Node
  publishable?: boolean
  publishState?: 'unpublished' | 'published' | 'draft'
  onEdit: () => void
  onAddChild: () => void
  onCopy: () => void
  onMove: () => void
  onPublish?: () => void
  onUnpublish?: () => void
  onDelete: () => void
}

export default function ContextMenu({
  publishable = false,
  publishState = 'unpublished',
  onEdit,
  onAddChild,
  onCopy,
  onMove,
  onPublish,
  onUnpublish,
  onDelete,
}: ContextMenuProps) {
  return (
    <Menu as="div" className="relative">
      <Menu.Button
        className="p-1 hover:bg-white/10 rounded transition-colors"
        onClick={(e: React.MouseEvent) => e.stopPropagation()}
      >
        <MoreVertical className="w-4 h-4" />
      </Menu.Button>

      <Menu.Items className="absolute right-0 mt-2 w-56 origin-top-right glass-dark rounded-lg shadow-lg ring-1 ring-white/10 focus:outline-none z-50">
        <div className="p-1">
          <Menu.Item>
            {({ active }) => (
              <button
                onClick={(e) => {
                  e.stopPropagation()
                  onEdit()
                }}
                className={`${
                  active ? 'bg-white/10' : ''
                } group flex w-full items-center gap-3 rounded-lg px-3 py-2 text-sm text-white transition-colors`}
              >
                <Edit className="w-4 h-4 text-purple-400" />
                Edit Node
              </button>
            )}
          </Menu.Item>

          <Menu.Item>
            {({ active }) => (
              <button
                onClick={(e) => {
                  e.stopPropagation()
                  onAddChild()
                }}
                className={`${
                  active ? 'bg-white/10' : ''
                } group flex w-full items-center gap-3 rounded-lg px-3 py-2 text-sm text-white transition-colors`}
              >
                <Plus className="w-4 h-4 text-green-400" />
                Add Child Node
              </button>
            )}
          </Menu.Item>

          <div className="my-1 border-t border-white/10" />

          <Menu.Item>
            {({ active }) => (
              <button
                onClick={(e) => {
                  e.stopPropagation()
                  onCopy()
                }}
                className={`${
                  active ? 'bg-white/10' : ''
                } group flex w-full items-center gap-3 rounded-lg px-3 py-2 text-sm text-white transition-colors`}
              >
                <Copy className="w-4 h-4 text-blue-400" />
                Copy
              </button>
            )}
          </Menu.Item>

          <Menu.Item>
            {({ active }) => (
              <button
                onClick={(e) => {
                  e.stopPropagation()
                  onMove()
                }}
                className={`${
                  active ? 'bg-white/10' : ''
                } group flex w-full items-center gap-3 rounded-lg px-3 py-2 text-sm text-white transition-colors`}
              >
                <MoveHorizontal className="w-4 h-4 text-yellow-400" />
                Move
              </button>
            )}
          </Menu.Item>

          {publishable && (
            <>
              <div className="my-1 border-t border-white/10" />
              {publishState === 'unpublished' && (
                <Menu.Item>
                  {({ active }) => (
                    <button
                      onClick={(e) => {
                        e.stopPropagation()
                        if (onPublish) onPublish()
                      }}
                      className={`${
                        active ? 'bg-white/10' : ''
                      } group flex w-full items-center gap-3 rounded-lg px-3 py-2 text-sm text-white transition-colors`}
                    >
                      <Eye className="w-4 h-4 text-red-400" />
                      Publish
                    </button>
                  )}
                </Menu.Item>
              )}
              {publishState === 'draft' && (
                <Menu.Item>
                  {({ active }) => (
                    <button
                      onClick={(e) => {
                        e.stopPropagation()
                        if (onPublish) onPublish()
                      }}
                      className={`${
                        active ? 'bg-white/10' : ''
                      } group flex w-full items-center gap-3 rounded-lg px-3 py-2 text-sm text-white transition-colors relative`}
                    >
                      <Eye className="w-4 h-4 text-orange-400" />
                      Publish Changes
                      <AlertCircle className="w-3 h-3 text-orange-400 ml-auto" />
                    </button>
                  )}
                </Menu.Item>
              )}
              {publishState === 'published' && (
                <Menu.Item>
                  {({ active }) => (
                    <button
                      onClick={(e) => {
                        e.stopPropagation()
                        if (onUnpublish) onUnpublish()
                      }}
                      className={`${
                        active ? 'bg-white/10' : ''
                      } group flex w-full items-center gap-3 rounded-lg px-3 py-2 text-sm text-white transition-colors`}
                    >
                      <EyeOff className="w-4 h-4 text-green-400" />
                      Unpublish
                    </button>
                  )}
                </Menu.Item>
              )}
            </>
          )}

          <div className="my-1 border-t border-white/10" />

          <Menu.Item>
            {({ active }) => (
              <button
                onClick={(e) => {
                  e.stopPropagation()
                  onDelete()
                }}
                className={`${
                  active ? 'bg-white/10' : ''
                } group flex w-full items-center gap-3 rounded-lg px-3 py-2 text-sm text-red-400 transition-colors`}
              >
                <Trash2 className="w-4 h-4" />
                Delete
              </button>
            )}
          </Menu.Item>
        </div>
      </Menu.Items>
    </Menu>
  )
}
