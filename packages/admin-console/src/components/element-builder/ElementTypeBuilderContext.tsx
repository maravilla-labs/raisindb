/**
 * ElementType Builder Context
 *
 * Provides state management with command history (undo/redo),
 * selection state, and preferences for the element type builder.
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
import { useElementTypeBuilderPreferences, type ElementTypeBuilderPreferences } from './useElementTypeBuilderPreferences'
import type { ElementTypeDefinition } from './types'

interface ElementTypeBuilderContextValue {
  // State
  elementType: ElementTypeDefinition
  setElementType: (newElementType: ElementTypeDefinition, description?: string) => void

  // Selection
  selectedPath: string | undefined
  setSelectedPath: (path: string | undefined) => void

  // Command history
  undo: () => void
  redo: () => void
  canUndo: boolean
  canRedo: boolean

  // Preferences
  preferences: ElementTypeBuilderPreferences
  setToolboxWidth: (w: number) => void
  setPropertiesWidth: (w: number) => void
}

const ElementTypeBuilderContext = createContext<ElementTypeBuilderContextValue | null>(null)

interface ElementTypeBuilderProviderProps {
  children: ReactNode
  initialElementType: ElementTypeDefinition
  onChange: (elementType: ElementTypeDefinition) => void
}

export function ElementTypeBuilderProvider({
  children,
  initialElementType,
  onChange,
}: ElementTypeBuilderProviderProps) {
  // Internal state - the source of truth
  const [elementType, setElementTypeState] = useState<ElementTypeDefinition>(initialElementType)
  const [selectedPath, setSelectedPath] = useState<string | undefined>()

  // Command history
  const { history, historyState, reset } = useBuilderCommandHistory<ElementTypeDefinition>()

  // Preferences
  const { preferences, setToolboxWidth, setPropertiesWidth } = useElementTypeBuilderPreferences()

  // Command context ref - updated on each render
  const contextRef = useRef<CommandContext<ElementTypeDefinition>>({
    getState: () => elementType,
    setState: (updater) => {
      const newState = updater(elementType)
      setElementTypeState(newState)
      onChange(newState)
    },
  })

  // Keep context ref up to date
  contextRef.current = {
    getState: () => elementType,
    setState: (updater) => {
      const newState = updater(elementType)
      setElementTypeState(newState)
      onChange(newState)
    },
  }

  // Reset history when initial elementType changes (e.g., loaded from server)
  useEffect(() => {
    setElementTypeState(initialElementType)
    reset()
  }, [initialElementType, reset])

  // Set elementType with command history
  const setElementType = useCallback((newElementType: ElementTypeDefinition, description?: string) => {
    const command = new UpdateStateCommand(
      contextRef.current,
      newElementType,
      description || 'Update element type'
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

  const value: ElementTypeBuilderContextValue = {
    elementType,
    setElementType,
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
    <ElementTypeBuilderContext.Provider value={value}>
      {children}
    </ElementTypeBuilderContext.Provider>
  )
}

export function useElementTypeBuilderContext() {
  const context = useContext(ElementTypeBuilderContext)
  if (!context) {
    throw new Error('useElementTypeBuilderContext must be used within ElementTypeBuilderProvider')
  }
  return context
}
