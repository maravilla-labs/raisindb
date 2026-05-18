// SPDX-License-Identifier: BSL-1.1

import { useState, useEffect } from 'react'
import { Outlet, useLocation } from 'react-router-dom'
import { Menu, PanelLeftClose, PanelLeftOpen, Building2 } from 'lucide-react'
import Sidebar from './Sidebar'
import { useAuth } from '../contexts/AuthContext'

export default function Layout() {
  const [sidebarOpen, setSidebarOpen] = useState(true)
  const [isMobile, setIsMobile] = useState(false)
  const location = useLocation()
  const { tenantId, serverVersion } = useAuth()

  // Detect mobile viewport
  useEffect(() => {
    const checkMobile = () => {
      const mobile = window.innerWidth < 768
      setIsMobile(mobile)
      // Auto-close sidebar on mobile
      if (mobile) {
        setSidebarOpen(false)
      }
    }

    checkMobile()
    window.addEventListener('resize', checkMobile)
    return () => window.removeEventListener('resize', checkMobile)
  }, [])

  return (
    <div className="flex h-screen overflow-hidden bg-gradient-to-br from-zinc-950 via-primary-950/30 to-black">
      {/* Mobile backdrop */}
      {isMobile && sidebarOpen && (
        <div
          className="fixed inset-0 bg-black/60 backdrop-blur-sm z-40 md:hidden"
          onClick={() => setSidebarOpen(false)}
        />
      )}

      {/* Sidebar - Fixed position */}
      <Sidebar
        isOpen={sidebarOpen}
        isMobile={isMobile}
        onClose={() => setSidebarOpen(false)}
      />

      {/* Main content - Transitions margin when sidebar opens/closes */}
      <div
        className={`flex-1 flex flex-col overflow-hidden transition-all duration-300 ease-in-out ${
          !isMobile && sidebarOpen ? 'md:ml-64' : 'ml-0'
        }`}
      >
        {/* Mobile header with hamburger */}
        <header className="md:hidden flex items-center gap-3 px-4 py-3 bg-black/30 backdrop-blur-md border-b border-white/10 select-none">
          <button
            onClick={() => setSidebarOpen(!sidebarOpen)}
            className="flex-shrink-0 p-1 text-white hover:text-primary-400 transition-colors"
            aria-label="Toggle menu"
          >
            <Menu className="w-6 h-6" />
          </button>
          <h1 className="text-lg font-bold text-white truncate">RaisinDB Admin</h1>
        </header>

        {/* Desktop header with hamburger */}
        <header className="hidden md:flex items-center gap-4 px-6 py-4 bg-black/30 backdrop-blur-md border-b border-white/10 transition-all duration-300 select-none">
          <button
            onClick={() => setSidebarOpen(!sidebarOpen)}
            className="group relative flex-shrink-0 p-2.5 rounded-lg bg-white/5 border border-white/10 text-white hover:text-primary-400 hover:border-primary-400/50 hover:bg-primary-500/10 transition-all duration-300 hover:scale-105 active:scale-95"
            aria-label={sidebarOpen ? 'Close sidebar' : 'Open sidebar'}
          >
            <div className="relative w-5 h-5">
              {/* Animated icon transition */}
              <PanelLeftClose
                className={`absolute inset-0 w-5 h-5 transition-all duration-300 ${
                  sidebarOpen
                    ? 'opacity-100 rotate-0'
                    : 'opacity-0 rotate-90'
                }`}
              />
              <PanelLeftOpen
                className={`absolute inset-0 w-5 h-5 transition-all duration-300 ${
                  !sidebarOpen
                    ? 'opacity-100 rotate-0'
                    : 'opacity-0 -rotate-90'
                }`}
              />
            </div>
            {/* Tooltip */}
            <span className="absolute left-full ml-2 px-2 py-1 bg-black/90 text-white text-xs rounded whitespace-nowrap opacity-0 group-hover:opacity-100 transition-opacity pointer-events-none">
              {sidebarOpen ? 'Close sidebar' : 'Open sidebar'}
            </span>
          </button>
          <div className="flex-1">
            <h1 className="text-xl font-bold text-white">
              {location.pathname.startsWith('/management')
                ? 'Raisin DB Management Console'
                : 'RaisinDB Admin Console'}
            </h1>
          </div>

          {/* Tenant badge — read-only. Always visible so the operator knows
              which tenant the SPA is talking to. There is no switcher. */}
          <div
            className="flex items-center gap-2 px-3 py-1.5 rounded-lg bg-primary-500/10 border border-primary-400/30 text-primary-200"
            title="Resolved tenant for this session (set via x-tenant-id header at the edge)"
          >
            <Building2 className="w-4 h-4" />
            <span className="text-xs uppercase tracking-wider text-primary-300/70">Tenant</span>
            <span className="text-sm font-semibold text-white">{tenantId}</span>
          </div>
        </header>

        {/* Page content */}
        <main className="flex-1 overflow-auto overscroll-contain p-4 md:p-8">
          <Outlet />
        </main>

        {/* Footer — server version. Sourced from /api/admin/bootstrap so it
            stays in sync with the actual running binary. */}
        <footer className="px-4 md:px-8 py-2 text-[11px] text-zinc-500 border-t border-white/5 bg-black/20 backdrop-blur-sm flex justify-between items-center select-none">
          <span>raisindb v{serverVersion}</span>
          <span className="text-zinc-600">{new Date().getFullYear()}</span>
        </footer>
      </div>
    </div>
  )
}
