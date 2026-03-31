import { NavLink, Link, useNavigate } from 'react-router-dom'
import {
  Activity,
  Database,
  Sparkles,
  HardDrive,
  Clock,
  Users,
  User,
  UserCircle,
  LogOut,
  X,
  Terminal,
  Workflow,
  Shield
} from 'lucide-react'
import { useAuth } from '../contexts/AuthContext'
import logoImage from '../assets/raisin-logo.png'

const navItems = [
  { to: '/management', icon: Activity, label: 'Dashboard', end: true },
  { to: '/management/logs', icon: Terminal, label: 'Execution Logs' },
  { to: '/management/flows', icon: Workflow, label: 'Flow Monitor' },
  { to: '/management/database', icon: Database, label: 'Database' },
  { to: '/management/ai', icon: Sparkles, label: 'AI Settings' },
  { to: '/management/auth', icon: Shield, label: 'Auth Settings' },
  { to: '/management/rocksdb', icon: HardDrive, label: 'RocksDB' },
  { to: '/management/jobs', icon: Clock, label: 'All Jobs' },
  { to: '/management/admin-users', icon: Users, label: 'Admin Users' },
  { to: '/management/identity-users', icon: UserCircle, label: 'Identity Users' },
]

interface SidebarProps {
  isOpen: boolean
  isMobile: boolean
  onClose: () => void
}

export default function Sidebar({ isOpen, isMobile, onClose }: SidebarProps) {
  const { user, logout } = useAuth()
  const navigate = useNavigate()

  // Both mobile and desktop use fixed positioning, with smooth transitions
  const baseClasses = "fixed top-0 left-0 glass-dark h-screen w-64 p-6 flex flex-col transition-all duration-300 ease-in-out z-50 border-r border-primary-900/20 select-none overscroll-none"
  const visibilityClasses = isOpen ? 'translate-x-0' : '-translate-x-full'

  // Add shadow when open for depth perception
  const shadowClasses = isOpen ? 'shadow-2xl shadow-black/50' : ''

  const handleLogout = () => {
    logout()
    navigate('/login')
    if (isMobile) onClose()
  }

  return (
    <aside className={`${baseClasses} ${visibilityClasses} ${shadowClasses}`}>
      {/* Mobile close button */}
      {isMobile && (
        <button
          onClick={onClose}
          className="absolute top-4 right-4 p-2 rounded-lg bg-white/5 border border-white/10 text-zinc-400 hover:text-white hover:bg-red-500/10 hover:border-red-400/50 transition-all duration-200 md:hidden"
          aria-label="Close menu"
        >
          <X className="w-5 h-5" />
        </button>
      )}

      {/* Logo */}
      <Link to="/" className="flex items-center gap-3 mb-8 hover:opacity-80 transition-opacity">
        <img src={logoImage} alt="RaisinDB Logo" className="w-10 h-10" />
        <div>
          <h1 className="text-xl font-bold text-white">RaisinDB</h1>
          <p className="text-xs text-primary-300">Admin Console</p>
        </div>
      </Link>

      {/* Navigation */}
      <nav className="flex-1 space-y-2">
        {navItems.map((item) => (
          <NavLink
            key={item.to}
            to={item.to}
            end={item.end}
            onClick={() => isMobile && onClose()}
            className={({ isActive }) =>
              `flex items-center gap-3 px-4 py-3 rounded-lg transition-all duration-200 ${
                isActive
                  ? 'bg-primary-500/30 text-white border border-primary-400/50'
                  : 'text-zinc-300 hover:bg-white/10 hover:text-white'
              }`
            }
          >
            <item.icon className="w-5 h-5" />
            <span className="font-medium">{item.label}</span>
          </NavLink>
        ))}
      </nav>

      {/* Footer */}
      <div className="space-y-4 border-t border-white/10 pt-4">
        {/* User Info - Link to Profile */}
        {user && (
          <NavLink
            to="/management/profile"
            onClick={() => isMobile && onClose()}
            className={({ isActive }) =>
              `flex items-center gap-3 px-3 py-2 rounded-lg transition-all duration-200 ${
                isActive
                  ? 'bg-primary-500/30 border border-primary-400/50'
                  : 'bg-white/5 hover:bg-white/10'
              }`
            }
          >
            <div className="p-1.5 rounded-lg bg-primary-500/20">
              <User className="w-4 h-4 text-primary-400" />
            </div>
            <div className="flex-1 min-w-0">
              <div className="text-sm font-medium text-white truncate">{user.username}</div>
              <div className="text-xs text-zinc-400">My Profile</div>
            </div>
          </NavLink>
        )}

        {/* Logout Button */}
        <button
          onClick={handleLogout}
          className="w-full flex items-center gap-3 px-4 py-3 rounded-lg text-zinc-300 hover:bg-red-500/10 hover:text-red-400 hover:border-red-400/50 border border-transparent transition-all duration-200"
        >
          <LogOut className="w-5 h-5" />
          <span className="font-medium">Logout</span>
        </button>

        {/* Version */}
        <div className="text-xs text-zinc-400 text-center">
          v0.1.0 • {new Date().getFullYear()}
        </div>
      </div>
    </aside>
  )
}
