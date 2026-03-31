/**
 * Archetype Builder Context
 *
 * Provides state management with command history (undo/redo),
 * selection state, and preferences for the archetype builder.
 */

import {
  createContext,
  useContext,
  useState,
  useCallback,
  useEffect,
  useRef,
  type ReactNode,
} from 'react'
import { useBuilderCommandHistory, UpdateStateCommand, type CommandContext } from '../shared/builder'
import { useArchetypeBuilderPreferences, type ArchetypeBuilderPreferences } from './useArchetypeBuilderPreferences'
import type { ArchetypeDefinition } from './types'

interface ArchetypeBuilderContextValue {
  // State
  archetype: ArchetypeDefinition
  setArchetype: (newArchetype: ArchetypeDefinition, description?: string) => void

  // Selection
  selectedPath: string | undefined
  setSelectedPath: (path: string | undefined) => void

  // Command history
  undo: () => void
  redo: () => void
  canUndo: boolean
  canRedo: boolean

  // Preferences
  preferences: ArchetypeBuilderPreferences
  setToolboxWidth: (w: number) => void
  setPropertiesWidth: (w: number) => void
}

const ArchetypeBuilderContext = createContext<ArchetypeBuilderContextValue | null>(null)

interface ArchetypeBuilderProviderProps {
  children: ReactNode
  initialArchetype: ArchetypeDefinition
  onChange: (archetype: ArchetypeDefinition) => void
}

export function ArchetypeBuilderProvider({
  children,
  initialArchetype,
  onChange,
}: ArchetypeBuilderProviderProps) {
  // Internal state - the source of truth
  const [archetype, setArchetypeState] = useState<ArchetypeDefinition>(initialArchetype)
  const [selectedPath, setSelectedPath] = useState<string | undefined>()

  // Command history
  const { history, historyState, reset } = useBuilderCommandHistory<ArchetypeDefinition>()

  // Preferences
  const { preferences, setToolboxWidth, setPropertiesWidth } = useArchetypeBuilderPreferences()

  // Command context ref - updated on each render
  const contextRef = useRef<CommandContext<ArchetypeDefinition>>({
    getState: () => archetype,
    setState: (updater) => {
      const newState = updater(archetype)
      setArchetypeState(newState)
      onChange(newState)
    },
  })

  // Keep context ref up to date
  contextRef.current = {
    getState: () => archetype,
    setState: (updater) => {
      const newState = updater(archetype)
      setArchetypeState(newState)
      onChange(newState)
    },
  }

  // Reset history when initial archetype changes (e.g., loaded from server)
  useEffect(() => {
    setArchetypeState(initialArchetype)
    reset()
  }, [initialArchetype, reset])

  // Set archetype with command history
  const setArchetype = useCallback((newArchetype: ArchetypeDefinition, description?: string) => {
    const command = new UpdateStateCommand(
      contextRef.current,
      newArchetype,
      description || 'Update archetype'
    )
    history.execute(command)
  }, [history])

  // Undo/Redo
  const undo = useCallback(() => {
    history.undo()
  }, [history])

  const redo = useCallback(() => {
    history.redo()
  }, [history])

  // Keyboard shortcuts
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      // Ignore if in an input/textarea
      if (
        e.target instanceof HTMLInputElement ||
        e.target instanceof HTMLTextAreaElement
      ) {
        return
      }

      if ((e.metaKey || e.ctrlKey) && e.key === 'z') {
        e.preventDefault()
        if (e.shiftKey) {
          redo()
        } else {
          undo()
        }
      }
    }

    window.addEventListener('keydown', handleKeyDown)
    return () => window.removeEventListener('keydown', handleKeyDown)
  }, [undo, redo])

  const value: ArchetypeBuilderContextValue = {
    archetype,
    setArchetype,
    selectedPath,
    setSelectedPath,
    undo,
    redo,
    canUndo: historyState.canUndo,
    canRedo: historyState.canRedo,
    preferences,
    setToolboxWidth,
    setPropertiesWidth,
  }

  return (
    <ArchetypeBuilderContext.Provider value={value}>
      {children}
    </ArchetypeBuilderContext.Provider>
  )
}

export function useArchetypeBuilderContext() {
  const context = useContext(ArchetypeBuilderContext)
  if (!context) {
    throw new Error('useArchetypeBuilderContext must be used within ArchetypeBuilderProvider')
  }
  return context
}
