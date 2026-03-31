import type { IndexTypeDef } from './IndexTypeDef';
import type { DefaultValue } from './DefaultValue';
import type { PropertyTypeDef } from './PropertyTypeDef';

export type PropertyDef = { 
/**
 * Property name (snake_case identifier) or nested path (e.g., "specs.dimensions.width")
 */
name: string, 
/**
 * Property type
 */
property_type: PropertyTypeDef, 
/**
 * Whether this property is required
 */
required: boolean, 
/**
 * Whether values must be unique across nodes
 */
unique: boolean, 
/**
 * Index types for this property
 */
index: Array<IndexTypeDef>, 
/**
 * Default value
 */
default: DefaultValue | null, 
/**
 * Whether this property is translatable (i18n)
 */
translatable: boolean, 
/**
 * Constraints (min, max, pattern, etc.)
 */
constraints: any, 
/**
 * Human-readable label (stored in meta.label)
 */
label: string | null, 
/**
 * Human-readable description (stored in meta.description)
 */
description: string | null, 
/**
 * Display order hint (stored in meta.order)
 */
order: number | null, 
/**
 * For Object types: allow properties not in schema
 */
allow_additional_properties: boolean, };