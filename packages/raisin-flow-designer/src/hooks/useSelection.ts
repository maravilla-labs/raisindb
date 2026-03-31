/**
 * Selection Hook
 *
 * Manages single or multi-selection of flow nodes.
 */

import { useState, useCallback } from 'react';

export interface UseSelectionOptions {
  initialSelection?: string[];
  onSelectionChange?: (selection: string[]) => void;
  multiSelect?: boolean;
}

export interface UseSelectionReturn {
  selection: string[];
  selectedId: string | null;
  isSelected: (nodeId: string) => boolean;
  select: (nodeId: string, addToSelection?: boolean) => void;
  deselect: (nodeId: string) => void;
  toggleSelect: (nodeId: string, addToSelection?: boolean) => void;
  clearSelection: () => void;
  selectAll: (nodeIds: string[]) => void;
}

export function useSelection(
  options: UseSelectionOptions = {}
): UseSelectionReturn {
  const {
    initialSelection = [],
    onSelectionChange,
    multiSelect = false,
  } = options;

  const [selection, setSelectionInternal] = useState<string[]>(initialSelection);

  /**
   * Update selection with callback
   */
  const setSelection = useCallback(
    (newSelection: string[]) => {
      setSelectionInternal(newSelection);
      onSelectionChange?.(newSelection);
    },
    [onSelectionChange]
  );

  /**
   * Check if a node is selected
   */
  const isSelected = useCallback(
    (nodeId: string) => selection.includes(nodeId),
    [selection]
  );

  /**
   * Select a node
   */
  const select = useCallback(
    (nodeId: string, addToSelection = false) => {
      if (multiSelect && addToSelection) {
        if (!selection.includes(nodeId)) {
          setSelection([...selection, nodeId]);
        }
      } else {
        setSelection([nodeId]);
      }
    },
    [multiSelect, selection, setSelection]
  );

  /**
   * Deselect a node
   */
  const deselect = useCallback(
    (nodeId: string) => {
      setSelection(selection.filter((id) => id !== nodeId));
    },
    [selection, setSelection]
  );

  /**
   * Toggle node selection
   */
  const toggleSelect = useCallback(
    (nodeId: string, addToSelection = false) => {
      if (selection.includes(nodeId)) {
        deselect(nodeId);
      } else {
        select(nodeId, addToSelection);
      }
    },
    [selection, select, deselect]
  );

  /**
   * Clear all selections
   */
  const clearSelection = useCallback(() => {
    setSelection([]);
  }, [setSelection]);

  /**
   * Select all specified nodes
   */
  const selectAll = useCallback(
    (nodeIds: string[]) => {
      if (multiSelect) {
        setSelection(nodeIds);
      } else if (nodeIds.length > 0) {
        setSelection([nodeIds[0]]);
      }
    },
    [multiSelect, setSelection]
  );

  return {
    selection,
    selectedId: selection.length > 0 ? selection[0] : null,
    isSelected,
    select,
    deselect,
    toggleSelect,
    clearSelection,
    selectAll,
  };
}
