import { useEffect, useMemo, useState } from 'react'
import { createPortal } from 'react-dom'
import { X, MoveHorizontal } from 'lucide-react'
import TreePicker from './TreePicker'
import type { Node } from '../api/nodes'
import { useToast, ToastContainer } from './Toast'

interface MoveNodeModalProps {
  node: Node
  allNodes: Node[]
  onMove: (destination: string) => Promise<void>
  onClose: () => void
}

export default function MoveNodeModal({ node, allNodes, onMove, onClose }: MoveNodeModalProps) {
  const initialParent = useMemo(() => {
    const lastSlash = node.path.lastIndexOf('/')
    if (lastSlash <= 0) {
      return '/'
    }
    return node.path.substring(0, lastSlash) || '/'
  }, [node.path])

  const [destination, setDestination] = useState<string>(initialParent)
  const [moving, setMoving] = useState(false)
  const { toasts, error: showError, closeToast } = useToast()

  useEffect(() => {
    setDestination(initialParent)
  }, [initialParent])

  async function handleMove() {
    const normalizedDestination = destination === '/'
      ? '/'
      : destination.replace(/\/+$/, '') || '/'

    const newPath = normalizedDestination === '/'
      ? `/${node.name}`
      : `${normalizedDestination}/${node.name}`

    if (newPath === node.path) {
      showError('Invalid Operation', 'Node is already in this location')
      return
    }

    if (normalizedDestination.startsWith(`${node.path}/`)) {
      showError('Invalid Operation', 'Cannot move a node into its own descendants')
      return
    }

    setMoving(true)
    try {
      await onMove(newPath)
      onClose()
    } catch (error) {
      console.error('Failed to move:', error)
      showError('Error', 'Failed to move node')
    } finally {
      setMoving(false)
    }
  }

  return createPortal(
    <div className="fixed inset-0 bg-black/50 backdrop-blur-sm flex items-center justify-center p-8 z-50 overscroll-none">
      <div className="glass-dark rounded-xl max-w-2xl w-full max-h-[90vh] overflow-auto overscroll-contain p-6">
        <div className="flex justify-between items-start mb-6">
          <div>
            <h2 className="text-2xl font-bold text-white flex items-center gap-2">
              <MoveHorizontal className="w-6 h-6 text-yellow-400" />
              Move Node
            </h2>
            <p className="text-sm text-gray-400 mt-1">
              Moving: <span className="text-purple-300">{node.path}</span>
            </p>
          </div>
          <button
            onClick={onClose}
            className="p-2 hover:bg-white/10 rounded-lg transition-colors"
          >
            <X className="w-6 h-6 text-gray-400" />
          </button>
        </div>

        <div className="space-y-6">
          <div className="p-4 bg-yellow-500/10 border border-yellow-400/30 rounded-lg">
            <p className="text-sm text-yellow-300">
              <strong>Warning:</strong> Moving a node will change its path. Any references to this node may need to be updated.
            </p>
          </div>

          <div>
            <label className="block text-sm font-medium text-gray-300 mb-2">
              Destination *
            </label>
            <div className="p-4 bg-white/5 border border-white/10 rounded-lg">
              <TreePicker
                nodes={allNodes}
                selectedPath={destination}
                onSelect={setDestination}
                excludePath={node.path}  // Can't move into itself
              />
            </div>
            <p className="text-xs text-gray-500 mt-1">
              Selected: <span className="text-purple-300">{destination}</span>
            </p>
          </div>

          <div className="flex gap-3 justify-end">
            <button
              onClick={onClose}
              className="px-4 py-2 bg-white/10 hover:bg-white/20 text-white rounded-lg transition-colors"
            >
              Cancel
            </button>
            <button
              onClick={handleMove}
              disabled={moving}
              className="flex items-center gap-2 px-4 py-2 bg-yellow-500 hover:bg-yellow-600 text-white rounded-lg transition-colors disabled:opacity-50"
            >
              <MoveHorizontal className="w-4 h-4" />
              {moving ? 'Moving...' : 'Move'}
            </button>
          </div>
        </div>
        <ToastContainer toasts={toasts} onClose={closeToast} />
      </div>
    </div>,
    document.body
  )
}
