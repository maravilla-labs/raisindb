/**
 * Event type constants for subscribing to node events
 *
 * @example
 * ```typescript
 * import { RaisinClient, NodeEventType } from '@raisindb/client';
 *
 * client.events.subscribeToTypes([NodeEventType.Created, NodeEventType.Updated], (event) => {
 *   console.log('Node changed:', event);
 * });
 * ```
 */
export const NodeEventType = {
  /** Node was created */
  Created: 'node:created',
  /** Node was updated */
  Updated: 'node:updated',
  /** Node was deleted */
  Deleted: 'node:deleted',
  /** Node was reordered among siblings */
  Reordered: 'node:reordered',
  /** Node was published */
  Published: 'node:published',
  /** Node was unpublished */
  Unpublished: 'node:unpublished',
  /** A single property was changed */
  PropertyChanged: 'node:property_changed',
  /** A relation was added to the node */
  RelationAdded: 'node:relation_added',
  /** A relation was removed from the node */
  RelationRemoved: 'node:relation_removed',
} as const;

/** Type for node event type values */
export type NodeEventTypeValue = (typeof NodeEventType)[keyof typeof NodeEventType];

/**
 * All node event types as an array (useful for subscribing to all events)
 *
 * @example
 * ```typescript
 * client.events.subscribeToTypes(AllNodeEventTypes, (event) => {
 *   console.log('Any node event:', event);
 * });
 * ```
 */
export const AllNodeEventTypes: NodeEventTypeValue[] = Object.values(NodeEventType);
