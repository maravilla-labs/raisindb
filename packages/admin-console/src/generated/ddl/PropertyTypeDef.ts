import type { PropertyDef } from './PropertyDef';

export type PropertyTypeDef = "String" | "Number" | "Boolean" | "Date" | "URL" | "Reference" | "Resource" | { "Object": { 
/**
 * Nested field definitions
 */
fields: Array<PropertyDef>, } } | { "Array": { 
/**
 * Type of array items
 */
items: PropertyTypeDef, } } | "Composite" | "Element" | "NodeType";