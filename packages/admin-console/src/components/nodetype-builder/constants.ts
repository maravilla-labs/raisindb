import {
  Type,
  Hash,
  ToggleLeft,
  List,
  Braces,
  Calendar,
  Link,
  ArrowRight,
  FileType,
  Box,
  Layers,
  File
} from 'lucide-react'
import type { PropertyType } from './types'

export const PROPERTY_TYPE_ICONS: Record<PropertyType, any> = {
  String: Type,
  Number: Hash,
  Boolean: ToggleLeft,
  Array: List,
  Object: Braces,
  Date: Calendar,
  URL: Link,
  Reference: ArrowRight,
  NodeType: FileType,
  Element: Box,
  Composite: Layers,
  Resource: File,
}

export const PROPERTY_TYPE_LABELS: Record<PropertyType, string> = {
  String: 'String',
  Number: 'Number',
  Boolean: 'Boolean',
  Array: 'Array',
  Object: 'Object',
  Date: 'Date',
  URL: 'URL',
  Reference: 'Reference',
  NodeType: 'Node Type',
  Element: 'Element',
  Composite: 'Composite',
  Resource: 'Resource',
}

export const PROPERTY_TYPE_COLORS: Record<PropertyType, string> = {
  String: 'text-blue-400 bg-blue-500/20 border-blue-400/50',
  Number: 'text-green-400 bg-green-500/20 border-green-400/50',
  Boolean: 'text-purple-400 bg-purple-500/20 border-purple-400/50',
  Array: 'text-orange-400 bg-orange-500/20 border-orange-400/50',
  Object: 'text-pink-400 bg-pink-500/20 border-pink-400/50',
  Date: 'text-cyan-400 bg-cyan-500/20 border-cyan-400/50',
  URL: 'text-indigo-400 bg-indigo-500/20 border-indigo-400/50',
  Reference: 'text-teal-400 bg-teal-500/20 border-teal-400/50',
  NodeType: 'text-yellow-400 bg-yellow-500/20 border-yellow-400/50',
  Element: 'text-red-400 bg-red-500/20 border-red-400/50',
  Composite: 'text-violet-400 bg-violet-500/20 border-violet-400/50',
  Resource: 'text-emerald-400 bg-emerald-500/20 border-emerald-400/50',
}

export const PROPERTY_TYPES: PropertyType[] = [
  'String',
  'Number',
  'Boolean',
  'Date',
  'Array',
  'Object',
  'URL',
  'Reference',
  'NodeType',
  'Resource',
  'Element',
  'Composite',
]

export const DEFAULT_NODE_TYPE: Partial<import('./types').NodeTypeDefinition> = {
  name: '',
  allowed_children: [],
  properties: [],
  strict: false,
  versionable: true,
  publishable: true,
  auditable: true,
}
