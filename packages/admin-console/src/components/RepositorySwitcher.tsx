import { useState, useEffect } from 'react'
import { useNavigate } from 'react-router-dom'
import { repositoriesApi, Repository } from '../api/repositories'

interface RepositorySwitcherProps {
  currentRepo?: string
}

export default function RepositorySwitcher({ currentRepo }: RepositorySwitcherProps) {
  const [repositories, setRepositories] = useState<Repository[]>([])
  const [isOpen, setIsOpen] = useState(false)
  const navigate = useNavigate()

  useEffect(() => {
    loadRepositories()
  }, [])

  const loadRepositories = async () => {
    try {
      const repos = await repositoriesApi.list()
      setRepositories(repos)
    } catch (err) {
      console.error('Failed to load repositories:', err)
    }
  }

  const handleSelect = (repoId: string) => {
    setIsOpen(false)
    navigate(`/${repoId}`)
  }

  const currentRepository = repositories.find(r => r.repo_id === currentRepo)

  return (
    <div className="relative">
      <button
        onClick={() => setIsOpen(!isOpen)}
        className="flex items-center gap-2 px-4 py-2 bg-white/5 hover:bg-white/10 border border-white/10 rounded-lg text-white transition-colors"
      >
        <span className="font-semibold">{currentRepository?.repo_id || 'Select Repository'}</span>
        <svg
          className={`w-4 h-4 transition-transform ${isOpen ? 'rotate-180' : ''}`}
          fill="none"
          stroke="currentColor"
          viewBox="0 0 24 24"
        >
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 9l-7 7-7-7" />
        </svg>
      </button>

      {isOpen && (
        <>
          {/* Backdrop */}
          <div
            className="fixed inset-0 z-40"
            onClick={() => setIsOpen(false)}
          />

          {/* Dropdown */}
          <div className="absolute top-full left-0 mt-2 w-64 bg-slate-900 border border-white/10 rounded-lg shadow-xl z-50">
            <div className="p-2 border-b border-white/10">
              <input
                type="text"
                placeholder="Find repository..."
                className="w-full px-3 py-2 bg-white/5 border border-white/10 rounded text-white text-sm placeholder-white/30 focus:outline-none focus:border-purple-500"
                onClick={(e) => e.stopPropagation()}
              />
            </div>

            <div className="max-h-80 overflow-y-auto">
              {repositories.length === 0 ? (
                <div className="p-4 text-white/40 text-sm text-center">
                  No repositories found
                </div>
              ) : (
                repositories.map((repo) => (
                  <button
                    key={repo.repo_id}
                    onClick={() => handleSelect(repo.repo_id)}
                    className={`w-full text-left px-4 py-3 hover:bg-white/5 transition-colors ${
                      repo.repo_id === currentRepo ? 'bg-purple-600/20' : ''
                    }`}
                  >
                    <div className="flex items-center justify-between">
                      <div>
                        <div className="text-white font-medium">{repo.repo_id}</div>
                        {repo.config.description && (
                          <div className="text-white/60 text-xs mt-1">
                            {repo.config.description}
                          </div>
                        )}
                      </div>
                      {repo.repo_id === currentRepo && (
                        <svg className="w-5 h-5 text-purple-400" fill="currentColor" viewBox="0 0 20 20">
                          <path fillRule="evenodd" d="M16.707 5.293a1 1 0 010 1.414l-8 8a1 1 0 01-1.414 0l-4-4a1 1 0 011.414-1.414L8 12.586l7.293-7.293a1 1 0 011.414 0z" clipRule="evenodd" />
                        </svg>
                      )}
                    </div>
                  </button>
                ))
              )}
            </div>

            <div className="p-2 border-t border-white/10">
              <button
                onClick={() => {
                  setIsOpen(false)
                  navigate('/')
                }}
                className="w-full px-4 py-2 text-left text-white/80 hover:text-white hover:bg-white/5 rounded transition-colors text-sm"
              >
                + Create New Repository
              </button>
            </div>
          </div>
        </>
      )}
    </div>
  )
}
