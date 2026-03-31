/**
 * NodeType Builder Context
 *
 * Provides state management with command history (undo/redo),
 * selection state, and preferences for the nodetype builder.
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
import { useNodeTypeBuilderPreferences, type NodeTypeBuilderPreferences } from './useNodeTypeBuilderPreferences'
import type { NodeTypeDefinition } from './types'

interface NodeTypeBuilderContextValue {
  // State
  nodeType: NodeTypeDefinition
  setNodeType: (newNodeType: NodeTypeDefinition, description?: string) => void

  // Selection
  selectedPath: string | undefined
  setSelectedPath: (path: string | undefined) => void

  // Command history
  undo: () => void
  redo: () => void
  canUndo: boolean
  canRedo: boolean

  // Preferences
  preferences: NodeTypeBuilderPreferences
  setToolboxWidth: (w: number) => void
  setPropertiesWidth: (w: number) => void
}

const NodeTypeBuilderContext = createContext<NodeTypeBuilderContextValue | null>(null)

interface NodeTypeBuilderProviderProps {
  children: ReactNode
  initialNodeType: NodeTypeDefinition
  onChange: (nodeType: NodeTypeDefinition) => void
}

export function NodeTypeBuilderProvider({
  children,
  initialNodeType,
  onChange,
}: NodeTypeBuilderProviderProps) {
  // Internal state - the source of truth
  const [nodeType, setNodeTypeState] = useState<NodeTypeDefinition>(initialNodeType)
  const [selectedPath, setSelectedPath] = useState<string | undefined>()

  // Command history
  const { history, historyState, reset } = useBuilderCommandHistory<NodeTypeDefinition>()

  // Preferences
  const { preferences, setToolboxWidth, setPropertiesWidth } = useNodeTypeBuilderPreferences()

  // Command context ref - updated on each render
  const contextRef = useRef<CommandContext<NodeTypeDefinition>>({
    getState: () => nodeType,
    setState: (updater) => {
      const newState = updater(nodeType)
      setNodeTypeState(newState)
      onChange(newState)
    },
  })

  // Keep context ref up to date
  contextRef.current = {
    getState: () => nodeType,
    setState: (updater) => {
      const newState = updater(nodeType)
      setNodeTypeState(newState)
      onChange(newState)
    },
  }

  // Reset history when initial nodeType changes (e.g., loaded from server)
  useEffect(() => {
    setNodeTypeState(initialNodeType)
    reset()
  }, [initialNodeType, reset])

  // Set nodeType with command history
  const setNodeType = useCallback((newNodeType: NodeTypeDefinition, description?: string) => {
    const command = new UpdateStateCommand(
      contextRef.current,
      newNodeType,
      description || 'Update node type'
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

  const value: NodeTypeBuilderContextValue = {
    nodeType,
    setNodeType,
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
    <NodeTypeBuilderContext.Provider value={value}>
      {children}
    </NodeTypeBuilderContext.Provider>
  )
}

export function useNodeTypeBuilderContext() {
  const context = useContext(NodeTypeBuilderContext)
  if (!context) {
    throw new Error('useNodeTypeBuilderContext must be used within NodeTypeBuilderProvider')
  }
  return context
}
