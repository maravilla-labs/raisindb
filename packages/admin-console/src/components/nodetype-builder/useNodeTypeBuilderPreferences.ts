/**
 * NodeType Builder Preferences Hook
 *
 * Manages and persists builder preferences to localStorage.
 */

import { useState, useCallback, useEffect } from 'react'

const STORAGE_KEY = 'nodetype-builder-preferences'

export interface NodeTypeBuilderPreferences {
  toolboxWidth: number
  propertiesWidth: number
}

const DEFAULT_PREFERENCES: NodeTypeBuilderPreferences = {
  toolboxWidth: 160,
  propertiesWidth: 320,
}

function loadPreferences(): NodeTypeBuilderPreferences {
  try {
    const stored = localStorage.getItem(STORAGE_KEY)
    if (stored) {
      return { ...DEFAULT_PREFERENCES, ...JSON.parse(stored) }
    }
  } catch (e) {
    console.warn('Failed to load nodetype builder preferences:', e)
  }
  return DEFAULT_PREFERENCES
}

function savePreferences(prefs: NodeTypeBuilderPreferences): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(prefs))
  } catch (e) {
    console.warn('Failed to save nodetype builder preferences:', e)
  }
}

export function useNodeTypeBuilderPreferences() {
  const [preferences, setPreferencesState] = useState<NodeTypeBuilderPreferences>(loadPreferences)

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
