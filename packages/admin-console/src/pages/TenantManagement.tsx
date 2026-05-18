import { useState } from 'react'
import { Link } from 'react-router-dom'
import {
  Database,
  Sparkles,
  HardDrive,
  Activity,
  Users,
  Globe,
  Server,
  Package,
  Archive,
  Gauge,
} from 'lucide-react'
import GlassCard from '../components/GlassCard'
import { useAuth } from '../contexts/AuthContext'
import { adminManagementApi } from '../api/management'

/**
 * Tenant-level management hub
 *
 * Routed under `/management` and gated to `tenantId === 'default'` (dev /
 * single-operator). In dev mode the hub additionally exposes a "Global /
 * Operator" section that calls the `/management/admin/*` endpoints —
 * server-config style ops that aren't appropriate inside a customer tenant.
 */
export default function TenantManagement() {
  const { tenantId, serverVersion, devMode } = useAuth()

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
      description: 'Tenant-scoped database operations (compaction, stats)',
      icon: HardDrive,
      link: '/management/rocksdb',
      color: 'amber',
      available: true,
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
        })}
      </div>

      {/* Dev-mode bonus: global / operator section.
          Visible only when the server reports dev_mode=true AND the resolved
          tenant is "default". Calls /management/admin/* directly using the
          superadmin token attached by api/client.ts. Hidden in production
          even on the (already-gated) /management hub. */}
      {devMode && tenantId === 'default' && (
        <GlobalOperatorSection />
      )}

      {/* Quick Stats */}
      <GlassCard className="mt-8">
        <h2 className="text-xl font-semibold text-white mb-4">System Overview</h2>
        <div className="grid grid-cols-1 md:grid-cols-3 gap-6">
          <div>
            <p className="text-sm text-zinc-400 mb-1">Tenant</p>
            <p className="text-2xl font-bold text-white">{tenantId}</p>
          </div>
          <div>
            <p className="text-sm text-zinc-400 mb-1">Server Version</p>
            <p className="text-2xl font-bold text-white">v{serverVersion}</p>
            <p className="text-xs text-zinc-500 mt-1">{devMode ? 'Dev / single-operator mode' : 'Production'}</p>
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

/**
 * Global / Operator section — dev mode only.
 *
 * Surfaces the operator-only endpoints under /management/admin/* so that
 * single-operator local development has friction-free access to:
 *   - cross-tenant compact
 *   - backup of every tenant
 *   - listing / toggling server-loaded extensions
 *   - server-wide health & metrics
 *
 * In production the entire /management hub is hidden, so this section is
 * never rendered on customer-facing deployments.
 */
