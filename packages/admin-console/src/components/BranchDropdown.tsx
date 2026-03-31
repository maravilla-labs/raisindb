import { useState, useEffect, useRef } from 'react'
import { createPortal } from 'react-dom'
import { GitBranch, Check, ChevronDown } from 'lucide-react'
import { branchesApi, type Branch } from '../api/branches'

interface BranchDropdownProps {
  repo: string
  currentBranch: string
  onBranchChange: (branch: string) => void
  disabled?: boolean
  className?: string
}

export default function BranchDropdown({
  repo,
  currentBranch,
  onBranchChange,
  disabled = false,
  className = '',
}: BranchDropdownProps) {
  const [branches, setBranches] = useState<Branch[]>([])
  const [isOpen, setIsOpen] = useState(false)
  const [loading, setLoading] = useState(false)
  const [dropdownPosition, setDropdownPosition] = useState({ top: 0, left: 0, width: 0 })
  const buttonRef = useRef<HTMLButtonElement>(null)

  useEffect(() => {
    if (repo) {
      loadBranches()
    }
  }, [repo])

  const loadBranches = async () => {
    setLoading(true)
    try {
      const branchList = await branchesApi.list(repo)
      setBranches(branchList)
    } catch (err) {
      console.error('Failed to load branches:', err)
    } finally {
      setLoading(false)
    }
  }

  // Update dropdown position when it opens
  useEffect(() => {
    if (isOpen && buttonRef.current) {
      const rect = buttonRef.current.getBoundingClientRect()
      setDropdownPosition({
        top: rect.bottom + 8, // 8px gap
        left: rect.left,
        width: rect.width
      })
    }
  }, [isOpen])

  const handleSelect = (branchName: string) => {
    setIsOpen(false)
    onBranchChange(branchName)
  }

  const toggleDropdown = () => {
    if (!disabled) {
      setIsOpen(!isOpen)
    }
  }

  return (
    <div className={className}>
      <button
        ref={buttonRef}
        onClick={toggleDropdown}
        disabled={disabled}
        className={`flex items-center gap-2 px-4 py-2 rounded-lg text-white transition-colors w-full ${
          disabled
            ? 'bg-white/5 border border-white/5 text-white/30 cursor-not-allowed'
            : 'bg-white/5 hover:bg-white/10 border border-white/10'
        }`}
      >
        <GitBranch className="w-4 h-4 flex-shrink-0" />
        <span className="font-medium">{currentBranch}</span>
        {!disabled && (
          <ChevronDown
            className={`w-4 h-4 transition-transform ml-auto ${isOpen ? 'rotate-180' : ''}`}
          />
        )}
      </button>

      {isOpen && !disabled && createPortal(
        <>
          {/* Backdrop */}
          <div
            className="fixed inset-0 z-[9998]"
            onClick={() => setIsOpen(false)}
          />

          {/* Dropdown */}
          <div
            className="fixed bg-zinc-800/95 backdrop-blur-sm border border-zinc-700 rounded-lg shadow-xl z-[9999]"
            style={{
              top: `${dropdownPosition.top}px`,
              left: `${dropdownPosition.left}px`,
              minWidth: '256px'
            }}
          >
            <div className="p-2">
              <div className="text-xs text-zinc-400 px-3 py-2 font-medium">
                Select Branch
              </div>

              <div className="max-h-64 overflow-y-auto">
                {loading ? (
                  <div className="p-4 text-zinc-400 text-sm text-center">
                    Loading branches...
                  </div>
                ) : branches.length === 0 ? (
                  <div className="p-4 text-zinc-400 text-sm text-center">
                    No branches found
                  </div>
                ) : (
                  branches.map((branch) => (
                    <button
                      key={branch.name}
                      onClick={() => handleSelect(branch.name)}
                      className={`w-full text-left px-3 py-2 rounded-md hover:bg-white/5 transition-colors ${
                        branch.name === currentBranch ? 'bg-primary-500/20' : ''
                      }`}
                    >
                      <div className="flex items-center justify-between">
                        <div className="flex-1 min-w-0">
                          <div className="flex items-center gap-2">
                            <GitBranch className="w-3 h-3 text-zinc-400 flex-shrink-0" />
                            <span className="text-white text-sm font-medium truncate">
                              {branch.name}
                            </span>
                            {branch.protected && (
                              <span className="px-1.5 py-0.5 bg-amber-500/20 border border-amber-500/30 rounded text-xs text-amber-300">
                                Protected
                              </span>
                            )}
                          </div>
                          <div className="text-xs text-zinc-500 mt-1 ml-5">
                            Revision: {branch.head}
                          </div>
                        </div>
                        {branch.name === currentBranch && (
                          <Check className="w-4 h-4 text-primary-400 flex-shrink-0 ml-2" />
                        )}
                      </div>
                    </button>
                  ))
                )}
              </div>
            </div>
          </div>
        </>,
        document.body
      )}
    </div>
  )
}
