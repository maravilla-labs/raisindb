/**
 * Element Type Builder Preferences Hook
 *
 * Manages and persists builder preferences to localStorage.
 */

import { useState, useCallback, useEffect } from 'react'

const STORAGE_KEY = 'elementtype-builder-preferences'

export interface ElementTypeBuilderPreferences {
  toolboxWidth: number
  propertiesWidth: number
}

const DEFAULT_PREFERENCES: ElementTypeBuilderPreferences = {
  toolboxWidth: 160,
  propertiesWidth: 320,
}

function loadPreferences(): ElementTypeBuilderPreferences {
  try {
    const stored = localStorage.getItem(STORAGE_KEY)
    if (stored) {
      return { ...DEFAULT_PREFERENCES, ...JSON.parse(stored) }
    }
  } catch (e) {
    console.warn('Failed to load element type builder preferences:', e)
  }
  return DEFAULT_PREFERENCES
}

function savePreferences(prefs: ElementTypeBuilderPreferences): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(prefs))
  } catch (e) {
    console.warn('Failed to save element type builder preferences:', e)
  }
}

export function useElementTypeBuilderPreferences() {
  const [preferences, setPreferencesState] = useState<ElementTypeBuilderPreferences>(loadPreferences)

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