function GlobalOperatorSection() {
  const [compactStatus, setCompactStatus] = useState<string | null>(null)
  const [backupStatus, setBackupStatus] = useState<string | null>(null)
  const [backupPath, setBackupPath] = useState('./backup')
  const [busy, setBusy] = useState<string | null>(null)

  const runGlobalCompact = async () => {
    setBusy('compact')
    setCompactStatus(null)
    try {
      const res = await adminManagementApi.startGlobalCompaction()
      setCompactStatus(
        res.success
          ? `Started global compaction (job ${res.data ?? '?'})`
          : `Failed: ${res.error}`
      )
    } catch (err) {
      setCompactStatus(`Failed: ${err instanceof Error ? err.message : String(err)}`)
    } finally {
      setBusy(null)
    }
  }

  const runBackupAll = async () => {
    setBusy('backup')
    setBackupStatus(null)
    try {
      const res = await adminManagementApi.startBackupAll(backupPath)
      setBackupStatus(
        res.success
          ? `Started global backup (job ${res.data ?? '?'})`
          : `Failed: ${res.error}`
      )
    } catch (err) {
      setBackupStatus(`Failed: ${err instanceof Error ? err.message : String(err)}`)
    } finally {
      setBusy(null)
    }
  }

  return (
    <div className="mt-10">
      <div className="flex items-center gap-3 mb-2">
        <Globe className="w-6 h-6 text-amber-300" />
        <h2 className="text-2xl font-bold text-white">Global / Operator</h2>
        <span className="px-2 py-0.5 bg-amber-500/20 border border-amber-400/30 rounded text-xs text-amber-200">
          Dev mode
        </span>
      </div>
      <p className="text-sm text-zinc-400 mb-6">
        Cross-tenant operations exposed only in dev / single-operator mode.
        These call <code className="text-amber-300">/management/admin/*</code>
        with a superadmin token attached automatically.
      </p>

      <div className="grid grid-cols-1 md:grid-cols-2 gap-4 mb-6">
        {/* Cross-tenant compaction */}
        <GlassCard>
          <div className="flex items-start gap-3 mb-3">
            <div className="p-2 rounded-lg bg-amber-500/20">
              <HardDrive className="w-5 h-5 text-amber-300" />
            </div>
            <div className="flex-1">
              <h3 className="font-semibold text-white">Global Compaction</h3>
              <p className="text-xs text-zinc-400">Compact every tenant's data in one shot.</p>
            </div>
          </div>
          <button
            onClick={runGlobalCompact}
            disabled={busy !== null}
            className="px-4 py-2 bg-amber-500/20 hover:bg-amber-500/30 border border-amber-500/30 rounded-lg text-amber-200 text-sm disabled:opacity-50"
          >
            {busy === 'compact' ? 'Starting…' : 'Start Global Compaction'}
          </button>
          {compactStatus && (
            <p className="text-xs text-zinc-300 mt-3">{compactStatus}</p>
          )}
        </GlassCard>

        {/* Backup all */}
        <GlassCard>
          <div className="flex items-start gap-3 mb-3">
            <div className="p-2 rounded-lg bg-blue-500/20">
              <Archive className="w-5 h-5 text-blue-300" />
            </div>
            <div className="flex-1">
              <h3 className="font-semibold text-white">Backup All Tenants</h3>
              <p className="text-xs text-zinc-400">Snapshot the entire RocksDB instance.</p>
            </div>
          </div>
          <div className="flex gap-2">
            <input
              value={backupPath}
              onChange={(e) => setBackupPath(e.target.value)}
              placeholder="./backup"
              className="flex-1 px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-sm text-white placeholder-zinc-500"
            />
            <button
              onClick={runBackupAll}
              disabled={busy !== null || !backupPath}
              className="px-4 py-2 bg-blue-500/20 hover:bg-blue-500/30 border border-blue-500/30 rounded-lg text-blue-200 text-sm disabled:opacity-50"
            >
              {busy === 'backup' ? 'Starting…' : 'Backup All'}
            </button>
          </div>
          {backupStatus && (
            <p className="text-xs text-zinc-300 mt-3">{backupStatus}</p>
          )}
        </GlassCard>

        {/* Server health link */}
        <Link to="/management/database" className="block group">
          <GlassCard className="h-full">
            <div className="flex items-start gap-3 mb-3">
              <div className="p-2 rounded-lg bg-green-500/20">
                <Server className="w-5 h-5 text-green-300" />
              </div>
              <div className="flex-1">
                <h3 className="font-semibold text-white group-hover:text-green-200 transition-colors">
                  Server Health & Metrics
                </h3>
                <p className="text-xs text-zinc-400">
                  Server-wide health, storage health, vector / replication metrics.
                </p>
              </div>
            </div>
          </GlassCard>
        </Link>

        {/* Dependencies hint */}
        <GlassCard>
          <div className="flex items-start gap-3 mb-3">
            <div className="p-2 rounded-lg bg-purple-500/20">
              <Package className="w-5 h-5 text-purple-300" />
            </div>
            <div className="flex-1">
              <h3 className="font-semibold text-white">Server Extensions</h3>
              <p className="text-xs text-zinc-400">
                List / enable loaded extensions (AI providers, storage backends).
                Available via <code className="text-purple-300">adminManagementApi.listDependencies()</code>.
              </p>
            </div>
          </div>
        </GlassCard>
      </div>

      <div className="flex items-center gap-2 text-xs text-zinc-500">
        <Gauge className="w-3.5 h-3.5" />
        <span>
          These endpoints require <code className="text-zinc-400">RAISIN_SUPERADMIN_TOKEN</code>
          on the server. The SPA attaches it automatically in dev mode.
        </span>
      </div>
    </div>
  )
}
