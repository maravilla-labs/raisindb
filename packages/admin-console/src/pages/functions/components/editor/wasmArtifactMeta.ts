/**
 * What a `.wasm` artifact node says about itself.
 *
 * Split out of `WasmArtifactPanel.tsx` to keep that component readable: these
 * are pure readers over the node the API returned, and the panel is the only
 * caller. The shapes they tolerate mirror `raisin_models::nodes::asset` on the
 * server — a `file` Resource carrying the numbers, with flat `file_size` /
 * `content_hash` properties as the fallback the package installer writes.
 */

import type { Node as NodeType } from '../../../../api/nodes'

/** The WIT world every RaisinDB function component must export. */
export const WASM_WORLD = 'raisin:function/function@0.1.0'

/** A `raisin:Asset` file property as the API serializes `PropertyValue::Resource`. */
interface FileResource {
  size?: number
  mime_type?: string
  updated_at?: string
  metadata?: Record<string, unknown>
}

function fileResource(node: NodeType | null): FileResource | null {
  const file = node?.properties?.file
  return file && typeof file === 'object' ? (file as FileResource) : null
}

/** Artifact size in bytes, from the Resource or the flat `file_size` property. */
export function artifactSize(node: NodeType | null): number | null {
  const size = fileResource(node)?.size
  if (typeof size === 'number') return size
  const flat = node?.properties?.file_size
  return typeof flat === 'number' ? flat : null
}

/**
 * Content hash as the SERVER recorded it.
 *
 * Shown for identification only — it is tenant-writable, so nothing here (and
 * nothing on the server) may treat it as proof of what the bytes are.
 */
export function artifactHash(node: NodeType | null): string | null {
  const nested = fileResource(node)?.metadata?.content_hash
  if (typeof nested === 'string') return nested
  const flat = node?.properties?.content_hash
  return typeof flat === 'string' ? flat : null
}

/** When the artifact last changed, node revision time preferred. */
export function artifactUpdatedAt(node: NodeType | null): string | null {
  return node?.updated_at || fileResource(node)?.updated_at || null
}

/** Human-readable byte count. */
export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KiB`
  return `${(bytes / (1024 * 1024)).toFixed(2)} MiB`
}

/**
 * The handler this artifact serves, per the parent function's `entry_file`.
 *
 * `main.wasm:on-order` selects `on-order`; a bare `main.wasm` means `default`.
 * The name is NOT validated against anything — the guest owns its handler
 * namespace and answers an unknown name itself.
 */
export function handlerOf(functionNode: NodeType | null): string {
  const entry = functionNode?.properties?.entry_file
  if (typeof entry !== 'string' || !entry.includes(':')) return 'default'
  const handler = entry.slice(entry.lastIndexOf(':') + 1).trim()
  return handler || 'default'
}
