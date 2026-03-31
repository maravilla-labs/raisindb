import type { Component } from 'svelte';
import Hero from './Hero.svelte';
import TextBlock from './TextBlock.svelte';
import FeatureGrid from './FeatureGrid.svelte';
import ListKanbanBoards from './ListKanbanBoards.svelte';

// Element component props type
interface ElementProps {
  content: Record<string, unknown>;
}

/**
 * Mapping: ElementType name → Svelte component
 */
// eslint-disable-next-line @typescript-eslint/no-explicit-any
export const elementComponents: Record<string, Component<any>> = {
  'launchpad:Hero': Hero,
  'launchpad:TextBlock': TextBlock,
  'launchpad:FeatureGrid': FeatureGrid,
  'launchpad:ListKanbanBoards': ListKanbanBoards
};

/**
 * Get element component for an element type
 */
export function getElementComponent(elementType: string): Component<ElementProps> | undefined {
  return elementComponents[elementType];
}
