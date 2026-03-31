import { useEffect, useState } from 'react'
import { useParams } from 'react-router-dom'
import { AlertCircle, Check, Globe, Lock, Save, ArrowRight, X, Plus } from 'lucide-react'
import GlassCard from '../../components/GlassCard'
import { repositoriesApi, type Repository } from '../../api/repositories'
import { useToast, ToastContainer } from '../../components/Toast'

// Language configuration with flags and display names
const LANGUAGES = [
  { code: 'en', name: 'English', flag: '🇬🇧', stemming: true },
  { code: 'de', name: 'German', flag: '🇩🇪', stemming: true },
  { code: 'fr', name: 'French', flag: '🇫🇷', stemming: true },
  { code: 'es', name: 'Spanish', flag: '🇪🇸', stemming: true },
  { code: 'it', name: 'Italian', flag: '🇮🇹', stemming: true },
  { code: 'pt', name: 'Portuguese', flag: '🇵🇹', stemming: true },
  { code: 'ru', name: 'Russian', flag: '🇷🇺', stemming: true },
  { code: 'ar', name: 'Arabic', flag: '🇸🇦', stemming: true },
  { code: 'da', name: 'Danish', flag: '🇩🇰', stemming: true },
  { code: 'nl', name: 'Dutch', flag: '🇳🇱', stemming: true },
  { code: 'fi', name: 'Finnish', flag: '🇫🇮', stemming: true },
  { code: 'hu', name: 'Hungarian', flag: '🇭🇺', stemming: true },
  { code: 'no', name: 'Norwegian', flag: '🇳🇴', stemming: true },
  { code: 'ro', name: 'Romanian', flag: '🇷🇴', stemming: true },
  { code: 'sv', name: 'Swedish', flag: '🇸🇪', stemming: true },
  { code: 'tr', name: 'Turkish', flag: '🇹🇷', stemming: true },
  { code: 'zh', name: 'Chinese', flag: '🇨🇳', stemming: false },
  { code: 'ja', name: 'Japanese', flag: '🇯🇵', stemming: false },
  { code: 'ko', name: 'Korean', flag: '🇰🇷', stemming: false },
]

