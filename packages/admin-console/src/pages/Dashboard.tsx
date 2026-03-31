import { useEffect, useState } from 'react'
import { Link } from 'react-router-dom'
import { Database, FileType, FolderTree, Activity } from 'lucide-react'
import GlassCard from '../components/GlassCard'
import { workspacesApi } from '../api/workspaces'

export default function Dashboard() {
  const [stats, setStats] = useState({
    workspaces: 0,
    nodeTypes: 0,
    publishedNodeTypes: 0,
  })
  const [loading, setLoading] = useState(true)

  useEffect(() => {
    async function loadStats() {
      try {
        // Use 'main' as default repo for dashboard stats
        // TODO: Could be enhanced to show stats across all repos
        const workspaces = await workspacesApi.list('main')

        setStats({
          workspaces: workspaces.length,
          nodeTypes: 0, // TODO: NodeTypes are now repository-scoped
          publishedNodeTypes: 0, // TODO: NodeTypes are now repository-scoped
        })
      } catch (error) {
        console.error('Failed to load stats:', error)
      } finally {
        setLoading(false)
      }
    }

    loadStats()
  }, [])

  const statCards = [
    {
      icon: FolderTree,
      label: 'Workspaces',
      value: stats.workspaces,
      link: 'workspaces',
      color: 'text-secondary-400',
    },
    {
      icon: FileType,
      label: 'Node Types',
      value: stats.nodeTypes,
      link: 'nodetypes',
      color: 'text-primary-400',
    },
    {
      icon: Activity,
      label: 'Published Types',
      value: stats.publishedNodeTypes,
      link: 'nodetypes',
      color: 'text-green-400',
    },
  ]

  return (
    <div className="animate-fade-in">
      <div className="mb-8">
        <h1 className="text-4xl font-bold text-white mb-2">Dashboard</h1>
        <p className="text-zinc-400">Overview of your RaisinDB instance</p>
      </div>

      {/* Stats Grid */}
      <div className="grid grid-cols-1 md:grid-cols-3 gap-6 mb-8">
        {statCards.map((stat) => (
          <Link key={stat.label} to={stat.link}>
            <GlassCard hover>
              <div className="flex items-center gap-4">
                <div className={`p-3 rounded-lg bg-white/10 ${stat.color}`}>
                  <stat.icon className="w-8 h-8" />
                </div>
                <div>
                  <p className="text-zinc-400 text-sm">{stat.label}</p>
                  <p className="text-3xl font-bold text-white">
                    {loading ? '...' : stat.value}
                  </p>
                </div>
              </div>
            </GlassCard>
          </Link>
        ))}
      </div>

      {/* Quick Actions */}
      <GlassCard>
        <h2 className="text-2xl font-semibold text-white mb-4 flex items-center gap-2">
          <Database className="w-6 h-6 text-primary-400" />
          Quick Actions
        </h2>
        <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
          <Link
            to="nodetypes/new"
            className="p-4 rounded-lg bg-primary-500/20 hover:bg-primary-500/30 border border-primary-400/50 transition-all duration-200 group"
          >
            <h3 className="text-white font-semibold mb-1 group-hover:text-primary-300">
              Create Node Type
            </h3>
            <p className="text-zinc-400 text-sm">
              Define a new content type with properties and validation
            </p>
          </Link>
          <Link
            to="workspaces"
            className="p-4 rounded-lg bg-secondary-500/20 hover:bg-secondary-500/30 border border-secondary-400/50 transition-all duration-200 group"
          >
            <h3 className="text-white font-semibold mb-1 group-hover:text-secondary-300">
              Manage Workspaces
            </h3>
            <p className="text-zinc-400 text-sm">
              Create and configure content workspaces
            </p>
          </Link>
        </div>
      </GlassCard>
    </div>
  )
}
