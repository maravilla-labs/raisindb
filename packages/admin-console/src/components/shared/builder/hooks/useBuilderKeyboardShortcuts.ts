/**
 * useBuilderKeyboardShortcuts Hook
 *
 * Handles keyboard shortcuts for undo/redo and other builder operations.
 */

import { useEffect, useCallback } from 'react'

export interface UseBuilderKeyboardShortcutsOptions {
  /** Callback for undo (Ctrl/Cmd+Z) */
  onUndo?: () => void
  /** Callback for redo (Ctrl/Cmd+Shift+Z or Ctrl/Cmd+Y) */
  onRedo?: () => void
  /** Callback for delete (Delete or Backspace) */
  onDelete?: () => void
  /** Callback for escape (clear selection) */
  onEscape?: () => void
  /** Whether shortcuts are disabled */
  disabled?: boolean
}

export function useBuilderKeyboardShortcuts({
  onUndo,
  onRedo,
  onDelete,
  onEscape,
  disabled = false,
}: UseBuilderKeyboardShortcutsOptions) {
  const handleKeyDown = useCallback(
    (e: KeyboardEvent) => {
      if (disabled) return

      // Don't handle shortcuts when typing in inputs
      const target = e.target as HTMLElement
      if (
        target.tagName === 'INPUT' ||
        target.tagName === 'TEXTAREA' ||
        target.isContentEditable
      ) {
        return
      }

      const isMac = navigator.platform.toUpperCase().indexOf('MAC') >= 0
      const modifier = isMac ? e.metaKey : e.ctrlKey

      // Undo: Ctrl/Cmd + Z
      if (modifier && e.key === 'z' && !e.shiftKey) {
        e.preventDefault()
        onUndo?.()
        return
      }

      // Redo: Ctrl/Cmd + Shift + Z or Ctrl/Cmd + Y
      if (
        (modifier && e.key === 'z' && e.shiftKey) ||
        (modifier && e.key === 'y')
      ) {
        e.preventDefault()
        onRedo?.()
        return
      }

      // Delete: Delete or Backspace
      if (e.key === 'Delete' || e.key === 'Backspace') {
        e.preventDefault()
        onDelete?.()
        return
      }

      // Escape: clear selection
      if (e.key === 'Escape') {
        e.preventDefault()
        onEscape?.()
        return
      }
    },
    [disabled, onUndo, onRedo, onDelete, onEscape]
  )

  useEffect(() => {
    window.addEventListener('keydown', handleKeyDown)
    return () => {
      window.removeEventListener('keydown', handleKeyDown)
    }
  }, [handleKeyDown])
}
