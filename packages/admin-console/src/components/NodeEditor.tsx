import { useState, useEffect } from 'react'
import { X, Save } from 'lucide-react'
import YamlEditor from './YamlEditor'
import * as yaml from 'js-yaml'
import type { Node } from '../api/nodes'

interface NodeEditorProps {
  node: Node | null
  onSave: (node: Partial<Node>) => Promise<void>
  onClose: () => void
}

export default function NodeEditor({ node, onSave, onClose }: NodeEditorProps) {
  const [yamlContent, setYamlContent] = useState('')
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    if (node) {
      setYamlContent(yaml.dump(node, { indent: 2 }))
    }
  }, [node])

  if (!node) return null

  async function handleSave() {
    setSaving(true)
    setError(null)

    try {
      const parsed = yaml.load(yamlContent) as Partial<Node>
      await onSave(parsed)
      onClose()
    } catch (err: any) {
      setError(err.message || 'Failed to save node')
    } finally {
      setSaving(false)
    }
  }

  return (
    <div className="fixed inset-0 bg-black/50 backdrop-blur-sm flex items-center justify-center p-8 z-50">
      <div className="glass-dark rounded-xl max-w-4xl w-full max-h-[90vh] overflow-auto p-6">
        <div className="flex justify-between items-start mb-6">
          <div>
            <h2 className="text-2xl font-bold text-white">{node.name}</h2>
            <p className="text-sm text-gray-400">{node.path}</p>
          </div>
          <button
            onClick={onClose}
            className="p-2 hover:bg-white/10 rounded-lg transition-colors"
          >
            <X className="w-6 h-6 text-gray-400" />
          </button>
        </div>

        {error && (
          <div className="mb-4 p-4 bg-red-500/20 border border-red-500/50 rounded-lg text-red-300">
            {error}
          </div>
        )}

        <div className="mb-4">
          <label className="block text-sm font-medium text-gray-300 mb-2">
            Node Properties (YAML)
          </label>
          <YamlEditor
            value={yamlContent}
            onChange={(value) => setYamlContent(value || '')}
            height="400px"
          />
        </div>

        <div className="flex gap-3 justify-end">
          <button
            onClick={onClose}
            className="px-4 py-2 bg-white/10 hover:bg-white/20 text-white rounded-lg transition-colors"
          >
            Cancel
          </button>
          <button
            onClick={handleSave}
            disabled={saving}
            className="flex items-center gap-2 px-4 py-2 bg-purple-500 hover:bg-purple-600 text-white rounded-lg transition-colors disabled:opacity-50"
          >
            <Save className="w-4 h-4" />
            {saving ? 'Saving...' : 'Save'}
          </button>
        </div>
      </div>
    </div>
  )
}
