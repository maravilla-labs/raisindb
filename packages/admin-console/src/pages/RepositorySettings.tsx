import { Routes, Route, Link, Navigate, useLocation, useParams } from 'react-router-dom'
import { Settings, Sparkles } from 'lucide-react'
import GeneralSettings from './repository-settings/GeneralSettings'
import AISettings from './repository-settings/AISettings'

interface TabConfig {
  path: string
  label: string
  icon: typeof Settings
  component: React.ComponentType
}

const tabs: TabConfig[] = [
  { path: 'general', label: 'General', icon: Settings, component: GeneralSettings },
  { path: 'ai', label: 'AI & Embeddings', icon: Sparkles, component: AISettings },
]

export default function RepositorySettings() {
  const location = useLocation()
  const { repo } = useParams<{ repo: string }>()

  // Determine active tab based on current path
  const getActiveTab = () => {
    const pathParts = location.pathname.split('/')
    const lastPart = pathParts[pathParts.length - 1]
    return tabs.find(tab => tab.path === lastPart) ? lastPart : 'general'
  }

  const activeTab = getActiveTab()

  return (
    <div>
      {/* Header with Tabs */}
      <div className="border-b border-white/10 bg-black/20 backdrop-blur-sm -m-8 mb-0 px-8 pt-0 pb-0">
        <div className="mb-6 pt-0">
          <h1 className="text-4xl font-bold text-white mb-2">Repository Settings</h1>
          <p className="text-gray-400">Configure settings for {repo}</p>
        </div>

        {/* Tab Navigation */}
        <div className="flex gap-2">
          {tabs.map((tab) => {
            const Icon = tab.icon
            const isActive = activeTab === tab.path

            return (
              <Link
                key={tab.path}
                to={`/${repo}/settings/${tab.path}`}
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
      <Routes>
        <Route index element={<Navigate to="general" replace />} />
        <Route path="general" element={<GeneralSettings />} />
        <Route path="ai" element={<AISettings />} />
      </Routes>
    </div>
  )
}
