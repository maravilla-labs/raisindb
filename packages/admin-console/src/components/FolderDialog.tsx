import { useState, useEffect } from 'react'
import { X } from 'lucide-react'
import IconPicker from './IconPicker'
import ColorPicker from './ColorPicker'
import type { Node } from '../api/nodes'

interface FolderDialogProps {
  folder?: Node // If provided, we're editing; otherwise creating
  onClose: () => void
  onSave: (data: {
    name: string
    description: string
    icon: string
    color: string
  }) => Promise<void>
}

export default function FolderDialog({ folder, onClose, onSave }: FolderDialogProps) {
  const isEdit = !!folder
  const [name, setName] = useState('')
  const [description, setDescription] = useState('')
  const [icon, setIcon] = useState('folder')
  const [color, setColor] = useState('#3b82f6')
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    if (folder) {
      setName(folder.name)
      setDescription((folder.properties?.description as string) || '')
      setIcon((folder.properties?.icon as string) || 'folder')
      setColor((folder.properties?.color as string) || '#3b82f6')
    }
  }, [folder])

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault()

    if (!name.trim()) {
      setError('Folder name is required')
      return
    }

    try {
      setSaving(true)
      setError(null)
      await onSave({ name: name.trim(), description, icon, color })
      onClose()
    } catch (err: any) {
      setError(err.message || 'Failed to save folder')
    } finally {
      setSaving(false)
    }
  }

  return (
    <div className="fixed inset-0 bg-black/50 backdrop-blur-sm flex items-center justify-center z-50 p-4 overscroll-none">
      <div className="bg-zinc-900 border border-white/10 rounded-xl shadow-2xl max-w-2xl w-full max-h-[90vh] overflow-y-auto overscroll-contain">
        {/* Header */}
        <div className="flex items-center justify-between p-6 border-b border-white/10">
          <h2 className="text-2xl font-bold text-white">
            {isEdit ? 'Edit Folder' : 'Create Folder'}
          </h2>
          <button
            onClick={onClose}
            className="p-2 hover:bg-white/10 rounded-lg transition-colors"
          >
            <X className="w-5 h-5 text-white/60" />
          </button>
        </div>

        {/* Form */}
        <form onSubmit={handleSubmit} className="p-6 space-y-6">
          {error && (
            <div className="p-4 bg-red-500/10 border border-red-500/20 rounded-lg text-red-400">
              {error}
            </div>
          )}

          {/* Name */}
          <div>
            <label htmlFor="name" className="block text-white text-sm font-medium mb-2">
              Folder Name *
            </label>
            <input
              id="name"
              type="text"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="e.g., My Folder"
              disabled={isEdit}
              className="w-full px-4 py-3 bg-white/5 border border-white/10 rounded-lg text-white placeholder-white/40 focus:outline-none focus:ring-2 focus:ring-primary-500 disabled:opacity-50 disabled:cursor-not-allowed"
            />
            {isEdit && (
              <p className="text-white/40 text-xs mt-1">
                Folder name cannot be changed after creation
              </p>
            )}
          </div>

          {/* Description */}
          <div>
            <label htmlFor="description" className="block text-white text-sm font-medium mb-2">
              Description
            </label>
            <textarea
              id="description"
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              placeholder="Brief description of this folder"
              rows={3}
              className="w-full px-4 py-3 bg-white/5 border border-white/10 rounded-lg text-white placeholder-white/40 focus:outline-none focus:ring-2 focus:ring-primary-500 resize-none"
            />
          </div>

          {/* Icon */}
          <div>
            <label className="block text-white text-sm font-medium mb-2">Icon</label>
            <IconPicker value={icon} onChange={setIcon} />
          </div>

          {/* Color */}
          <div>
            <label className="block text-white text-sm font-medium mb-2">Color</label>
            <ColorPicker value={color} onChange={setColor} />
          </div>

          {/* Preview */}
          <div>
            <label className="block text-white text-sm font-medium mb-2">Preview</label>
            <div
              className="relative overflow-hidden rounded-xl p-6 md:p-8"
              style={{
                background: `linear-gradient(135deg, ${color} 0%, ${color}dd 100%)`,
              }}
            >
              <div className="relative">
                <div className="mb-4">
                  <div className="w-12 h-12 md:w-16 md:h-16 rounded-lg bg-white/20 backdrop-blur-sm flex items-center justify-center">
                    <span className="text-white text-2xl">{icon}</span>
                  </div>
                </div>
                <h3 className="text-lg md:text-xl font-semibold text-white mb-1">
                  {name || 'Folder Name'}
                </h3>
                {description && (
                  <p className="text-sm text-white/80 line-clamp-2">{description}</p>
                )}
              </div>
            </div>
          </div>

          {/* Actions */}
          <div className="flex flex-col-reverse md:flex-row gap-3 pt-4">
            <button
              type="button"
              onClick={onClose}
              className="flex-1 px-6 py-3 bg-white/5 hover:bg-white/10 border border-white/10 text-white rounded-lg transition-colors"
            >
              Cancel
            </button>
            <button
              type="submit"
              disabled={saving || !name.trim()}
              className="flex-1 px-6 py-3 bg-primary-500 hover:bg-primary-600 text-white rounded-lg transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
            >
              {saving ? 'Saving...' : isEdit ? 'Update Folder' : 'Create Folder'}
            </button>
          </div>
        </form>
      </div>
    </div>
  )
}
