// SPDX-License-Identifier: BSL-1.1

import { Cloud, Lock } from 'lucide-react'
import type { Node } from '../api/nodes'

/** True when a node was materialized by a virtual mount (`__virtual: true`). */
export function isVirtualNode(node: Pick<Node, 'properties'>): boolean {
  return node.properties?.__virtual === true
}

/** Read the owning mount id stamped on a virtual node, if any. */
export function virtualMountId(node: Pick<Node, 'properties'>): string | undefined {
  const id = node.properties?.__mount_id
  return typeof id === 'string' ? id : undefined
}

interface VirtualNodeBadgeProps {
  node: Pick<Node, 'properties'>
  /** When true, the owning mount has writeback off — node is read-only. */
  readOnly?: boolean
  className?: string
}

/**
 * Small pill marking a node as backed by an external system (a virtual mount).
 * Renders nothing for regular nodes. Shows a lock when the mount is read-only.
 */
export default function VirtualNodeBadge({ node, readOnly, className = '' }: VirtualNodeBadgeProps) {
  if (!isVirtualNode(node)) return null

  const title = readOnly
    ? 'Virtual node — mounted from an external system (read-only: writeback is off)'
    : 'Virtual node — mounted from an external system'

  return (
    <span
      title={title}
      className={`inline-flex items-center gap-1 px-1.5 py-0.5 rounded-full text-[10px] font-medium bg-sky-500/20 text-sky-300 border border-sky-500/30 ${className}`}
    >
      {readOnly ? <Lock className="w-2.5 h-2.5" /> : <Cloud className="w-2.5 h-2.5" />}
      {readOnly ? 'Virtual · read-only' : 'Virtual'}
    </span>
  )
}
