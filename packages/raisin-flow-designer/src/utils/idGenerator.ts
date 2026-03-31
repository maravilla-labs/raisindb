/**
 * ID Generator
 *
 * Wrapper around nanoid for generating unique IDs.
 */

import { nanoid } from 'nanoid';

/** Length of generated IDs */
const ID_LENGTH = 12;

/**
 * Generate a unique ID for flow nodes
 */
export function generateNodeId(): string {
  return `node_${nanoid(ID_LENGTH)}`;
}

/**
 * Generate a unique ID for steps
 */
export function generateStepId(): string {
  return `step_${nanoid(ID_LENGTH)}`;
}

/**
 * Generate a unique ID for containers
 */
export function generateContainerId(): string {
  return `container_${nanoid(ID_LENGTH)}`;
}

/**
 * Generate a unique ID with custom prefix
 */
export function generateId(prefix: string = ''): string {
  const id = nanoid(ID_LENGTH);
  return prefix ? `${prefix}_${id}` : id;
}
