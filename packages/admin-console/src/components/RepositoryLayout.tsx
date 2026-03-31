import { Outlet, useParams, Link, useLocation, useNavigate } from 'react-router-dom'
import { useEffect, useState } from 'react'
import { repositoriesApi, Repository } from '../api/repositories'
import { useAuth } from '../contexts/AuthContext'
import RepositorySwitcher from './RepositorySwitcher'
import BranchSwitcher from './BranchSwitcher'
import GlobalSearchBar from './GlobalSearchBar'
import SystemUpdateBanner from './SystemUpdateBanner'
import ImpersonationSelector from './ImpersonationSelector'
import logo from '../assets/raisin-logo.png'
import {
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  FileText,
  FolderOpen,
  Tag,
  GitBranch,
  Search,
  Settings,
  Wrench,
  Layers,
  Puzzle,
  Shapes,
  Shield,
  User,
  Users,
  LogOut,
  Code,
  Package,
  Terminal,
  Link2,
  Workflow,
  Bot
} from 'lucide-react'

export default function RepositoryLayout() {
  const { repo, branch } = useParams<{ repo: string; branch?: string }>()
  const [repository, setRepository] = useState<Repository | null>(null)
  const [loading, setLoading] = useState(true)
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false)
  const [modelsExpanded, setModelsExpanded] = useState(true)
  const [accessControlExpanded, setAccessControlExpanded] = useState(true)
  const location = useLocation()
  const navigate = useNavigate()
  const { user, logout } = useAuth()

  // Default to main branch if not specified in URL
  const currentBranch = branch || repository?.config.default_branch || 'main'

  useEffect(() => {
    if (repo) {
      loadRepository()
    }
  }, [repo])

  const loadRepository = async () => {
    try {
      setLoading(true)
      const repoData = await repositoriesApi.get(repo!)
      setRepository(repoData)
    } catch (err) {
      console.error('Failed to load repository:', err)
    } finally {
      setLoading(false)
    }
  }

  const handleLogout = () => {
    logout()
    navigate('/login')
  }

  // Smart branch selection that preserves current route context
  const handleBranchSelect = (branchName: string, _isTag: boolean) => {
    const path = location.pathname

    // Content routes: /repo/content/branch/workspace/*
    if (path.includes('/content/')) {
      const match = path.match(/\/([^/]+)\/content\/[^/]+\/([^/]+)(.*)/)
      if (match) {
        navigate(`/${match[1]}/content/${branchName}/${match[2]}${match[3]}`)
        return
      }
    }

    // Functions routes: /repo/functions/branch/*
    if (path.includes('/functions/')) {
      const match = path.match(/\/([^/]+)\/functions\/[^/]+(.*)/)
      if (match) {
        navigate(`/${match[1]}/functions/${branchName}${match[2]}`)
        return
      }
      // Functions without branch yet: /repo/functions
      const matchNoParams = path.match(/\/([^/]+)\/functions$/)
      if (matchNoParams) {
        navigate(`/${matchNoParams[1]}/functions/${branchName}`)
        return
      }
    }

    // Routes with potential branch segment: /repo/branch/type/* or /repo/type/*
    const routeTypes = ['nodetypes', 'archetypes', 'elementtypes', 'users', 'roles', 'groups', 'circles', 'relation-types', 'agents', 'packages', 'models', 'access-control']

    for (const type of routeTypes) {
      // Pattern with branch: /repo/branch/type/*
      const withBranch = new RegExp(`^/([^/]+)/([^/]+)/${type}(.*)$`)
      const matchWithBranch = path.match(withBranch)
      if (matchWithBranch && !routeTypes.includes(matchWithBranch[2])) {
        navigate(`/${matchWithBranch[1]}/${branchName}/${type}${matchWithBranch[3]}`)
        return
      }

      // Pattern without branch: /repo/type/*
      const withoutBranch = new RegExp(`^/([^/]+)/${type}(.*)$`)
      const matchWithoutBranch = path.match(withoutBranch)
      if (matchWithoutBranch) {
        navigate(`/${matchWithoutBranch[1]}/${branchName}/${type}${matchWithoutBranch[2]}`)
        return
      }
    }

    // Default: for pages like /repo/branches, /repo/settings, /repo/query
    // Just stay on current page - these don't have branch-specific views
  }

  const isActive = (path: string) => location.pathname.startsWith(`/${repo}${path}`)
  const modelsActive =
    isActive('/models') || isActive('/nodetypes') || isActive('/archetypes') || isActive('/elementtypes')
  const accessControlActive =
    isActive('/users') || isActive('/roles') || isActive('/groups') || isActive('/relation-types') || isActive('/access-control') || isActive('/circles')

  useEffect(() => {
    if (modelsActive) {
      setModelsExpanded(true)
    }
  }, [modelsActive])

  useEffect(() => {
    if (accessControlActive) {
      setAccessControlExpanded(true)
    }
  }, [accessControlActive])

  // Check if current route is ContentExplorer, FunctionsIDE, or builder editors (which have their own layout)
  const isContentExplorer = location.pathname.includes('/content/')
  const isFunctionsIDE = location.pathname.includes('/functions')

  // Builder editors need full screen (no padding) - match /repo/archetypes/name or /repo/branch/archetypes/name patterns
  const pathParts = location.pathname.split('/').filter(Boolean)
  const isBuilderEditor = (() => {
    // Check for patterns like: /repo/archetypes/new, /repo/archetypes/:name, /repo/branch/archetypes/new, etc.
    const builderTypes = ['archetypes', 'nodetypes', 'elementtypes']
    for (const type of builderTypes) {
      const typeIndex = pathParts.indexOf(type)
      // If we find the builder type and there's something after it (the name or "new")
      if (typeIndex !== -1 && pathParts.length > typeIndex + 1) {
        return true
      }
    }
    return false
  })()

  if (loading) {
    return (
      <div className="min-h-screen bg-gradient-to-br from-zinc-900 via-primary-950/20 to-black flex items-center justify-center">
        <div className="text-white text-xl">Loading...</div>
      </div>
    )
  }

  return (
    <div className="h-screen flex flex-col bg-gradient-to-br from-zinc-900 via-primary-950/20 to-black">
      {/* Header */}
      <header className="border-b border-white/10 bg-black/30 backdrop-blur-md sticky top-0 z-40 flex-shrink-0 select-none">
        <div className="flex items-center justify-between px-6 py-4 gap-6">
          <div className="flex items-center gap-4 flex-shrink-0">
            <Link to="/" className="flex items-center gap-3 text-xl font-bold text-white hover:text-primary-300 transition-colors">
              <img src={logo} alt="RaisinDB" className="h-8" />
              <span>RaisinDB</span>
            </Link>
            <div className="text-white/30">|</div>
            <RepositorySwitcher currentRepo={repo} />
            {repo && <BranchSwitcher onBranchSelect={handleBranchSelect} />}
          </div>

          {/* Global Search Bar */}
          {repo && (
            <div className="flex-1 max-w-2xl">
              <GlobalSearchBar repo={repo} branch={currentBranch} />
            </div>
          )}

          {repository && (
            <div className="text-white/60 text-sm flex-shrink-0">
              {repository.config.description || repository.repo_id}
            </div>
          )}

          {/* Impersonation Selector - only shown if admin has can_impersonate flag */}
          {repo && (
            <ImpersonationSelector
              repo={repo}
              className="hidden md:flex"
              onChange={() => {
                // Force reload to reflect impersonation change across all pages
                window.location.reload()
              }}
            />
          )}
        </div>
      </header>

      {/* System Updates Banner */}
      {repo && <SystemUpdateBanner tenant="default" repo={repo} />}

      <div className="flex flex-1 overflow-hidden">
        {/* Sidebar */}
        <aside className={`border-r border-white/10 bg-black/30 backdrop-blur-md flex-shrink-0 transition-all duration-300 select-none overscroll-none ${
          sidebarCollapsed ? 'w-16' : 'w-64'
        }`}>
          <nav className="p-4 space-y-1 relative h-full flex flex-col">
            {/* Collapse/Expand Button */}
            <button
              onClick={() => setSidebarCollapsed(!sidebarCollapsed)}
              className="absolute -right-3 top-6 bg-black/50 border border-white/10 rounded-full p-1 text-white/60 hover:text-white hover:bg-black/70 transition-all z-10"
              title={sidebarCollapsed ? 'Expand sidebar' : 'Collapse sidebar'}
            >
              {sidebarCollapsed ? (
                <ChevronRight className="w-4 h-4" />
              ) : (
                <ChevronLeft className="w-4 h-4" />
              )}
            </button>

            <Link
              to={`/${repo}/content`}
              className={`flex items-center gap-3 px-4 py-2 rounded-lg transition-colors ${
                isActive('/content')
                  ? 'bg-primary-500 text-white font-semibold'
                  : 'text-white/80 hover:bg-white/5 hover:text-white'
              } ${sidebarCollapsed ? 'justify-center' : ''}`}
              title={sidebarCollapsed ? 'Content' : ''}
            >
              <FileText className="w-5 h-5 flex-shrink-0" />
              {!sidebarCollapsed && <span>Content</span>}
            </Link>
            <Link
              to={`/${repo}/workspaces`}
              className={`flex items-center gap-3 px-4 py-2 rounded-lg transition-colors ${
                isActive('/workspaces')
                  ? 'bg-primary-500 text-white font-semibold'
                  : 'text-white/80 hover:bg-white/5 hover:text-white'
              } ${sidebarCollapsed ? 'justify-center' : ''}`}
              title={sidebarCollapsed ? 'Workspaces' : ''}
            >
              <FolderOpen className="w-5 h-5 flex-shrink-0" />
              {!sidebarCollapsed && <span>Workspaces</span>}
            </Link>
            <div className="space-y-1">
              <div
                className={`flex items-center rounded-lg transition-colors ${
                  modelsActive
                    ? 'bg-primary-500 text-white font-semibold'
                    : 'text-white/80 hover:bg-white/5 hover:text-white'
                } ${sidebarCollapsed ? 'justify-center' : 'gap-3 px-4 py-2'}`}
                title={sidebarCollapsed ? 'Models' : ''}
              >
                <Link
                  to={`/${repo}/models`}
                  className={`flex items-center gap-3 flex-1 ${sidebarCollapsed ? 'justify-center px-4 py-2' : ''}`}
                >
                  <Layers className="w-5 h-5 flex-shrink-0" />
                  {!sidebarCollapsed && <span>Models</span>}
                </Link>
                {!sidebarCollapsed && (
                  <button
                    type="button"
                    onClick={() => setModelsExpanded((prev) => !prev)}
                    className="p-1 rounded-md hover:bg-white/10 transition-colors"
                    aria-label={modelsExpanded ? 'Collapse models menu' : 'Expand models menu'}
                  >
                    {modelsExpanded ? <ChevronDown className="w-4 h-4" /> : <ChevronRight className="w-4 h-4" />}
                  </button>
                )}
              </div>
              {(modelsExpanded || sidebarCollapsed) && (
                <div className={`${sidebarCollapsed ? 'flex flex-col items-center gap-1' : 'pl-8 space-y-1'}`}>
                  <Link
                    to={`/${repo}/nodetypes`}
                    className={`flex items-center gap-2 px-3 py-1.5 rounded-lg text-sm transition-colors ${
                      isActive('/nodetypes')
                        ? 'bg-primary-400/40 text-white'
                        : 'text-white/70 hover:bg-white/5 hover:text-white'
                    } ${sidebarCollapsed ? 'justify-center w-12 h-12' : ''}`}
                    title={sidebarCollapsed ? 'Node Types' : ''}
                  >
                    <Tag className="w-4 h-4 flex-shrink-0" />
                    {!sidebarCollapsed && <span>Node Types</span>}
                  </Link>
                  <Link
                    to={`/${repo}/archetypes`}
                    className={`flex items-center gap-2 px-3 py-1.5 rounded-lg text-sm transition-colors ${
                      isActive('/archetypes')
                        ? 'bg-primary-400/40 text-white'
                        : 'text-white/70 hover:bg-white/5 hover:text-white'
                    } ${sidebarCollapsed ? 'justify-center w-12 h-12' : ''}`}
                    title={sidebarCollapsed ? 'Archetypes' : ''}
                  >
                    <Puzzle className="w-4 h-4 flex-shrink-0" />
                    {!sidebarCollapsed && <span>Archetypes</span>}
                  </Link>
                  <Link
                    to={`/${repo}/elementtypes`}
                    className={`flex items-center gap-2 px-3 py-1.5 rounded-lg text-sm transition-colors ${
                      isActive('/elementtypes')
                        ? 'bg-primary-400/40 text-white'
                        : 'text-white/70 hover:bg-white/5 hover:text-white'
                    } ${sidebarCollapsed ? 'justify-center w-12 h-12' : ''}`}
                    title={sidebarCollapsed ? 'Elements' : ''}
                  >
                    <Shapes className="w-4 h-4 flex-shrink-0" />
                    {!sidebarCollapsed && <span>Elements</span>}
                  </Link>
                </div>
              )}
            </div>
            <div className="space-y-1">
              <div
                className={`flex items-center rounded-lg transition-colors ${
                  accessControlActive
                    ? 'bg-primary-500 text-white font-semibold'
                    : 'text-white/80 hover:bg-white/5 hover:text-white'
                } ${sidebarCollapsed ? 'justify-center' : 'gap-3 px-4 py-2'}`}
                title={sidebarCollapsed ? 'Access Control' : ''}
              >
                <Link
                  to={`/${repo}/users`}
                  className={`flex items-center gap-3 flex-1 ${sidebarCollapsed ? 'justify-center px-4 py-2' : ''}`}
                >
                  <Shield className="w-5 h-5 flex-shrink-0" />
                  {!sidebarCollapsed && <span>Access Control</span>}
                </Link>
                {!sidebarCollapsed && (
                  <button
                    type="button"
                    onClick={() => setAccessControlExpanded((prev) => !prev)}
                    className="p-1 rounded-md hover:bg-white/10 transition-colors"
                    aria-label={accessControlExpanded ? 'Collapse access control menu' : 'Expand access control menu'}
                  >
                    {accessControlExpanded ? <ChevronDown className="w-4 h-4" /> : <ChevronRight className="w-4 h-4" />}
                  </button>
                )}
              </div>
              {(accessControlExpanded || sidebarCollapsed) && (
                <div className={`${sidebarCollapsed ? 'flex flex-col items-center gap-1' : 'pl-8 space-y-1'}`}>
                  <Link
                    to={`/${repo}/users`}
                    className={`flex items-center gap-2 px-3 py-1.5 rounded-lg text-sm transition-colors ${
                      isActive('/users')
                        ? 'bg-primary-400/40 text-white'
                        : 'text-white/70 hover:bg-white/5 hover:text-white'
                    } ${sidebarCollapsed ? 'justify-center w-12 h-12' : ''}`}
                    title={sidebarCollapsed ? 'Users' : ''}
                  >
                    <User className="w-4 h-4 flex-shrink-0" />
                    {!sidebarCollapsed && <span>Users</span>}
                  </Link>
                  <Link
                    to={`/${repo}/roles`}
                    className={`flex items-center gap-2 px-3 py-1.5 rounded-lg text-sm transition-colors ${
                      isActive('/roles')
                        ? 'bg-primary-400/40 text-white'
                        : 'text-white/70 hover:bg-white/5 hover:text-white'
                    } ${sidebarCollapsed ? 'justify-center w-12 h-12' : ''}`}
                    title={sidebarCollapsed ? 'Roles' : ''}
                  >
                    <Shield className="w-4 h-4 flex-shrink-0" />
                    {!sidebarCollapsed && <span>Roles</span>}
                  </Link>
                  <Link
                    to={`/${repo}/groups`}
                    className={`flex items-center gap-2 px-3 py-1.5 rounded-lg text-sm transition-colors ${
                      isActive('/groups')
                        ? 'bg-primary-400/40 text-white'
                        : 'text-white/70 hover:bg-white/5 hover:text-white'
                    } ${sidebarCollapsed ? 'justify-center w-12 h-12' : ''}`}
                    title={sidebarCollapsed ? 'Groups' : ''}
                  >
                    <Users className="w-4 h-4 flex-shrink-0" />
                    {!sidebarCollapsed && <span>Groups</span>}
                  </Link>
                  <Link
                    to={`/${repo}/relation-types`}
                    className={`flex items-center gap-2 px-3 py-1.5 rounded-lg text-sm transition-colors ${
                      isActive('/relation-types')
                        ? 'bg-primary-400/40 text-white'
                        : 'text-white/70 hover:bg-white/5 hover:text-white'
                    } ${sidebarCollapsed ? 'justify-center w-12 h-12' : ''}`}
                    title={sidebarCollapsed ? 'Relation Types' : ''}
                  >
                    <Link2 className="w-4 h-4 flex-shrink-0" />
                    {!sidebarCollapsed && <span>Relation Types</span>}
                  </Link>
                  <Link
                    to={`/${repo}/access-control/settings`}
                    className={`flex items-center gap-2 px-3 py-1.5 rounded-lg text-sm transition-colors ${
                      isActive('/access-control/settings')
                        ? 'bg-primary-400/40 text-white'
                        : 'text-white/70 hover:bg-white/5 hover:text-white'
                    } ${sidebarCollapsed ? 'justify-center w-12 h-12' : ''}`}
                    title={sidebarCollapsed ? 'Settings' : ''}
                  >
                    <Settings className="w-4 h-4 flex-shrink-0" />
                    {!sidebarCollapsed && <span>Settings</span>}
                  </Link>
                </div>
              )}
            </div>
            <Link
              to={`/${repo}/branches`}
              className={`flex items-center gap-3 px-4 py-2 rounded-lg transition-colors ${
                isActive('/branches')
                  ? 'bg-primary-500 text-white font-semibold'
                  : 'text-white/80 hover:bg-white/5 hover:text-white'
              } ${sidebarCollapsed ? 'justify-center' : ''}`}
              title={sidebarCollapsed ? 'Branches' : ''}
            >
              <GitBranch className="w-5 h-5 flex-shrink-0" />
              {!sidebarCollapsed && <span>Branches</span>}
            </Link>
            <Link
              to={`/${repo}/functions/${currentBranch}`}
              className={`flex items-center gap-3 px-4 py-2 rounded-lg transition-colors ${
                isActive('/functions')
                  ? 'bg-primary-500 text-white font-semibold'
                  : 'text-white/80 hover:bg-white/5 hover:text-white'
              } ${sidebarCollapsed ? 'justify-center' : ''}`}
              title={sidebarCollapsed ? 'Functions' : ''}
            >
              <Code className="w-5 h-5 flex-shrink-0" />
              {!sidebarCollapsed && <span>Functions</span>}
            </Link>
            <Link
              to={`/${repo}/logs`}
              className={`flex items-center gap-3 px-4 py-2 rounded-lg transition-colors ${
                isActive('/logs')
                  ? 'bg-primary-500 text-white font-semibold'
                  : 'text-white/80 hover:bg-white/5 hover:text-white'
              } ${sidebarCollapsed ? 'justify-center' : ''}`}
              title={sidebarCollapsed ? 'Logs' : ''}
            >
              <Terminal className="w-5 h-5 flex-shrink-0" />
              {!sidebarCollapsed && <span>Execution Logs</span>}
            </Link>
            <Link
              to={`/${repo}/flows`}
              className={`flex items-center gap-3 px-4 py-2 rounded-lg transition-colors ${
                isActive('/flows')
                  ? 'bg-primary-500 text-white font-semibold'
                  : 'text-white/80 hover:bg-white/5 hover:text-white'
              } ${sidebarCollapsed ? 'justify-center' : ''}`}
              title={sidebarCollapsed ? 'Flows' : ''}
            >
              <Workflow className="w-5 h-5 flex-shrink-0" />
              {!sidebarCollapsed && <span>Flow Instances</span>}
            </Link>
            <Link
              to={`/${repo}/agents`}
              className={`flex items-center gap-3 px-4 py-2 rounded-lg transition-colors ${
                isActive('/agents')
                  ? 'bg-primary-500 text-white font-semibold'
                  : 'text-white/80 hover:bg-white/5 hover:text-white'
              } ${sidebarCollapsed ? 'justify-center' : ''}`}
              title={sidebarCollapsed ? 'AI' : ''}
            >
              <Bot className="w-5 h-5 flex-shrink-0" />
              {!sidebarCollapsed && <span>AI</span>}
            </Link>
            <Link
              to={`/${repo}/packages`}
              className={`flex items-center gap-3 px-4 py-2 rounded-lg transition-colors ${
                isActive('/packages')
                  ? 'bg-primary-500 text-white font-semibold'
                  : 'text-white/80 hover:bg-white/5 hover:text-white'
              } ${sidebarCollapsed ? 'justify-center' : ''}`}
              title={sidebarCollapsed ? 'Packages' : ''}
            >
              <Package className="w-5 h-5 flex-shrink-0" />
              {!sidebarCollapsed && <span>Packages</span>}
            </Link>
            <Link
              to={`/${repo}/query`}
              className={`flex items-center gap-3 px-4 py-2 rounded-lg transition-colors ${
                isActive('/query')
                  ? 'bg-primary-500 text-white font-semibold'
                  : 'text-white/80 hover:bg-white/5 hover:text-white'
              } ${sidebarCollapsed ? 'justify-center' : ''}`}
              title={sidebarCollapsed ? 'Query' : ''}
            >
              <Search className="w-5 h-5 flex-shrink-0" />
              {!sidebarCollapsed && <span>Query</span>}
            </Link>
            <Link
              to={`/${repo}/settings`}
              className={`flex items-center gap-3 px-4 py-2 rounded-lg transition-colors ${
                isActive('/settings')
                  ? 'bg-primary-500 text-white font-semibold'
                  : 'text-white/80 hover:bg-white/5 hover:text-white'
              } ${sidebarCollapsed ? 'justify-center' : ''}`}
              title={sidebarCollapsed ? 'Settings' : ''}
            >
              <Settings className="w-5 h-5 flex-shrink-0" />
              {!sidebarCollapsed && <span>Settings</span>}
            </Link>
            <Link
              to={`/${repo}/management`}
              className={`flex items-center gap-3 px-4 py-2 rounded-lg transition-colors ${
                isActive('/management')
                  ? 'bg-primary-500 text-white font-semibold'
                  : 'text-white/80 hover:bg-white/5 hover:text-white'
              } ${sidebarCollapsed ? 'justify-center' : ''}`}
              title={sidebarCollapsed ? 'Management' : ''}
            >
              <Wrench className="w-5 h-5 flex-shrink-0" />
              {!sidebarCollapsed && <span>Management</span>}
            </Link>

            {/* Spacer to push footer to bottom */}
            <div className="flex-1"></div>

            {/* Footer with user info and logout */}
            {!sidebarCollapsed && (
              <div className="space-y-3 border-t border-white/10 pt-4 mt-4">
                {/* User Info */}
                {user && (
                  <div className="px-3 py-2 rounded-lg bg-white/5">
                    <div className="text-sm font-medium text-white truncate">{user.username}</div>
                    <div className="text-xs text-white/60">Logged in</div>
                  </div>
                )}

                {/* Logout Button */}
                <button
                  onClick={handleLogout}
                  className="w-full flex items-center gap-3 px-4 py-2 rounded-lg text-white/80 hover:bg-red-500/10 hover:text-red-400 hover:border-red-400/50 border border-transparent transition-all duration-200"
                >
                  <LogOut className="w-4 h-4" />
                  <span className="text-sm font-medium">Logout</span>
                </button>
              </div>
            )}

            {/* Collapsed sidebar - just logout icon */}
            {sidebarCollapsed && (
              <div className="border-t border-white/10 pt-4 mt-4">
                <button
                  onClick={handleLogout}
                  className="w-full flex items-center justify-center p-2 rounded-lg text-white/80 hover:bg-red-500/10 hover:text-red-400 transition-all duration-200"
                  title="Logout"
                >
                  <LogOut className="w-5 h-5" />
                </button>
              </div>
            )}
          </nav>
        </aside>

        {/* Main content */}
        <main
          key={location.pathname.split('/')[2] || 'index'}
          className={`flex-1 overflow-auto overscroll-contain ${isContentExplorer || isFunctionsIDE || isBuilderEditor ? '' : 'p-6 md:p-8'}`}
        >
          <Outlet />
        </main>
      </div>
    </div>
  )
}
