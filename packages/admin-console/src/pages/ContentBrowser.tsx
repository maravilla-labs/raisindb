import { useParams, Link } from 'react-router-dom'
import { ArrowLeft, FolderTree } from 'lucide-react'
import GlassCard from '../components/GlassCard'

export default function ContentBrowser() {
  const { repo, workspace } = useParams<{ repo: string; workspace: string }>()

  return (
    <div className="animate-fade-in">
      <div className="mb-8">
        <Link
          to={`/${repo}/workspaces`}
          className="inline-flex items-center gap-2 text-purple-400 hover:text-purple-300 mb-4"
        >
          <ArrowLeft className="w-4 h-4" />
          Back to Workspaces
        </Link>
        <div>
          <h1 className="text-4xl font-bold text-white mb-2">{workspace}</h1>
          <p className="text-gray-400">Browse and manage workspace content</p>
        </div>
      </div>

      <GlassCard>
        <div className="text-center py-12">
          <FolderTree className="w-16 h-16 text-gray-500 mx-auto mb-4" />
          <h3 className="text-xl font-semibold text-white mb-2">Content Browser</h3>
          <p className="text-gray-400 mb-4">
            Content tree navigation and node management coming soon
          </p>
          <p className="text-sm text-gray-500">
            Browse and manage workspace content with tree view, create/edit/move/delete nodes, and edit properties.
          </p>
        </div>
      </GlassCard>
    </div>
  )
}
