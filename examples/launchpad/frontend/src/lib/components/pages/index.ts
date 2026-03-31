import type { Component } from 'svelte';
import LandingPage from './LandingPage.svelte';
import KanbanBoardPage from './KanbanBoardPage.svelte';
import FileBrowserPage from './FileBrowserPage.svelte';
import type { PageNode } from '$lib/raisin';

/**
 * Mapping: Archetype name → Page layout component
 */
// eslint-disable-next-line @typescript-eslint/no-explicit-any
export const pageComponents: Record<string, Component<any>> = {
  'launchpad:LandingPage': LandingPage,
  'launchpad:KanbanBoard': KanbanBoardPage,
  'launchpad:FileBrowser': FileBrowserPage
};

/**
 * Get page component for an archetype
 */
export function getPageComponent(archetype: string | undefined): Component<{ page: PageNode }> | undefined {
  if (!archetype) return undefined;
  return pageComponents[archetype];
}
