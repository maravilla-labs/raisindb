import { Routes, Route, Link, Navigate, useLocation, useParams } from 'react-router-dom'
import { Database, Network, Sparkles } from 'lucide-react'
import DatabaseManagementShared from '../components/DatabaseManagementShared'
import GraphAlgorithmsManagement from './management/GraphAlgorithmsManagement'
import ProcessingRulesManagement from './management/ProcessingRulesManagement'

interface TabConfig {
  path: string
  label: string
  icon: typeof Database
}

const tabs: TabConfig[] = [
  { path: 'database', label: 'Database', icon: Database },
  { path: 'graph', label: 'Graph Algorithms', icon: Network },
  { path: 'ai-rules', label: 'AI Rules', icon: Sparkles },
]

/**
 * Repository-level management page
 *
 * Shows database management operations and graph algorithm configurations
 * for a specific repository. The repository selector is disabled since
 * we're already in the repository context.
 */
export default function RepositoryManagement() {
  const { repo } = useParams<{ repo: string }>()
  const location = useLocation()

  if (!repo) {
    return (
      <div className="text-center py-12">
        <p className="text-red-400">Error: Repository not specified in URL</p>
      </div>
    )
  }

  // Determine active tab based on current path
  const getActiveTab = () => {
    const pathParts = location.pathname.split('/')
    const lastPart = pathParts[pathParts.length - 1]
    return tabs.find(tab => tab.path === lastPart) ? lastPart : 'database'
  }

  const activeTab = getActiveTab()

  return (
    <div>
      {/* Header with Tabs */}
      <div className="border-b border-white/10 bg-black/20 backdrop-blur-sm -m-8 mb-0 px-8 pt-0 pb-0">
        <div className="mb-6 pt-0">
          <h1 className="text-4xl font-bold text-white mb-2">Repository Management</h1>
          <p className="text-gray-400">Manage database operations and graph algorithms for this repository</p>
        </div>

        {/* Tab Navigation */}
        <div className="flex gap-2">
          {tabs.map((tab) => {
            const Icon = tab.icon
            const isActive = activeTab === tab.path

            return (
              <Link
                key={tab.path}
                to={`/${repo}/management/${tab.path}`}
                className={`
                  flex items-center gap-2 px-6 py-3 rounded-t-lg font-medium transition-all
                  ${isActive
                    ? 'bg-white/10 text-white border-b-2 border-purple-400'
                    : 'text-gray-400 hover:text-white hover:bg-white/5'
                  }
                `}
              >
                <Icon className="w-5 h-5" />
                {tab.label}
              </Link>
            )
          })}
        </div>
      </div>

      {/* Content Area */}
      <div className="pt-6">
        <Routes>
          <Route index element={<Navigate to="database" replace />} />
          <Route
            path="database"
            element={
              <DatabaseManagementShared
                fixedRepository={repo}
                showBranchSelector={true}
                context="repository"
              />
            }
          />
          <Route path="graph" element={<GraphAlgorithmsManagement repo={repo} />} />
          <Route path="ai-rules" element={<ProcessingRulesManagement repo={repo} />} />
        </Routes>
      </div>
    </div>
  )
}