export default function GeneralSettings() {
  const { repo } = useParams<{ repo: string }>()
  const [repository, setRepository] = useState<Repository | null>(null)
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)
  const [selectedLanguages, setSelectedLanguages] = useState<string[]>([])
  const [defaultLanguage, setDefaultLanguage] = useState<string>('')
  const [description, setDescription] = useState<string>('')
  const [defaultBranch, setDefaultBranch] = useState<string>('')
  const [fallbackChains, setFallbackChains] = useState<Record<string, string[]>>({})
  const [success, setSuccess] = useState(false)
  const { toasts, error: showError, success: showSuccess, closeToast } = useToast()

  useEffect(() => {
    if (repo) {
      loadRepository()
    }
  }, [repo])

  async function loadRepository() {
    if (!repo) return
    try {
      const data = await repositoriesApi.get(repo)
      setRepository(data)
      setDefaultLanguage(data.config.default_language)
      setSelectedLanguages(data.config.supported_languages)
      setDescription(data.config.description || '')
      setDefaultBranch(data.config.default_branch)
      setFallbackChains(data.config.locale_fallback_chains || {})
    } catch (error) {
      console.error('Failed to load repository:', error)
    } finally {
      setLoading(false)
    }
  }

  function toggleLanguage(languageCode: string) {
    if (languageCode === defaultLanguage) {
      // Cannot remove default language
      return
    }

    setSelectedLanguages((prev) =>
      prev.includes(languageCode)
        ? prev.filter((code) => code !== languageCode)
        : [...prev, languageCode]
    )
  }

  function addFallbackLocale(locale: string, fallbackLocale: string) {
    setFallbackChains(prev => ({
      ...prev,
      [locale]: [...(prev[locale] || []), fallbackLocale]
    }))
  }

  function removeFallbackLocale(locale: string, index: number) {
    setFallbackChains(prev => ({
      ...prev,
      [locale]: (prev[locale] || []).filter((_, i) => i !== index)
    }))
  }

  function clearFallbackChain(locale: string) {
    setFallbackChains(prev => {
      const updated = { ...prev }
      delete updated[locale]
      return updated
    })
  }

  async function handleSave() {
    if (!repo) return

    setSaving(true)
    setSuccess(false)

    try {
      await repositoriesApi.update(repo, {
        description: description || undefined,
        default_branch: defaultBranch,
        supported_languages: selectedLanguages,
        locale_fallback_chains: fallbackChains,
      })

      setSuccess(true)
      showSuccess('Success', 'Repository settings updated successfully')
      setTimeout(() => setSuccess(false), 3000)
      await loadRepository() // Reload to get updated config
    } catch (error) {
      console.error('Failed to update repository:', error)
      showError('Error', 'Failed to update repository settings')
    } finally {
      setSaving(false)
    }
  }

  const hasChanges =
    repository &&
    (description !== (repository.config.description || '') ||
      defaultBranch !== repository.config.default_branch ||
      JSON.stringify(selectedLanguages.sort()) !==
        JSON.stringify(repository.config.supported_languages.sort()) ||
      JSON.stringify(fallbackChains) !==
        JSON.stringify(repository.config.locale_fallback_chains || {}))

  if (loading) {
    return (
      <div className="flex items-center justify-center h-64">
        <div className="text-white">Loading...</div>
      </div>
    )
  }

  if (!repository) {
    return (
      <div className="flex items-center justify-center h-64">
        <div className="text-red-400">Repository not found</div>
      </div>
    )
  }

  return (
    <div className="animate-fade-in max-w-4xl pt-6">
      {/* General Settings */}
      <GlassCard className="mb-6">
        <h2 className="text-2xl font-semibold text-white mb-4">General</h2>

        <div className="space-y-4">
          <div>
            <label className="block text-sm font-medium text-zinc-300 mb-2">
              Description
            </label>
            <input
              type="text"
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              className="w-full px-4 py-2 bg-zinc-800/50 border border-zinc-700 rounded-lg text-white placeholder-zinc-500 focus:outline-none focus:ring-2 focus:ring-primary-500"
              placeholder="Optional repository description"
            />
          </div>

          <div>
            <label className="block text-sm font-medium text-zinc-300 mb-2">
              Default Branch
            </label>
            <input
              type="text"
              value={defaultBranch}
              onChange={(e) => setDefaultBranch(e.target.value)}
              className="w-full px-4 py-2 bg-zinc-800/50 border border-zinc-700 rounded-lg text-white placeholder-zinc-500 focus:outline-none focus:ring-2 focus:ring-primary-500"
            />
          </div>
        </div>
      </GlassCard>

      {/* Language Settings */}
      <GlassCard className="mb-6">
        <div className="flex items-start gap-3 mb-6">
          <Globe className="w-6 h-6 text-primary-400 mt-1" />
          <div className="flex-1">
            <h2 className="text-2xl font-semibold text-white mb-2">Language Configuration</h2>
            <p className="text-zinc-400 text-sm">
              Configure which languages are supported for content translation and full-text search.
            </p>
          </div>
        </div>

        {/* Immutability Warning */}
        <div className="mb-6 p-4 bg-amber-500/10 border border-amber-500/30 rounded-lg">
          <div className="flex items-start gap-3">
            <Lock className="w-5 h-5 text-amber-400 mt-0.5 flex-shrink-0" />
            <div>
              <h3 className="text-amber-200 font-semibold mb-1">Default Language is Immutable</h3>
              <p className="text-amber-100/80 text-sm">
                The default language <span className="font-mono bg-amber-500/20 px-1.5 py-0.5 rounded">{defaultLanguage}</span> cannot be changed after repository creation.
                This ensures consistency in the full-text search indexing system.
                You can add or remove other supported languages at any time.
              </p>
            </div>
          </div>
        </div>

        {/* Default Language Display */}
        <div className="mb-6">
          <label className="block text-sm font-medium text-zinc-300 mb-3">
            Default Language (Immutable)
          </label>
          <div className="flex items-center gap-3 p-4 bg-primary-500/10 border-2 border-primary-500/30 rounded-lg">
            <span className="text-4xl">{LANGUAGES.find((l) => l.code === defaultLanguage)?.flag}</span>
            <div className="flex-1">
              <div className="text-white font-semibold">
                {LANGUAGES.find((l) => l.code === defaultLanguage)?.name}
              </div>
              <div className="text-xs text-zinc-400 font-mono">{defaultLanguage}</div>
            </div>
            <div className="flex items-center gap-2 text-primary-400">
              <Lock className="w-4 h-4" />
              <span className="text-sm font-medium">Locked</span>
            </div>
          </div>
        </div>

        {/* Supported Languages */}
        <div>
          <label className="block text-sm font-medium text-zinc-300 mb-3">
            Supported Languages
          </label>
          <p className="text-xs text-zinc-500 mb-4">
            Select all languages you want to support for translations. Languages with stemming support provide better search results.
          </p>

          <div className="grid grid-cols-2 md:grid-cols-3 lg:grid-cols-4 gap-3">
            {LANGUAGES.map((language) => {
              const isSelected = selectedLanguages.includes(language.code)
              const isDefault = language.code === defaultLanguage

              return (
                <button
                  key={language.code}
                  onClick={() => !isDefault && toggleLanguage(language.code)}
                  className={`
                    relative p-3 rounded-lg border-2 transition-all
                    ${
                      isDefault
                        ? 'border-primary-500/50 bg-primary-500/10 cursor-not-allowed'
                        : isSelected
                          ? 'border-primary-500 bg-primary-500/20 hover:bg-primary-500/30'
                          : 'border-zinc-700 bg-zinc-800/30 hover:bg-zinc-800/50 hover:border-zinc-600'
                    }
                  `}
                >
                  <div className="flex items-center gap-2 mb-1">
                    <span className="text-2xl">{language.flag}</span>
                    <div className="flex-1 text-left">
                      <div className="text-white text-sm font-medium">{language.name}</div>
                      <div className="text-zinc-500 text-xs font-mono">{language.code}</div>
                    </div>
                    {isSelected && (
                      <Check className="w-4 h-4 text-primary-400 absolute top-2 right-2" />
                    )}
                  </div>
                  {language.stemming && (
                    <div className="text-xs text-zinc-500 flex items-center gap-1">
                      <AlertCircle className="w-3 h-3" />
                      Stemming
                    </div>
                  )}
                  {isDefault && (
                    <div className="absolute inset-0 flex items-center justify-center bg-primary-500/5 rounded-lg">
                      <div className="text-xs font-semibold text-primary-400 bg-primary-500/20 px-2 py-1 rounded">
                        DEFAULT
                      </div>
                    </div>
                  )}
                </button>
              )
            })}
          </div>
        </div>

        <div className="mt-4 text-xs text-zinc-500">
          Selected: {selectedLanguages.length} language{selectedLanguages.length !== 1 ? 's' : ''}
        </div>
      </GlassCard>

      {/* Locale Fallback Chains */}
      <GlassCard className="mb-6">
        <div className="flex items-start gap-3 mb-6">
          <ArrowRight className="w-6 h-6 text-accent-400 mt-1" />
          <div className="flex-1">
            <h2 className="text-2xl font-semibold text-white mb-2">Locale Fallback Chains</h2>
            <p className="text-zinc-400 text-sm">
              Configure fallback chains for each locale. When content is not available in a locale,
              the system will try each locale in the chain until it finds translated content or falls back to the default language.
            </p>
          </div>
        </div>

        <div className="mb-6 p-4 bg-accent-500/10 border border-accent-500/30 rounded-lg">
          <div className="flex items-start gap-3">
            <AlertCircle className="w-5 h-5 text-accent-400 mt-0.5 flex-shrink-0" />
            <div>
              <h3 className="text-accent-200 font-semibold mb-1">How Fallback Chains Work</h3>
              <p className="text-accent-100/80 text-sm">
                Example: <span className="font-mono bg-accent-500/20 px-1.5 py-0.5 rounded">fr-CA → fr → en</span>
                <br />
                If content isn't available in Canadian French (fr-CA), the system will try French (fr),
                then English (en) as the final fallback.
              </p>
            </div>
          </div>
        </div>

        {selectedLanguages.filter(lang => lang !== defaultLanguage).length === 0 ? (
          <div className="text-center py-8 text-zinc-500">
            Add more supported languages to configure fallback chains
          </div>
        ) : (
          <div className="space-y-4">
            {selectedLanguages
              .filter(lang => lang !== defaultLanguage)
              .map(locale => {
                const chain = fallbackChains[locale] || []
                const availableFallbacks = selectedLanguages.filter(
                  lang => lang !== locale && !chain.includes(lang)
                )

                return (
                  <div key={locale} className="p-4 bg-white/5 rounded-lg border border-white/10">
                    <div className="flex items-start justify-between mb-3">
                      <div className="flex items-center gap-2">
                        <span className="text-2xl">{LANGUAGES.find(l => l.code === locale)?.flag}</span>
                        <div>
                          <div className="text-white font-semibold">
                            {LANGUAGES.find(l => l.code === locale)?.name}
                          </div>
                          <div className="text-xs text-zinc-400 font-mono">{locale}</div>
                        </div>
                      </div>
                      {chain.length > 0 && (
                        <button
                          onClick={() => clearFallbackChain(locale)}
                          className="text-xs text-red-400 hover:text-red-300 transition-colors"
                        >
                          Clear chain
                        </button>
                      )}
                    </div>

                    {/* Fallback chain display */}
                    <div className="flex items-center gap-2 mb-3 flex-wrap">
                      <span className="text-sm text-zinc-400 font-mono">{locale}</span>
                      {chain.map((fallbackLocale, index) => (
                        <div key={index} className="flex items-center gap-2">
                          <ArrowRight className="w-4 h-4 text-zinc-500" />
                          <div className="flex items-center gap-1 px-2 py-1 bg-accent-500/20 text-accent-300 rounded text-sm font-mono">
                            <span>{fallbackLocale}</span>
                            <button
                              onClick={() => removeFallbackLocale(locale, index)}
                              className="ml-1 hover:bg-red-500/20 rounded p-0.5 transition-colors"
                              title="Remove"
                            >
                              <X className="w-3 h-3" />
                            </button>
                          </div>
                        </div>
                      ))}
                      <ArrowRight className="w-4 h-4 text-zinc-500" />
                      <span className="text-sm text-primary-400 font-mono">{defaultLanguage}</span>
                    </div>

                    {/* Add fallback dropdown */}
                    {availableFallbacks.length > 0 && (
                      <div className="flex items-center gap-2">
                        <select
                          onChange={(e) => {
                            if (e.target.value) {
                              addFallbackLocale(locale, e.target.value)
                              e.target.value = ''
                            }
                          }}
                          className="px-3 py-1.5 bg-zinc-800/50 border border-zinc-700 rounded text-sm text-white focus:outline-none focus:ring-2 focus:ring-accent-500"
                          defaultValue=""
                        >
                          <option value="" disabled>Add fallback locale...</option>
                          {availableFallbacks.map(fallbackLocale => (
                            <option key={fallbackLocale} value={fallbackLocale}>
                              {LANGUAGES.find(l => l.code === fallbackLocale)?.name} ({fallbackLocale})
                            </option>
                          ))}
                        </select>
                        <Plus className="w-4 h-4 text-zinc-500" />
                      </div>
                    )}
                  </div>
                )
              })}
          </div>
        )}
      </GlassCard>

      {/* Save Button */}
      <div className="flex items-center justify-end gap-4">
        {success && (
          <div className="flex items-center gap-2 text-green-400 animate-fade-in">
            <Check className="w-5 h-5" />
            <span>Settings saved successfully!</span>
          </div>
        )}

        <button
          onClick={handleSave}
          disabled={!hasChanges || saving}
          className="flex items-center gap-2 px-6 py-3 bg-primary-500 hover:bg-primary-600 text-white rounded-lg transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
        >
          <Save className="w-5 h-5" />
          {saving ? 'Saving...' : 'Save Changes'}
        </button>
      </div>
      <ToastContainer toasts={toasts} onClose={closeToast} />
    </div>
  )
}
