/**
 * Field Canvas Component
 *
 * Displays the list of fields in the archetype.
 * Uses Pragmatic Drag and Drop for drop target functionality.
 */

import { useRef, useState, useEffect } from 'react'
import { Layers, Plus } from 'lucide-react'
import { dropTargetForElements } from '@atlaskit/pragmatic-drag-and-drop/element/adapter'
import FieldItem from './FieldItem'
import type { ArchetypeDefinition } from './types'

interface FieldCanvasProps {
  archetype: ArchetypeDefinition
  selectedPath?: string
  onPathSelect: (path: string | undefined) => void
  onPathDelete: (path: string) => void
}

export default function FieldCanvas({
  archetype,
  selectedPath,
  onPathSelect,
  onPathDelete,
}: FieldCanvasProps) {
  const dropZoneRef = useRef<HTMLDivElement>(null)
  const [isDraggedOver, setIsDraggedOver] = useState(false)

  // Setup drop target for the canvas
  useEffect(() => {
    const el = dropZoneRef.current
    if (!el) return

    return dropTargetForElements({
      element: el,
      canDrop: () => true,
      onDragEnter: () => setIsDraggedOver(true),
      onDragLeave: () => setIsDraggedOver(false),
      onDrop: () => setIsDraggedOver(false),
      getData: () => ({
        path: '', // Top-level canvas (empty path means root)
        isCanvas: true,
        // Provide instruction for the drop monitor
        instruction: { type: 'make-child' },
      }),
    })
  }, [])

  return (
    <div className="h-full flex flex-col overflow-hidden bg-zinc-900/30">
      {/* Fields List Section */}
      <div className="flex-1 flex flex-col overflow-hidden">
        <div className="flex-shrink-0 px-3 py-2 border-b border-white/10">
          <div className="flex items-center gap-2">
            <Layers className="w-4 h-4 text-primary-400" />
            <h3 className="text-sm font-semibold text-white">Fields</h3>
            <span className="text-[10px] text-zinc-500 bg-white/5 px-1.5 py-0.5 rounded">
              {archetype.fields.length}
            </span>
          </div>
          <p className="text-[11px] text-zinc-400 mt-0.5">
            Drag from toolbox or reorder • Click to edit
          </p>
        </div>

        <div
          ref={dropZoneRef}
          className={`
            flex-1 overflow-y-auto p-3 min-h-[200px]
            transition-all duration-200
            ${isDraggedOver ? 'bg-primary-500/10' : ''}
          `}
          style={{ touchAction: 'none' }}
        >
          {archetype.fields.length === 0 ? (
            <div className={`
              flex flex-col items-center justify-center h-full min-h-[250px] text-center py-8
              border-2 border-dashed rounded-lg transition-all duration-200
              ${isDraggedOver
                ? 'border-primary-500 bg-primary-500/10 scale-105'
                : 'border-white/10 bg-white/5'
              }
            `}>
              <Plus className={`w-10 h-10 mb-2 transition-colors ${
                isDraggedOver ? 'text-primary-400' : 'text-zinc-600'
              }`} />
              <p className="text-zinc-400 text-sm font-medium">
                {isDraggedOver ? 'Drop here to add field' : 'No fields yet'}
              </p>
              <p className="text-zinc-600 text-xs mt-1">
                Drag field types from the left toolbox
              </p>
            </div>
          ) : (
            <>
              {archetype.fields.map((field, index) => (
                <FieldItem
                  key={field.id}
                  field={field}
                  index={index}
                  path={field.id!}
                  selectedPath={selectedPath}
                  onPathSelect={onPathSelect}
                  onPathDelete={onPathDelete}
                  isDraggable={true}
                />
              ))}
            </>
          )}
        </div>
      </div>
    </div>
  )
}
