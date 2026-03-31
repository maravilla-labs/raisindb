import { writable, derived } from 'svelte/store';
import type { NavItem } from '$lib/raisin';

// Navigation items store
export const navigationItems = writable<NavItem[]>([]);

// Loading state
export const isLoading = writable(true);

// Current page path (URL path)
export const currentPath = writable('/home');

// Current node path (full database path like /launchpad/boards/my-board)
export const currentNodePath = writable<string | null>(null);

// Current page node info for AI context
export interface CurrentPageContext {
  nodePath: string;
  nodeType?: string;
  archetype?: string;
  title?: string;
}
export const currentPageContext = writable<CurrentPageContext | null>(null);

// Derived: current navigation item
export const currentNavItem = derived(
  [navigationItems, currentPath],
  ([$items, $path]) => {
    // Match by slug or by path (path from SQL doesn't have leading slash for root children)
    return $items.find(item => {
      const slug = item.properties.slug || item.name;
      return `/${slug}` === $path || `/${item.path}` === $path;
    });
  }
);

// Set navigation items
export function setNavigation(items: NavItem[]) {
  navigationItems.set(items);
  isLoading.set(false);
}

// Set current path
export function setCurrentPath(path: string) {
  currentPath.set(path);
}

// Set current node path (full database path)
export function setCurrentNodePath(nodePath: string | null) {
  currentNodePath.set(nodePath);
}

// Set current page context for AI
export function setCurrentPageContext(context: CurrentPageContext | null) {
  currentPageContext.set(context);
  if (context) {
    currentNodePath.set(context.nodePath);
  } else {
    currentNodePath.set(null);
  }
}
