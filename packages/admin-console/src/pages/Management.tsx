import { Routes, Route, Link, Navigate, useLocation } from 'react-router-dom'
import { Activity, Briefcase, Network, TrendingUp, Wrench } from 'lucide-react'
import SystemHealth from './management/SystemHealth'
import JobsManagement from './management/JobsManagement'
import GraphAlgorithmsManagement from './management/GraphAlgorithmsManagement'
import Metrics from './management/Metrics'
import Actions from './management/Actions'

interface TabConfig {
  path: string
  label: string
  icon: typeof Activity
  component: React.ComponentType
}

const tabs: TabConfig[] = [
  { path: 'health', label: 'System Health', icon: Activity, component: SystemHealth },
  { path: 'jobs', label: 'Background Jobs', icon: Briefcase, component: JobsManagement },
  { path: 'graph', label: 'Graph Algorithms', icon: Network, component: GraphAlgorithmsManagement },
  { path: 'metrics', label: 'Metrics', icon: TrendingUp, component: Metrics },
  { path: 'actions', label: 'Actions', icon: Wrench, component: Actions },
]

export default function Management() {
  const location = useLocation()

  // Determine active tab based on current path
  const getActiveTab = () => {
    const currentPath = location.pathname.split('/').pop()
    return tabs.find(tab => tab.path === currentPath) ? currentPath : 'health'
  }

  const activeTab = getActiveTab()

  return (
    <div>
      {/* Header with Tabs */}
      <div className="border-b border-white/10 bg-black/20 backdrop-blur-sm -m-8 mb-0 px-8 pt-0 pb-0">
        <div className="mb-6 pt-0">
          <h1 className="text-4xl font-bold text-white mb-2">System Management</h1>
          <p className="text-gray-400">Monitor and manage your RaisinDB instance</p>
        </div>

        {/* Tab Navigation */}
        <div className="flex gap-2">
          {tabs.map((tab) => {
            const Icon = tab.icon
            const isActive = activeTab === tab.path

            return (
              <Link
                key={tab.path}
                to={tab.path}
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
        <Route index element={<Navigate to="health" replace />} />
        <Route path="health" element={<SystemHealth />} />
        <Route path="jobs" element={<JobsManagement />} />
        <Route path="graph" element={<GraphAlgorithmsManagement />} />
        <Route path="metrics" element={<Metrics />} />
        <Route path="actions" element={<Actions />} />
      </Routes>
    </div>
  )
}
