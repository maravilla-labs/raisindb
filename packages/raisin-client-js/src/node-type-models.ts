/**
 * NodeType models matching the Rust server structures
 */

/**
 * Property types supported by RaisinDB
 * Note: These must match the Rust PropertyType enum (capitalized)
 */
export type PropertyType =
  | 'String'
  | 'Number'
  | 'Boolean'
  | 'Date'
  | 'Object'
  | 'Array'
  | 'URL'
  | 'Reference'
  | 'NodeType'
  | 'Element'
  | 'Composite'
  | 'Resource';

/**
 * Property value schema definition
 */
export interface PropertyValueSchema {
  /** Property name */
  name: string;
  /** Property type */
  type: PropertyType;
  /** Whether this property is required */
  required?: boolean;
  /** Whether this property must be unique */
  unique?: boolean;
  /** Default value for this property */
  default?: unknown;
  /** Constraints for this property (e.g., minLength, maxLength, min, max) */
  constraints?: Record<string, unknown>;
  /** Structure for object types */
  structure?: Record<string, PropertyValueSchema>;
  /** Schema for array items */
  items?: PropertyValueSchema;
  /** Static value */
  value?: unknown;
  /** Whether this property is translatable */
  is_translatable?: boolean;
  /** Allow additional properties (for objects) */
  allow_additional_properties?: boolean;
  /** Index types for this property */
  index_types?: string[];
}

/**
 * NodeType definition
 */
export interface NodeTypeDefinition {
  /** Unique identifier (auto-generated if not provided) */
  id?: string;
  /** Strict mode (no additional properties allowed) */
  strict?: boolean;
  /** NodeType name */
  name: string;
  /** Parent NodeType to extend from */
  extends?: string;
  /** Mixins to include */
  mixins?: string[];
  /** Property overrides */
  overrides?: Record<string, unknown>;
  /** Description of this NodeType */
  description?: string;
  /** Icon for this NodeType */
  icon?: string;
  /** Version number */
  version?: number;
  /** Property schemas (MUST be an array, not an object) */
  properties?: PropertyValueSchema[];
  /** Allowed child NodeTypes */
  allowed_children?: string[];
  /** Required child nodes */
  required_nodes?: string[];
  /** Initial structure for new nodes */
  initial_structure?: unknown;
  /** Whether nodes of this type are versionable */
  versionable?: boolean;
  /** Whether nodes of this type are publishable */
  publishable?: boolean;
  /** Whether nodes of this type are auditable */
  auditable?: boolean;
  /** Whether nodes of this type are indexable */
  indexable?: boolean;
  /** Which index types are enabled */
  index_types?: string[];
}
