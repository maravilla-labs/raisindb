/**
 * Archetype Builder Preferences Hook
 *
 * Manages and persists builder preferences to localStorage.
 */

import { useState, useCallback, useEffect } from 'react'

const STORAGE_KEY = 'archetype-builder-preferences'

export interface ArchetypeBuilderPreferences {
  toolboxWidth: number
  propertiesWidth: number
}

const DEFAULT_PREFERENCES: ArchetypeBuilderPreferences = {
  toolboxWidth: 160,
  propertiesWidth: 320,
}

function loadPreferences(): ArchetypeBuilderPreferences {
  try {
    const stored = localStorage.getItem(STORAGE_KEY)
    if (stored) {
      return { ...DEFAULT_PREFERENCES, ...JSON.parse(stored) }
    }
  } catch (e) {
    console.warn('Failed to load archetype builder preferences:', e)
  }
  return DEFAULT_PREFERENCES
}

function savePreferences(prefs: ArchetypeBuilderPreferences): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(prefs))
  } catch (e) {
    console.warn('Failed to save archetype builder preferences:', e)
  }
}

export function useArchetypeBuilderPreferences() {
  const [preferences, setPreferencesState] = useState<ArchetypeBuilderPreferences>(loadPreferences)

  // Persist on change
  useEffect(() => {
    savePreferences(preferences)
  }, [preferences])

  const setToolboxWidth = useCallback((width: number) => {
    setPreferencesState((prev) => ({ ...prev, toolboxWidth: width }))
  }, [])

  const setPropertiesWidth = useCallback((width: number) => {
    setPreferencesState((prev) => ({ ...prev, propertiesWidth: width }))
  }, [])

  return {
    preferences,
    setToolboxWidth,
    setPropertiesWidth,
  }
}
