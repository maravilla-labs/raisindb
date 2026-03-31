import { useState, useEffect, useRef } from 'react'
import { createPortal } from 'react-dom'
import { Languages, Check, ChevronDown, Search } from 'lucide-react'
import { translationsApi, type TranslationConfig } from '../api/translations'
import { useRepositoryContext } from '../hooks/useRepositoryContext'

interface LanguageSwitcherProps {
  /** Optional className for styling */
  className?: string
  /** Show in compact mode (for mobile/tight spaces) */
  compact?: boolean
  /** Callback when locale changes */
  onLocaleChange?: (locale: string) => void
}

// Hardcoded map for common locale display names
const localeNames: Record<string, string> = {
  'en': 'English',
  'fr': 'Français',
  'de': 'Deutsch',
  'es': 'Español',
  'it': 'Italiano',
  'pt': 'Português',
  'ja': '日本語',
  'zh': '中文',
  'ko': '한국어',
  'ru': 'Русский',
  'ar': 'العربية',
  'hi': 'हिन्दी',
}

const LOCALE_STORAGE_KEY = 'raisin:currentLocale'

export default function LanguageSwitcher({
  className = '',
  compact = false,
  onLocaleChange
}: LanguageSwitcherProps) {
  const { repo } = useRepositoryContext()
  const [isOpen, setIsOpen] = useState(false)
  const [config, setConfig] = useState<TranslationConfig | null>(null)
  const [currentLocale, setCurrentLocale] = useState<string>('')
  const [searchTerm, setSearchTerm] = useState('')
  const [loading, setLoading] = useState(false)
  const [buttonRect, setButtonRect] = useState<DOMRect | null>(null)
  const dropdownRef = useRef<HTMLDivElement>(null)
  const buttonRef = useRef<HTMLButtonElement>(null)

  // Load translation config when component mounts or repo changes
  useEffect(() => {
    if (repo) {
      loadTranslationConfig()
    }
  }, [repo])

  // Initialize current locale from localStorage or default
  useEffect(() => {
    if (config) {
      const stored = localStorage.getItem(LOCALE_STORAGE_KEY)
      const initialLocale = stored && config.supported_languages.includes(stored)
        ? stored
        : config.default_language
      setCurrentLocale(initialLocale)

      // Notify parent of initial locale
      if (onLocaleChange) {
        onLocaleChange(initialLocale)
      }
    }
  }, [config, onLocaleChange])

  // Close dropdown when clicking outside
  useEffect(() => {
    function handleClickOutside(event: MouseEvent) {
      if (dropdownRef.current && !dropdownRef.current.contains(event.target as Node)) {
        setIsOpen(false)
      }
    }

    function handleEscape(event: KeyboardEvent) {
      if (event.key === 'Escape') {
        setIsOpen(false)
      }
    }

    if (isOpen) {
      // Update button position when dropdown opens
      if (buttonRef.current) {
        setButtonRect(buttonRef.current.getBoundingClientRect())
      }
      document.addEventListener('mousedown', handleClickOutside)
      document.addEventListener('keydown', handleEscape)
      return () => {
        document.removeEventListener('mousedown', handleClickOutside)
        document.removeEventListener('keydown', handleEscape)
      }
    }
  }, [isOpen])

  async function loadTranslationConfig() {
    setLoading(true)
    try {
      const data = await translationsApi.getConfig(repo)
      setConfig(data)
    } catch (error) {
      console.error('Failed to load translation config:', error)
    } finally {
      setLoading(false)
    }
  }

  function handleLocaleSelect(locale: string) {
    setCurrentLocale(locale)
    localStorage.setItem(LOCALE_STORAGE_KEY, locale)

    if (onLocaleChange) {
      onLocaleChange(locale)
    }

    setIsOpen(false)
    setSearchTerm('')
  }

  function getLocaleDisplayName(locale: string): string {
    const nativeName = localeNames[locale]
    return nativeName ? `${locale} (${nativeName})` : locale
  }

  // Filter languages by search term
  const filteredLocales = config?.supported_languages.filter(locale =>
    locale.toLowerCase().includes(searchTerm.toLowerCase()) ||
    localeNames[locale]?.toLowerCase().includes(searchTerm.toLowerCase())
  ) || []

  if (!config || !currentLocale) {
    return null
  }

  return (
    <div className={`relative ${className}`}>
      {/* Trigger Button */}
      <button
        ref={buttonRef}
        onClick={() => setIsOpen(!isOpen)}
        className={`
          flex items-center gap-2 px-3 py-1.5
          bg-black/30 hover:bg-black/40
          border border-white/20 hover:border-white/30
          rounded-lg text-white transition-colors
          ${compact ? 'text-sm' : ''}
        `}
        aria-label="Select language"
        aria-expanded={isOpen}
        aria-haspopup="listbox"
      >
        <Languages className="w-4 h-4 text-primary-400" />
        <span className="font-medium">{currentLocale.toUpperCase()}</span>
        <ChevronDown className={`w-4 h-4 text-gray-400 transition-transform ${isOpen ? 'rotate-180' : ''}`} />
      </button>

      {/* Dropdown Menu */}
      {isOpen && buttonRect && createPortal(
        <div
          ref={dropdownRef}
          className="fixed w-80 bg-zinc-900 border border-white/20 rounded-lg shadow-2xl overflow-hidden"
          style={{
            top: `${buttonRect.bottom + 8}px`,
            left: `${buttonRect.left}px`,
            zIndex: 9999
          }}
          role="listbox"
          aria-label="Select language"
        >
          {/* Header */}
          <div className="px-4 py-3 border-b border-white/10">
            <div className="flex items-center gap-2 text-white font-medium">
              <Languages className="w-4 h-4 text-primary-400" />
              Select Language
            </div>
          </div>

          {/* Search */}
          <div className="p-3 border-b border-white/10">
            <div className="relative">
              <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-gray-400" />
              <input
                type="text"
                placeholder="Search languages..."
                value={searchTerm}
                onChange={(e) => setSearchTerm(e.target.value)}
                className="w-full pl-10 pr-4 py-2 bg-black/30 border border-white/20 rounded-lg text-white text-sm placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-primary-500"
                autoFocus
                aria-label="Search languages"
              />
            </div>
          </div>

          {/* List */}
          <div className="max-h-80 overflow-y-auto">
            {loading ? (
              <div className="p-8 text-center text-gray-400">
                <div className="animate-spin w-6 h-6 border-2 border-primary-400 border-t-transparent rounded-full mx-auto mb-2" />
                Loading...
              </div>
            ) : filteredLocales.length > 0 ? (
              <div className="py-2">
                {filteredLocales.map((locale) => (
                  <button
                    key={locale}
                    onClick={() => handleLocaleSelect(locale)}
                    className="w-full flex items-center gap-3 px-4 py-2.5 hover:bg-white/5 transition-colors text-left"
                    role="option"
                    aria-selected={locale === currentLocale}
                  >
                    <Languages className="w-4 h-4 text-primary-400 flex-shrink-0" />
                    <div className="flex-1 min-w-0">
                      <div className="flex items-center gap-2">
                        <span className="text-white text-sm font-medium truncate">
                          {getLocaleDisplayName(locale)}
                        </span>
                        {locale === currentLocale && (
                          <Check className="w-4 h-4 text-green-400 flex-shrink-0" />
                        )}
                      </div>
                      {locale === config.default_language && (
                        <p className="text-xs text-gray-400">Default language</p>
                      )}
                    </div>
                  </button>
                ))}
              </div>
            ) : (
              <div className="p-8 text-center text-gray-400">
                {searchTerm ? 'No languages match your search' : 'No languages configured'}
              </div>
            )}
          </div>

          {/* Footer info */}
          {config.supported_languages.length > 0 && (
            <div className="px-4 py-2 bg-primary-500/10 border-t border-primary-500/20 text-xs text-primary-300 flex items-center gap-2">
              <Languages className="w-3 h-3" />
              {config.supported_languages.length} language{config.supported_languages.length !== 1 ? 's' : ''} available
            </div>
          )}
        </div>,
        document.body
      )}
    </div>
  )
}
