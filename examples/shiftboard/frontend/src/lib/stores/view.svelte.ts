/**
 * Which side-column view is active. The board stays visible either way —
 * the Planner tab swaps only the right column (tasks+planner chat → plan
 * panel + coordinator chat), so the shifts filling live remain on screen.
 */
export type Tab = 'board' | 'planner';

class ViewState {
  tab = $state<Tab>('board');
}

export const view = new ViewState();
