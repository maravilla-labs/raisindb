import { Link } from 'react-router-dom'
import { Database, Sparkles, HardDrive, Activity, Users } from 'lucide-react'
import GlassCard from '../components/GlassCard'

/**
 * Tenant-level management hub
 *
 * Dashboard showing all management operations available at the tenant level.
 * Provides quick access to database management, embeddings, and RocksDB operations.
 */
export default function TenantManagement() {
  const managementSections = [
    {
      title: 'Database Management',
      description: 'Manage fulltext and vector indexes across all repositories',
      icon: Database,
      link: '/management/database',
      color: 'primary',
      available: true,
    },
    {
      title: 'AI Configuration',
      description: 'Configure AI providers, models, and settings',
      icon: Sparkles,
      link: '/management/ai',
      color: 'purple',
      available: true,
    },
    {
      title: 'RocksDB Operations',
      description: 'Global database operations (backup, compaction, stats)',
      icon: HardDrive,
      link: '/management/rocksdb',
      color: 'amber',
      available: true, // Enabled for default tenant (local development)
    },
    {
      title: 'Admin Users',
      description: 'Manage admin users and access control for the console',
      icon: Users,
      link: '/management/admin-users',
      color: 'green',
      available: true,
    },
  ]

  const getColorClasses = (color: string, available: boolean) => {
    if (!available) {
      return 'bg-gray-500/10 border-gray-500/20 text-gray-400 hover:bg-gray-500/15'
    }

    switch (color) {
      case 'primary':
        return 'bg-primary-500/10 border-primary-500/20 text-primary-300 hover:bg-primary-500/15'
      case 'purple':
        return 'bg-purple-500/10 border-purple-500/20 text-purple-300 hover:bg-purple-500/15'
      case 'amber':
        return 'bg-amber-500/10 border-amber-500/20 text-amber-300 hover:bg-amber-500/15'
      case 'green':
        return 'bg-green-500/10 border-green-500/20 text-green-300 hover:bg-green-500/15'
      default:
        return 'bg-gray-500/10 border-gray-500/20 text-gray-300 hover:bg-gray-500/15'
    }
  }

  return (
    <div className="animate-fade-in max-w-6xl mx-auto">
      {/* Page Header */}
      <div className="mb-8">
        <div className="flex items-center gap-3 mb-2">
          <Activity className="w-10 h-10 text-primary-400" />
          <h1 className="text-4xl font-bold text-white">Raisin DB Management</h1>
        </div>
        <p className="text-zinc-400">
          Manage tenant-wide operations, indexes, and configurations
        </p>
      </div>

      {/* Management Sections Grid */}
      <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-6">
        {managementSections.map((section) => {
          const Icon = section.icon
          const colorClasses = getColorClasses(section.color, section.available)

          if (section.available) {
            return (
              <Link
                key={section.link}
                to={section.link}
                className="block group"
              >
                <GlassCard className={`h-full transition-all duration-200 ${colorClasses} border`}>
                  <div className="flex flex-col h-full">
                    <div className="flex items-start gap-4 mb-4">
                      <div className={`p-3 rounded-lg ${
                        section.color === 'primary'
                          ? 'bg-primary-500/20'
                          : section.color === 'purple'
                          ? 'bg-purple-500/20'
                          : section.color === 'amber'
                          ? 'bg-amber-500/20'
                          : section.color === 'green'
                          ? 'bg-green-500/20'
                          : 'bg-gray-500/20'
                      }`}>
                        <Icon className="w-6 h-6" />
                      </div>
                      <div className="flex-1">
                        <h3 className="text-lg font-semibold text-white mb-1 group-hover:text-primary-300 transition-colors">
                          {section.title}
                        </h3>
                        <p className="text-sm text-zinc-400">
                          {section.description}
                        </p>
                      </div>
                    </div>

                    <div className="mt-auto pt-4 border-t border-white/10">
                      <div className="flex items-center gap-2 text-sm font-medium">
                        <span>Manage</span>
                        <svg
                          className="w-4 h-4 group-hover:translate-x-1 transition-transform"
                          fill="none"
                          stroke="currentColor"
                          viewBox="0 0 24 24"
                        >
                          <path
                            strokeLinecap="round"
                            strokeLinejoin="round"
                            strokeWidth={2}
                            d="M9 5l7 7-7 7"
                          />
                        </svg>
                      </div>
                    </div>
                  </div>
                </GlassCard>
              </Link>
            )
          }

          // Coming Soon card
          return (
            <div key={section.link} className="block">
              <GlassCard className={`h-full ${colorClasses} border cursor-not-allowed opacity-60`}>
                <div className="flex flex-col h-full">
                  <div className="flex items-start gap-4 mb-4">
                    <div className="p-3 rounded-lg bg-gray-500/20">
                      <Icon className="w-6 h-6" />
                    </div>
                    <div className="flex-1">
                      <div className="flex items-center gap-2 mb-1">
                        <h3 className="text-lg font-semibold text-white">
                          {section.title}
                        </h3>
                        <span className="px-2 py-0.5 bg-amber-500/20 border border-amber-500/30 rounded text-xs text-amber-300">
                          Coming Soon
                        </span>
                      </div>
                      <p className="text-sm text-zinc-400">
                        {section.description}
                      </p>
                    </div>
                  </div>

                  <div className="mt-auto pt-4 border-t border-white/10">
                    <div className="flex items-center gap-2 text-sm font-medium text-gray-500">
                      <span>Not Available</span>
                    </div>
                  </div>
                </div>
              </GlassCard>
            </div>
          )
        })}
      </div>

      {/* Quick Stats (Optional - can be enhanced later) */}
      <GlassCard className="mt-8">
        <h2 className="text-xl font-semibold text-white mb-4">System Overview</h2>
        <div className="grid grid-cols-1 md:grid-cols-3 gap-6">
          <div>
            <p className="text-sm text-zinc-400 mb-1">Tenant</p>
            <p className="text-2xl font-bold text-white">default</p>
          </div>
          <div>
            <p className="text-sm text-zinc-400 mb-1">Active Operations</p>
            <p className="text-2xl font-bold text-white">-</p>
            <p className="text-xs text-zinc-500 mt-1">Real-time job monitoring available in each section</p>
          </div>
          <div>
            <p className="text-sm text-zinc-400 mb-1">Management Level</p>
            <p className="text-2xl font-bold text-primary-400">Tenant-Wide</p>
          </div>
        </div>
      </GlassCard>
    </div>
  )
}
