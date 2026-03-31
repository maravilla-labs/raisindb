import { useState } from 'react'
import { createPortal } from 'react-dom'
import { X, Copy } from 'lucide-react'
import TreePicker from './TreePicker'
import type { Node } from '../api/nodes'
import { useToast, ToastContainer } from './Toast'

interface CopyNodeModalProps {
  node: Node
  allNodes: Node[]
  onCopy: (destination: string, newName?: string, recursive?: boolean) => Promise<void>
  onClose: () => void
}

export default function CopyNodeModal({ node, allNodes, onCopy, onClose }: CopyNodeModalProps) {
  const [destination, setDestination] = useState<string>('/')
  const [newName, setNewName] = useState(node.name)
  const [recursive, setRecursive] = useState(false)
  const [copying, setCopying] = useState(false)
  const { toasts, error: showError, closeToast } = useToast()

  async function handleCopy() {
    setCopying(true)
    try {
      await onCopy(destination, newName !== node.name ? newName : undefined, recursive)
      onClose()
    } catch (error) {
      console.error('Failed to copy:', error)
      showError('Error', 'Failed to copy node')
    } finally {
      setCopying(false)
    }
  }

  return createPortal(
    <div className="fixed inset-0 bg-black/50 backdrop-blur-sm flex items-center justify-center p-8 z-50 overscroll-none">
      <div className="glass-dark rounded-xl max-w-2xl w-full max-h-[90vh] overflow-auto overscroll-contain p-6">
        <div className="flex justify-between items-start mb-6">
          <div>
            <h2 className="text-2xl font-bold text-white flex items-center gap-2">
              <Copy className="w-6 h-6 text-blue-400" />
              Copy Node
            </h2>
            <p className="text-sm text-gray-400 mt-1">
              Source: <span className="text-purple-300">{node.path}</span>
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
          <div>
            <label className="block text-sm font-medium text-gray-300 mb-2">
              New Name (optional)
            </label>
            <input
              type="text"
              value={newName}
              onChange={(e) => setNewName(e.target.value)}
              className="w-full px-4 py-2 bg-white/10 border border-white/20 rounded-lg text-white focus:outline-none focus:ring-2 focus:ring-purple-500"
              placeholder={node.name}
            />
            <p className="text-xs text-gray-500 mt-1">
              Leave unchanged to copy with the same name
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
              />
            </div>
            <p className="text-xs text-gray-500 mt-1">
              Selected: <span className="text-purple-300">{destination}</span>
            </p>
          </div>

          <div>
            <label className="flex items-center gap-3 cursor-pointer">
              <input
                type="checkbox"
                checked={recursive}
                onChange={(e) => setRecursive(e.target.checked)}
                className="w-4 h-4 rounded border-white/20 bg-white/10 text-blue-500 focus:ring-2 focus:ring-blue-500 focus:ring-offset-0"
              />
              <div>
                <span className="text-sm font-medium text-gray-300">
                  Copy recursively (include all children)
                </span>
                <p className="text-xs text-gray-500 mt-0.5">
                  When enabled, copies the entire node tree including all descendants
                </p>
              </div>
            </label>
          </div>

          <div className="flex gap-3 justify-end">
            <button
              onClick={onClose}
              className="px-4 py-2 bg-white/10 hover:bg-white/20 text-white rounded-lg transition-colors"
            >
              Cancel
            </button>
            <button
              onClick={handleCopy}
              disabled={copying}
              className="flex items-center gap-2 px-4 py-2 bg-blue-500 hover:bg-blue-600 text-white rounded-lg transition-colors disabled:opacity-50"
            >
              <Copy className="w-4 h-4" />
              {copying ? 'Copying...' : 'Copy'}
            </button>
          </div>
        </div>
        <ToastContainer toasts={toasts} onClose={closeToast} />
      </div>
    </div>,
    document.body
  )
}
