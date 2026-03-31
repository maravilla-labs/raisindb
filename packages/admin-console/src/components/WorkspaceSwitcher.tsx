import { useState, useEffect, useRef } from 'react'
import { createPortal } from 'react-dom'
import { useNavigate } from 'react-router-dom'
import { FolderTree, ChevronDown, Check } from 'lucide-react'
import { workspacesApi, type Workspace } from '../api/workspaces'
import { useRepositoryContext } from '../hooks/useRepositoryContext'

interface WorkspaceSwitcherProps {
  className?: string
}

export default function WorkspaceSwitcher({ className = '' }: WorkspaceSwitcherProps) {
  const { repo, branch, workspace } = useRepositoryContext()
  const navigate = useNavigate()
  const [workspaces, setWorkspaces] = useState<Workspace[]>([])
  const [isOpen, setIsOpen] = useState(false)
  const [loading, setLoading] = useState(true)
  const [buttonRect, setButtonRect] = useState<DOMRect | null>(null)
  const dropdownRef = useRef<HTMLDivElement>(null)
  const buttonRef = useRef<HTMLButtonElement>(null)

  useEffect(() => {
    if (repo) {
      loadWorkspaces()
    }
  }, [repo])

  useEffect(() => {
    function handleClickOutside(event: MouseEvent) {
      if (dropdownRef.current && !dropdownRef.current.contains(event.target as Node)) {
        setIsOpen(false)
      }
    }

    if (isOpen) {
      // Update button position when dropdown opens
      if (buttonRef.current) {
        setButtonRect(buttonRef.current.getBoundingClientRect())
      }
      document.addEventListener('mousedown', handleClickOutside)
      return () => document.removeEventListener('mousedown', handleClickOutside)
    }
  }, [isOpen])

  async function loadWorkspaces() {
    if (!repo) return
    try {
      const data = await workspacesApi.list(repo)
      setWorkspaces(data)
    } catch (error) {
      console.error('Failed to load workspaces:', error)
    } finally {
      setLoading(false)
    }
  }

  function handleWorkspaceSelect(workspaceName: string) {
    // Navigate to the new workspace while preserving repo and branch
    navigate(`/${repo}/content/${branch}/${workspaceName}`)
    setIsOpen(false)
  }

  if (loading) {
    return (
      <div className={`flex items-center gap-2 px-3 py-1.5 bg-white/5 rounded-lg ${className}`}>
        <FolderTree className="w-4 h-4 text-gray-400" />
        <span className="text-sm text-gray-400">Loading...</span>
      </div>
    )
  }

  return (
    <div className={`relative ${className}`}>
      <button
        ref={buttonRef}
        onClick={() => setIsOpen(!isOpen)}
        className="flex items-center gap-2 px-3 py-1.5 bg-white/5 hover:bg-white/10 border border-white/10 hover:border-white/20 rounded-lg transition-all group"
      >
        <FolderTree className="w-4 h-4 text-purple-400" />
        <span className="text-sm font-medium text-white">{workspace}</span>
        <ChevronDown className={`w-4 h-4 text-gray-400 transition-transform ${isOpen ? 'rotate-180' : ''}`} />
      </button>

      {isOpen && buttonRect && createPortal(
        <div 
          ref={dropdownRef}
          className="fixed w-64 bg-zinc-900 border border-white/20 rounded-lg shadow-xl overflow-hidden"
          style={{ 
            top: `${buttonRect.bottom + 8}px`,
            left: `${buttonRect.left}px`,
            zIndex: 9999
          }}
        >
          <div className="max-h-96 overflow-y-auto">
            <div className="px-3 py-2 border-b border-white/10">
              <p className="text-xs text-gray-400 font-medium uppercase">Select Workspace</p>
            </div>
            {workspaces.length === 0 ? (
              <div className="px-4 py-8 text-center text-gray-500 text-sm">
                No workspaces found
              </div>
            ) : (
              workspaces.map((ws) => (
                <button
                  key={ws.name}
                  onClick={() => handleWorkspaceSelect(ws.name)}
                  className={`w-full text-left px-4 py-2 hover:bg-white/10 transition-colors flex items-center justify-between ${
                    ws.name === workspace ? 'bg-purple-500/20' : ''
                  }`}
                >
                  <div className="flex items-center gap-3">
                    <FolderTree className={`w-4 h-4 ${ws.name === workspace ? 'text-purple-400' : 'text-gray-400'}`} />
                    <div>
                      <div className={`text-sm font-medium ${ws.name === workspace ? 'text-white' : 'text-gray-300'}`}>
                        {ws.name}
                      </div>
                      {ws.description && (
                        <div className="text-xs text-gray-500 truncate max-w-[200px]">
                          {ws.description}
                        </div>
                      )}
                    </div>
                  </div>
                  {ws.name === workspace && (
                    <Check className="w-4 h-4 text-purple-400" />
                  )}
                </button>
              ))
            )}
          </div>
        </div>,
        document.body
      )}
    </div>
  )
}
