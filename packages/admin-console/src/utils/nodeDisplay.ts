// SPDX-License-Identifier: BSL-1.1

/**
 * How a node is labelled in the UI.
 *
 * A node's `name` is a PATH SEGMENT, not a label. Synced content makes the
 * difference stark: a mail connector names each message after the provider's
 * item id, so the tree, the breadcrumb and the detail header all rendered a
 * 200-character base64 blob while the readable subject sat in a property one
 * line below. Path stability requires that id; the operator should never see it.
 *
 * Rule: show a title-ish property when one exists, fall back to the segment,
 * and clamp everything so no single node can blow out the layout.
 */

import type { Node } from '../api/nodes'

/**
 * Properties treated as a human label, most specific first.
 *
 * `title` is the house convention. `subject` is here because mail is the case
 * that motivated all this and its natural field name is `subject` — including
 * it means a mail node reads correctly whether it was mapped before or after
 * `raisin:Mail` landed, without either mapper having to write both.
 */
const LABEL_PROPERTIES = ['title', 'subject', 'display_name', 'label'] as const

/** Longest label rendered inline before it is clamped. */
export const MAX_LABEL_CHARS = 80
/** Longest path rendered inline. Paths clamp in the MIDDLE — see below. */
export const MAX_PATH_CHARS = 90

function firstString(value: unknown): string | null {
  return typeof value === 'string' && value.trim().length > 0 ? value.trim() : null
}

/**
 * The human label for a node: a title-ish property when present, else `name`.
 *
 * Never returns an empty string — an unnamed node falls back to its id so a row
 * is always clickable and never collapses to a zero-width target.
 */
export function nodeLabel(node: Pick<Node, 'name' | 'properties' | 'id'>): string {
  const props = (node.properties || {}) as Record<string, unknown>
  for (const key of LABEL_PROPERTIES) {
    const found = firstString(props[key])
    if (found) return found
  }
  return firstString(node.name) || node.id || '(untitled)'
}

/** [`nodeLabel`] clamped for inline rendering. Pair with `title={nodeLabel(n)}`. */
export function nodeLabelShort(
  node: Pick<Node, 'name' | 'properties' | 'id'>,
  max: number = MAX_LABEL_CHARS,
): string {
  return truncateEnd(nodeLabel(node), max)
}

/** True when the label shown differs from the node's actual path segment. */
export function labelDiffersFromName(node: Pick<Node, 'name' | 'properties' | 'id'>): boolean {
  return nodeLabel(node) !== node.name
}

/**
 * Clamp at the end with an ellipsis. Uses Array.from so astral characters
 * (emoji in a subject line) are never split into broken halves.
 */
export function truncateEnd(value: string, max: number): string {
  const chars = Array.from(value)
  return chars.length <= max ? value : chars.slice(0, Math.max(1, max - 1)).join('') + '…'
}

/**
 * Clamp in the MIDDLE, keeping both ends.
 *
 * For paths and ids the tail is the informative part — `/senol/AAMkAD…` tells
 * you nothing, while `/senol/AAMk…QGizQY` at least identifies the node and stays
 * comparable against another. End-truncation on a tree of provider ids renders
 * every row identical.
 */
export function truncateMiddle(value: string, max: number): string {
  const chars = Array.from(value)
  if (chars.length <= max) return value
  const keep = max - 1
  const head = Math.ceil(keep / 2)
  const tail = Math.floor(keep / 2)
  return chars.slice(0, head).join('') + '…' + chars.slice(chars.length - tail).join('')
}

/** A path clamped for inline rendering. Pair with `title={path}`. */
export function pathShort(path: string, max: number = MAX_PATH_CHARS): string {
  return truncateMiddle(path, max)
}

/**
 * Breadcrumb segments, clamped per segment.
 *
 * Clamped individually rather than as a whole path so the LAST segment — the
 * node you are actually looking at — never gets dropped by a long ancestor.
 */
export function breadcrumbSegments(path: string, maxPerSegment = 24): string[] {
  return path
    .split('/')
    .filter(Boolean)
    .map((segment) => truncateMiddle(segment, maxPerSegment))
}
