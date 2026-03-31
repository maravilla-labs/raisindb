import {
  Type,
  FileText,
  Hash,
  Calendar,
  MapPin,
  ToggleLeft,
  Image,
  Link2,
  Tag,
  ListOrdered,
  Braces,
  Layers,
  Layout,
  Box,
  List,
} from 'lucide-react'
import type { FieldType, ArchetypeDefinition } from './types'

// All field types in display order
export const FIELD_TYPES: FieldType[] = [
  'TextField',
  'RichTextField',
  'NumberField',
  'DateField',
  'BooleanField',
  'MediaField',
  'ReferenceField',
  'TagField',
  'OptionsField',
  'LocationField',
  'JsonObjectField',
  'ElementField',
  'CompositeField',
  'SectionField',
  'ListingField',
]

// Icons for each field type
export const FIELD_TYPE_ICONS: Record<FieldType, any> = {
  TextField: Type,
  RichTextField: FileText,
  NumberField: Hash,
  DateField: Calendar,
  LocationField: MapPin,
  BooleanField: ToggleLeft,
  MediaField: Image,
  ReferenceField: Link2,
  TagField: Tag,
  OptionsField: ListOrdered,
  JsonObjectField: Braces,
  CompositeField: Layers,
  SectionField: Layout,
  ElementField: Box,
  ListingField: List,
}

// Display labels for each field type
export const FIELD_TYPE_LABELS: Record<FieldType, string> = {
  TextField: 'Text',
  RichTextField: 'Rich Text',
  NumberField: 'Number',
  DateField: 'Date',
  LocationField: 'Location',
  BooleanField: 'Boolean',
  MediaField: 'Media',
  ReferenceField: 'Reference',
  TagField: 'Tags',
  OptionsField: 'Options',
  JsonObjectField: 'JSON',
  CompositeField: 'Composite',
  SectionField: 'Section',
  ElementField: 'Element',
  ListingField: 'Listing',
}

// Colors for each field type (Tailwind classes)
export const FIELD_TYPE_COLORS: Record<FieldType, string> = {
  TextField: 'text-blue-400 bg-blue-500/20',
  RichTextField: 'text-purple-400 bg-purple-500/20',
  NumberField: 'text-green-400 bg-green-500/20',
  DateField: 'text-yellow-400 bg-yellow-500/20',
  LocationField: 'text-pink-400 bg-pink-500/20',
  BooleanField: 'text-orange-400 bg-orange-500/20',
  MediaField: 'text-indigo-400 bg-indigo-500/20',
  ReferenceField: 'text-cyan-400 bg-cyan-500/20',
  TagField: 'text-teal-400 bg-teal-500/20',
  OptionsField: 'text-lime-400 bg-lime-500/20',
  JsonObjectField: 'text-amber-400 bg-amber-500/20',
  CompositeField: 'text-violet-400 bg-violet-500/20',
  SectionField: 'text-fuchsia-400 bg-fuchsia-500/20',
  ElementField: 'text-rose-400 bg-rose-500/20',
  ListingField: 'text-sky-400 bg-sky-500/20',
}

// Default archetype template
export const DEFAULT_ARCHETYPE: ArchetypeDefinition = {
  name: '',
  title: '',
  description: '',
  base_node_type: undefined,
  fields: [],
  publishable: false,
}
