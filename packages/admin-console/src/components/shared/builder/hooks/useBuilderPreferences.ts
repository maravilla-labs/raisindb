/**
 * useBuilderPreferences Hook
 *
 * Generic preferences hook for visual builders with localStorage persistence.
 */

import { useState, useEffect, useCallback } from 'react'
import {
  type BuilderLayoutPreferences,
  DEFAULT_BUILDER_PREFERENCES,
} from '../layout'

export interface UseBuilderPreferencesOptions {
  /** Storage key for localStorage */
  storageKey: string
  /** Default preferences (merged with DEFAULT_BUILDER_PREFERENCES) */
  defaults?: Partial<BuilderLayoutPreferences>
}

export interface UseBuilderPreferencesReturn {
  preferences: BuilderLayoutPreferences
  setToolboxWidth: (width: number) => void
  setPropertiesWidth: (width: number) => void
  toggleToolbox: () => void
  toggleProperties: () => void
  resetPreferences: () => void
}

export function useBuilderPreferences({
  storageKey,
  defaults = {},
}: UseBuilderPreferencesOptions): UseBuilderPreferencesReturn {
  const defaultPreferences: BuilderLayoutPreferences = {
    ...DEFAULT_BUILDER_PREFERENCES,
    ...defaults,
  }

  // Load initial preferences from localStorage
  const [preferences, setPreferences] = useState<BuilderLayoutPreferences>(
    () => {
      try {
        const stored = localStorage.getItem(storageKey)
        if (stored) {
          return { ...defaultPreferences, ...JSON.parse(stored) }
        }
      } catch {
        // Ignore parse errors
      }
      return defaultPreferences
    }
  )

  // Persist preferences to localStorage
  useEffect(() => {
    try {
      localStorage.setItem(storageKey, JSON.stringify(preferences))
    } catch {
      // Ignore storage errors (quota, etc.)
    }
  }, [storageKey, preferences])

  const setToolboxWidth = useCallback((width: number) => {
    setPreferences((prev) => ({ ...prev, toolboxWidth: width }))
  }, [])

  const setPropertiesWidth = useCallback((width: number) => {
    setPreferences((prev) => ({ ...prev, propertiesWidth: width }))
  }, [])

  const toggleToolbox = useCallback(() => {
    setPreferences((prev) => ({ ...prev, toolboxVisible: !prev.toolboxVisible }))
  }, [])

  const toggleProperties = useCallback(() => {
    setPreferences((prev) => ({
      ...prev,
      propertiesVisible: !prev.propertiesVisible,
    }))
  }, [])

  const resetPreferences = useCallback(() => {
    setPreferences(defaultPreferences)
  }, [defaultPreferences])

  return {
    preferences,
    setToolboxWidth,
    setPropertiesWidth,
    toggleToolbox,
    toggleProperties,
    resetPreferences,
  }
}
