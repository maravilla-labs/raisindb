import { useEffect, useState } from 'react'
import { FolderTree } from 'lucide-react'
import { workspacesApi, type Workspace } from '../../api/workspaces'

interface WorkspaceSelectorProps {
  value: string
  onChange: (workspace: string) => void
  repo: string  // Repository ID is now required
  label?: string
  className?: string
}

export default function WorkspaceSelector({ value, onChange, repo, label = 'Workspace', className = '' }: WorkspaceSelectorProps) {
  const [workspaces, setWorkspaces] = useState<Workspace[]>([])
  const [loading, setLoading] = useState(true)

  useEffect(() => {
    const loadWorkspaces = async () => {
      try {
        const data = await workspacesApi.list(repo)
        setWorkspaces(data)
        // Auto-select first workspace if none selected and workspaces exist
        if (!value && data.length > 0) {
          onChange(data[0].name)
        }
      } catch (error) {
        console.error('Failed to load workspaces:', error)
      } finally {
        setLoading(false)
      }
    }

    loadWorkspaces()
  }, [repo])

  if (loading) {
    return (
      <div className={className}>
        <label className="block text-sm font-medium text-gray-300 mb-2">{label}</label>
        <div className="px-4 py-2 bg-white/10 border border-white/20 rounded-lg text-gray-400 animate-pulse">
          Loading workspaces...
        </div>
      </div>
    )
  }

  if (workspaces.length === 0) {
    return (
      <div className={className}>
        <label className="block text-sm font-medium text-gray-300 mb-2">{label}</label>
        <div className="px-4 py-2 bg-red-500/10 border border-red-500/20 rounded-lg text-red-300 text-sm">
          No workspaces found. Please create a workspace first.
        </div>
      </div>
    )
  }

  return (
    <div className={className}>
      <label className="block text-sm font-medium text-gray-300 mb-2">{label}</label>
      <div className="relative">
        <FolderTree className="absolute left-3 top-1/2 transform -translate-y-1/2 w-5 h-5 text-purple-400 pointer-events-none" />
        <select
          value={value}
          onChange={(e) => onChange(e.target.value)}
          className="w-full pl-10 pr-4 py-2 bg-white/10 border border-white/20 rounded-lg text-white focus:outline-none focus:ring-2 focus:ring-purple-500 appearance-none cursor-pointer"
        >
          {workspaces.map((workspace) => (
            <option key={workspace.name} value={workspace.name} className="bg-gray-800">
              {workspace.name}
            </option>
          ))}
        </select>
        <div className="absolute right-3 top-1/2 transform -translate-y-1/2 pointer-events-none">
          <svg className="w-4 h-4 text-gray-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 9l-7 7-7-7" />
          </svg>
        </div>
      </div>
    </div>
  )
}
