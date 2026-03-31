import { useState, useEffect } from 'react'
import { useNavigate } from 'react-router-dom'
import { repositoriesApi, Repository, CreateRepositoryRequest } from '../api/repositories'
import { AlertTriangle, Globe, Check, Database, Plus, Trash2, Calendar, Settings } from 'lucide-react'
import logo from '../assets/raisin-logo.png'
import ConfirmDialog from '../components/ConfirmDialog'

export default function RepositoryList() {
  const [repositories, setRepositories] = useState<Repository[]>([])
  const [loading, setLoading] = useState(true)
  const [showCreate, setShowCreate] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [deleteConfirm, setDeleteConfirm] = useState<{ message: string; onConfirm: () => void } | null>(null)
  const navigate = useNavigate()

  useEffect(() => {
    loadRepositories()
  }, [])

  const loadRepositories = async () => {
    try {
      setLoading(true)
      setError(null)
      const repos = await repositoriesApi.list()
      setRepositories(repos)
      
      // If no repositories, show create dialog
      if (repos.length === 0) {
        setShowCreate(true)
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to load repositories')
    } finally {
      setLoading(false)
    }
  }

  const handleCreate = async (data: CreateRepositoryRequest) => {
    try {
      setError(null)
      await repositoriesApi.create(data)
      await loadRepositories()
      setShowCreate(false)
      navigate(`/${data.repo_id}`)
    } catch (err: any) {
      // Display error in the dialog by re-throwing
      const errorMsg = err.message || 'Failed to create repository'
      setError(errorMsg)
      throw new Error(errorMsg)
    }
  }

  const handleDelete = async (repoId: string) => {
    setDeleteConfirm({
      message: `Delete repository "${repoId}"? This cannot be undone.`,
      onConfirm: async () => {
        try {
          await repositoriesApi.delete(repoId)
          await loadRepositories()
        } catch (err) {
          setError(err instanceof Error ? err.message : 'Failed to delete repository')
        }
      }
    })
  }

  if (loading) {
    return (
      <div className="min-h-screen bg-gradient-to-br from-zinc-900 via-primary-950/20 to-black flex items-center justify-center">
        <div className="text-white text-xl">Loading repositories...</div>
      </div>
    )
  }

  return (
    <div className="min-h-screen bg-gradient-to-br from-zinc-900 via-primary-950/20 to-black p-8">
      <div className="max-w-7xl mx-auto">
        {/* Header with Logo and Branding */}
        <div className="text-center mb-12">
          <div className="flex justify-center mb-6">
            <img src={logo} alt="RaisinDB" className="h-20 md:h-24" />
          </div>
          <h1 className="text-5xl md:text-6xl font-bold text-white mb-3">RaisinDB</h1>
          <p className="text-xl text-zinc-400 mb-2">The Multi-Model Database</p>
          <p className="text-lg text-primary-400 mb-8">With Git-like Workflows</p>
        </div>

        <div className="flex items-center justify-between mb-8">
          <div>
            <h2 className="text-3xl font-bold text-white mb-2">Repositories</h2>
            <p className="text-zinc-400">Manage your content repositories</p>
          </div>
          <div className="flex items-center gap-3">
            <button
              onClick={() => navigate('/management')}
              className="flex items-center gap-2 px-6 py-3 bg-white/5 hover:bg-white/10 border border-white/10 hover:border-white/20 text-white rounded-lg font-semibold transition-all active:scale-95"
            >
              <Settings className="w-5 h-5" />
              System Management
            </button>
            <button
              onClick={() => setShowCreate(true)}
              className="flex items-center gap-2 px-6 py-3 bg-gradient-to-r from-primary-500 to-primary-600 hover:from-primary-600 hover:to-primary-700 text-white rounded-lg font-semibold transition-all shadow-lg shadow-primary-500/20 active:scale-95"
            >
              <Plus className="w-5 h-5" />
              Create Repository
            </button>
          </div>
        </div>

        {error && (
          <div className="mb-6 p-4 bg-gradient-to-br from-red-500/10 to-red-600/5 border border-red-500/30 rounded-xl text-red-200 shadow-lg">
            <div className="flex items-center gap-2">
              <AlertTriangle className="w-5 h-5 text-red-400" />
              <span>{error}</span>
            </div>
          </div>
        )}

        {repositories.length === 0 ? (
          <div className="bg-white/5 backdrop-blur-md border border-white/10 rounded-xl shadow-lg p-16 text-center">
            <div className="p-4 bg-primary-500/10 rounded-full w-24 h-24 mx-auto mb-6 flex items-center justify-center">
              <Database className="w-12 h-12 text-primary-400" />
            </div>
            <h2 className="text-3xl font-bold text-white mb-3">No Repositories Yet</h2>
            <p className="text-zinc-400 mb-8 max-w-md mx-auto">
              Create your first repository to start managing your content with version control
            </p>
            <button
              onClick={() => setShowCreate(true)}
              className="inline-flex items-center gap-2 px-8 py-4 bg-gradient-to-r from-primary-500 to-primary-600 hover:from-primary-600 hover:to-primary-700 text-white rounded-lg font-semibold transition-all shadow-lg shadow-primary-500/20 active:scale-95"
            >
              <Plus className="w-5 h-5" />
              Create Your First Repository
            </button>
          </div>
        ) : (
          <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-6">
            {repositories.map((repo) => (
              <div
                key={repo.repo_id}
                className="bg-white/5 backdrop-blur-md border border-white/10 rounded-xl shadow-lg p-6 cursor-pointer hover:bg-white/10 hover:border-white/20 transition-all group"
                onClick={() => navigate(`/${repo.repo_id}`)}
              >
                <div className="flex items-start justify-between mb-4">
                  <div className="p-3 bg-primary-500/10 rounded-lg border border-primary-500/20">
                    <Database className="w-8 h-8 text-primary-400" />
                  </div>
                  <button
                    onClick={(e) => {
                      e.stopPropagation()
                      handleDelete(repo.repo_id)
                    }}
                    className="flex items-center gap-1 opacity-0 group-hover:opacity-100 transition-all px-3 py-1.5 bg-red-500/10 hover:bg-red-500/20 border border-red-500/20 hover:border-red-500/30 text-red-400 hover:text-red-300 text-sm rounded-lg"
                  >
                    <Trash2 className="w-3.5 h-3.5" />
                    Delete
                  </button>
                </div>
                <h2 className="text-2xl font-bold text-white mb-2 group-hover:text-primary-300 transition-colors">
                  {repo.repo_id}
                </h2>
                {repo.config.description && (
                  <p className="text-zinc-400 text-sm mb-4 line-clamp-2">{repo.config.description}</p>
                )}
                <div className="flex flex-col gap-2 text-xs text-zinc-500">
                  <div className="flex items-center gap-2 px-2 py-1 bg-white/5 rounded">
                    <Globe className="w-3.5 h-3.5 text-primary-400" />
                    <span>Branch: <span className="text-white/80">{repo.config.default_branch || 'main'}</span></span>
                  </div>
                  <div className="flex items-center gap-2 px-2 py-1 bg-white/5 rounded">
                    <Calendar className="w-3.5 h-3.5 text-primary-400" />
                    <span>Created: <span className="text-white/80">{new Date(repo.created_at).toLocaleDateString()}</span></span>
                  </div>
                </div>
              </div>
            ))}
          </div>
        )}

        {showCreate && (
          <CreateRepositoryDialog
            onClose={() => setShowCreate(false)}
            onCreate={handleCreate}
          />
        )}
      </div>
      <ConfirmDialog
        open={deleteConfirm !== null}
        title="Confirm Deletion"
        message={deleteConfirm?.message || ''}
        variant="danger"
        confirmText="Delete"
        onConfirm={() => {
          deleteConfirm?.onConfirm()
          setDeleteConfirm(null)
        }}
        onCancel={() => setDeleteConfirm(null)}
      />
    </div>
  )
}

interface CreateRepositoryDialogProps {
  onClose: () => void
  onCreate: (data: CreateRepositoryRequest) => Promise<void>
}

// Popular languages with flags
const LANGUAGES = [
  { code: 'en', name: 'English', flag: '🇬🇧' },
  { code: 'de', name: 'German', flag: '🇩🇪' },
  { code: 'fr', name: 'French', flag: '🇫🇷' },
  { code: 'es', name: 'Spanish', flag: '🇪🇸' },
  { code: 'it', name: 'Italian', flag: '🇮🇹' },
  { code: 'pt', name: 'Portuguese', flag: '🇵🇹' },
  { code: 'zh', name: 'Chinese', flag: '🇨🇳' },
  { code: 'ja', name: 'Japanese', flag: '🇯🇵' },
  { code: 'ko', name: 'Korean', flag: '🇰🇷' },
  { code: 'ar', name: 'Arabic', flag: '🇸🇦' },
  { code: 'ru', name: 'Russian', flag: '🇷🇺' },
  { code: 'nl', name: 'Dutch', flag: '🇳🇱' },
]

function CreateRepositoryDialog({ onClose, onCreate }: CreateRepositoryDialogProps) {
  const [step, setStep] = useState(1) // Wizard step: 1 = basic info, 2 = language
  const [formData, setFormData] = useState<CreateRepositoryRequest>({
    repo_id: '',
    description: '',
    default_branch: 'main',
    default_language: 'en',
    supported_languages: ['en'],
  })
  const [error, setError] = useState<string | null>(null)
  const [submitting, setSubmitting] = useState(false)
  const [acknowledgedWarning, setAcknowledgedWarning] = useState(false)

  const handleDefaultLanguageChange = (langCode: string) => {
    const currentSupported = formData.supported_languages || []
    setFormData({
      ...formData,
      default_language: langCode,
      supported_languages: currentSupported.includes(langCode)
        ? currentSupported
        : [...currentSupported, langCode],
    })
  }

  const toggleSupportedLanguage = (langCode: string) => {
    if (langCode === formData.default_language) {
      // Cannot remove default language
      return
    }

    const currentSupported = formData.supported_languages || []
    setFormData({
      ...formData,
      supported_languages: currentSupported.includes(langCode)
        ? currentSupported.filter((code) => code !== langCode)
        : [...currentSupported, langCode],
    })
  }

  const handleNextStep = () => {
    setError(null)

    if (!formData.repo_id) {
      setError('Repository ID is required')
      return
    }

    if (!/^[a-z0-9-]+$/.test(formData.repo_id)) {
      setError('Repository ID must be lowercase letters, numbers, and hyphens only')
      return
    }

    setStep(2)
  }

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault()

    // Require acknowledgment in step 2
    if (!acknowledgedWarning) {
      setError('Please acknowledge that the default language cannot be changed after creation')
      return
    }

    try {
      setError(null)
      setSubmitting(true)
      await onCreate(formData)
    } catch (err: any) {
      setError(err.message || 'Failed to create repository')
    } finally {
      setSubmitting(false)
    }
  }

  return (
    <div className="fixed inset-0 bg-black/70 backdrop-blur-md flex items-center justify-center z-50 p-4" onClick={onClose}>
      <div className="bg-zinc-900/95 backdrop-blur-xl border border-white/10 rounded-xl shadow-2xl p-8 max-w-lg w-full" onClick={(e) => e.stopPropagation()}>
        <div className="flex items-center justify-between mb-6">
          <div>
            <h2 className="text-3xl font-bold text-white">Create Repository</h2>
            <p className="text-zinc-400 text-sm mt-1">Step {step} of 2</p>
          </div>
          <button
            onClick={onClose}
            className="text-zinc-400 hover:text-white text-3xl w-10 h-10 flex items-center justify-center hover:bg-white/10 rounded-lg transition-all"
          >
            ×
          </button>
        </div>

        {error && (
          <div className="mb-4 p-4 bg-gradient-to-br from-red-500/10 to-red-600/5 border border-red-500/30 rounded-xl text-red-200 text-sm shadow-lg">
            <div className="flex items-center gap-2">
              <AlertTriangle className="w-4 h-4 text-red-400" />
              <span>{error}</span>
            </div>
          </div>
        )}

        {/* Step 1: Basic Information */}
        {step === 1 && (
          <div className="space-y-4">
            <div>
              <label className="block text-white/80 mb-2 text-sm font-medium">
                Repository ID <span className="text-red-400">*</span>
              </label>
              <input
                type="text"
                value={formData.repo_id}
                onChange={(e) => setFormData({ ...formData, repo_id: e.target.value })}
                placeholder="website"
                className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-zinc-500 focus:outline-none focus:border-primary-500 focus:ring-2 focus:ring-primary-500/20 transition-all"
                autoFocus
              />
              <p className="text-zinc-500 text-xs mt-1">Lowercase letters, numbers, and hyphens only</p>
            </div>

            <div>
              <label className="block text-white/80 mb-2 text-sm font-medium">
                Description
              </label>
              <textarea
                value={formData.description}
                onChange={(e) => setFormData({ ...formData, description: e.target.value })}
                placeholder="Main website content repository"
                rows={3}
                className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-zinc-500 focus:outline-none focus:border-primary-500 focus:ring-2 focus:ring-primary-500/20 transition-all"
              />
            </div>

            <div>
              <label className="block text-white/80 mb-2 text-sm font-medium">
                Default Branch
              </label>
              <input
                type="text"
                value={formData.default_branch}
                onChange={(e) => setFormData({ ...formData, default_branch: e.target.value })}
                className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white focus:outline-none focus:border-primary-500 focus:ring-2 focus:ring-primary-500/20 transition-all"
              />
            </div>

            <div className="flex gap-3 pt-4">
              <button
                type="button"
                onClick={onClose}
                className="flex-1 px-4 py-2.5 bg-white/5 hover:bg-white/10 border border-white/10 text-white rounded-lg font-medium transition-all active:scale-95"
              >
                Cancel
              </button>
              <button
                type="button"
                onClick={handleNextStep}
                className="flex-1 px-4 py-2.5 bg-gradient-to-r from-primary-500 to-primary-600 hover:from-primary-600 hover:to-primary-700 text-white rounded-lg font-medium transition-all shadow-lg shadow-primary-500/20 active:scale-95"
              >
                Next: Language Setup →
              </button>
            </div>
          </div>
        )}

        {/* Step 2: Language Configuration */}
        {step === 2 && (
          <form onSubmit={handleSubmit} className="space-y-4">
            <div className="flex items-start gap-2 mb-4">
              <Globe className="w-5 h-5 text-primary-400 mt-0.5" />
              <div className="flex-1">
                <h3 className="text-lg font-semibold text-white mb-1">Language Configuration</h3>
                <p className="text-zinc-400 text-sm">Choose your repository's default language</p>
              </div>
            </div>

            {/* Warning */}
            <div className="p-3 bg-amber-500/10 border border-amber-500/30 rounded-xl mb-4">
              <div className="flex items-start gap-2">
                <AlertTriangle className="w-4 h-4 text-amber-400 mt-0.5 flex-shrink-0" />
                <p className="text-sm text-amber-100/90">
                  The default language is <strong>permanent</strong> and cannot be changed after creation.
                </p>
              </div>
            </div>

            {/* Default Language Selector */}
            <div>
              <label className="block text-white/80 mb-3 text-sm font-medium">
                Default Language <span className="text-red-400">*</span>
              </label>
              <div className="grid grid-cols-2 sm:grid-cols-3 gap-2">
                {LANGUAGES.map((lang) => (
                  <button
                    key={lang.code}
                    type="button"
                    onClick={() => handleDefaultLanguageChange(lang.code)}
                    className={`
                      p-3 rounded-xl border-2 transition-all text-left
                      ${
                        formData.default_language === lang.code
                          ? 'border-primary-500 bg-primary-500/20 shadow-lg shadow-primary-500/20'
                          : 'border-white/10 bg-white/5 hover:bg-white/10 hover:border-white/20'
                      }
                    `}
                  >
                    <div className="flex items-center gap-2">
                      <span className="text-2xl">{lang.flag}</span>
                      <div className="flex-1 min-w-0">
                        <div className="text-white text-sm font-medium truncate">{lang.name}</div>
                        <div className="text-zinc-400 text-xs">{lang.code}</div>
                      </div>
                      {formData.default_language === lang.code && (
                        <Check className="w-4 h-4 text-primary-400 flex-shrink-0" />
                      )}
                    </div>
                  </button>
                ))}
              </div>
            </div>

            {/* Supported Languages */}
            <div>
              <label className="block text-white/80 mb-3 text-sm font-medium">
                Additional Languages (Optional)
              </label>
              <p className="text-zinc-500 text-xs mb-3">
                You can modify these later in settings
              </p>
              <div className="grid grid-cols-3 gap-2">
                {LANGUAGES.filter((lang) => lang.code !== formData.default_language).map((lang) => {
                  const isSelected = formData.supported_languages?.includes(lang.code)
                  return (
                    <button
                      key={lang.code}
                      type="button"
                      onClick={() => toggleSupportedLanguage(lang.code)}
                      className={`
                        p-2 rounded-lg border transition-all text-left
                        ${
                          isSelected
                            ? 'border-primary-500/50 bg-primary-500/10'
                            : 'border-white/10 bg-white/5 hover:bg-white/10'
                        }
                      `}
                    >
                      <div className="flex items-center gap-1">
                        <span className="text-lg">{lang.flag}</span>
                        <div className="flex-1 min-w-0">
                          <div className="text-white text-xs truncate">{lang.name}</div>
                        </div>
                        {isSelected && <Check className="w-3 h-3 text-primary-400 flex-shrink-0" />}
                      </div>
                    </button>
                  )
                })}
              </div>
            </div>

            {/* Confirmation Checkbox */}
            <div className="border-t border-white/10 pt-4">
              <label className="flex items-start gap-3 cursor-pointer group p-3 bg-red-500/5 border border-red-500/20 rounded-lg hover:bg-red-500/10 transition-colors">
                <input
                  type="checkbox"
                  checked={acknowledgedWarning}
                  onChange={(e) => setAcknowledgedWarning(e.target.checked)}
                  className="mt-0.5 w-4 h-4 rounded border-red-500/50 bg-white/5 text-purple-600 focus:ring-purple-500 focus:ring-offset-0"
                />
                <span className="text-white/90 text-sm flex-1">
                  I understand that <strong className="text-white">
                    {LANGUAGES.find((l) => l.code === formData.default_language)?.name} ({formData.default_language})
                  </strong> cannot be changed after creation
                </span>
              </label>
            </div>

            <div className="flex gap-3 pt-4">
              <button
                type="button"
                onClick={() => setStep(1)}
                disabled={submitting}
                className="flex-1 px-4 py-2.5 bg-white/5 hover:bg-white/10 border border-white/10 text-white rounded-lg font-medium transition-all disabled:opacity-50 active:scale-95"
              >
                ← Back
              </button>
              <button
                type="submit"
                disabled={submitting || !acknowledgedWarning}
                className="flex-1 px-4 py-2.5 bg-gradient-to-r from-primary-500 to-primary-600 hover:from-primary-600 hover:to-primary-700 text-white rounded-lg font-medium transition-all shadow-lg shadow-primary-500/20 disabled:opacity-50 disabled:cursor-not-allowed active:scale-95"
              >
                {submitting ? 'Creating...' : 'Create Repository'}
              </button>
            </div>
          </form>
        )}
      </div>
    </div>
  )
}
