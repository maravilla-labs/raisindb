/**
 * Actions store for cross-component communication
 *
 * Elements can trigger actions (e.g., Hero with cta_action="createBoard")
 * and other components can listen and respond to these actions.
 */
import { writable } from 'svelte/store';

// Available actions
export type ActionType = 'createBoard' | null;

// Current triggered action
export const currentAction = writable<ActionType>(null);

/**
 * Trigger an action by name
 */
export function triggerAction(action: string) {
  currentAction.set(action as ActionType);
}

/**
 * Clear the current action (after handling)
 */
export function clearAction() {
  currentAction.set(null);
}
