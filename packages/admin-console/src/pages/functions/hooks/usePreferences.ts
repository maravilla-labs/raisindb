/**
 * localStorage preferences hook for Functions IDE
 */

import { useState, useCallback, useEffect } from 'react'
import type { FunctionsPreferences } from '../types'

const STORAGE_KEY = 'raisindb-functions-ide-preferences'

const DEFAULT_PREFERENCES: FunctionsPreferences = {
  sidebarWidth: 280,
  propertiesWidth: 300,
  outputHeight: 200,
  sidebarVisible: true,
  propertiesVisible: true,
  outputVisible: true,
  fontSize: 14,
  lastOpenTabs: [],
  expandedFolders: [],
}

/**
 * Hook for managing Functions IDE preferences in localStorage
 */
export function usePreferences() {
  const [preferences, setPreferencesState] = useState<FunctionsPreferences>(() => {
    try {
      const stored = localStorage.getItem(STORAGE_KEY)
      if (stored) {
        return { ...DEFAULT_PREFERENCES, ...JSON.parse(stored) }
      }
    } catch (e) {
      console.error('Failed to load preferences:', e)
    }
    return DEFAULT_PREFERENCES
  })

  // Save to localStorage whenever preferences change
  useEffect(() => {
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(preferences))
    } catch (e) {
      console.error('Failed to save preferences:', e)
    }
  }, [preferences])

  const setPreferences = useCallback((update: Partial<FunctionsPreferences>) => {
    setPreferencesState((prev) => ({ ...prev, ...update }))
  }, [])

  const setSidebarWidth = useCallback((width: number) => {
    setPreferencesState((prev) => ({ ...prev, sidebarWidth: width }))
  }, [])

  const setPropertiesWidth = useCallback((width: number) => {
    setPreferencesState((prev) => ({ ...prev, propertiesWidth: width }))
  }, [])

  const setOutputHeight = useCallback((height: number) => {
    setPreferencesState((prev) => ({ ...prev, outputHeight: height }))
  }, [])

  const toggleSidebar = useCallback(() => {
    setPreferencesState((prev) => ({ ...prev, sidebarVisible: !prev.sidebarVisible }))
  }, [])

  const toggleProperties = useCallback(() => {
    setPreferencesState((prev) => ({ ...prev, propertiesVisible: !prev.propertiesVisible }))
  }, [])

  const toggleOutput = useCallback(() => {
    setPreferencesState((prev) => ({ ...prev, outputVisible: !prev.outputVisible }))
  }, [])

  const setFontSize = useCallback((size: number) => {
    setPreferencesState((prev) => ({ ...prev, fontSize: size }))
  }, [])

  const addOpenTab = useCallback((tabPath: string) => {
    setPreferencesState((prev) => {
      if (prev.lastOpenTabs.includes(tabPath)) {
        return prev
      }
      return { ...prev, lastOpenTabs: [...prev.lastOpenTabs, tabPath] }
    })
  }, [])

  const removeOpenTab = useCallback((tabPath: string) => {
    setPreferencesState((prev) => ({
      ...prev,
      lastOpenTabs: prev.lastOpenTabs.filter((p) => p !== tabPath),
    }))
  }, [])

  const setExpandedFolders = useCallback((folders: string[]) => {
    setPreferencesState((prev) => ({ ...prev, expandedFolders: folders }))
  }, [])

  const toggleFolder = useCallback((path: string) => {
    setPreferencesState((prev) => {
      const expanded = new Set(prev.expandedFolders)
      if (expanded.has(path)) {
        expanded.delete(path)
      } else {
        expanded.add(path)
      }
      return { ...prev, expandedFolders: Array.from(expanded) }
    })
  }, [])

  const resetPreferences = useCallback(() => {
    setPreferencesState(DEFAULT_PREFERENCES)
  }, [])

  return {
    preferences,
    setPreferences,
    setSidebarWidth,
    setPropertiesWidth,
    setOutputHeight,
    toggleSidebar,
    toggleProperties,
    toggleOutput,
    setFontSize,
    addOpenTab,
    removeOpenTab,
    setExpandedFolders,
    toggleFolder,
    resetPreferences,
  }
}

export type UsePreferencesReturn = ReturnType<typeof usePreferences>
