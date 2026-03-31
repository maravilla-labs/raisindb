/**
 * Utility for computing property-level diffs between two objects
 * Used for comparing node properties across revisions
 */

export type DiffType = 'added' | 'removed' | 'modified' | 'unchanged'

export interface PropertyDiff {
  path: string[]
  type: DiffType
  oldValue?: unknown
  newValue?: unknown
}

/**
 * Deep comparison of two values
 */
function deepEqual(a: unknown, b: unknown): boolean {
  if (a === b) return true

  if (a === null || b === null) return false
  if (a === undefined || b === undefined) return false

  if (typeof a !== typeof b) return false

  if (typeof a === 'object' && typeof b === 'object') {
    const aObj = a as Record<string, unknown>
    const bObj = b as Record<string, unknown>

    const aKeys = Object.keys(aObj)
    const bKeys = Object.keys(bObj)

    if (aKeys.length !== bKeys.length) return false

    for (const key of aKeys) {
      if (!bKeys.includes(key)) return false
      if (!deepEqual(aObj[key], bObj[key])) return false
    }

    return true
  }

  return false
}

/**
 * Compute property-level diffs between two objects
 * Returns a flat list of changes with their paths
 */
export function computePropertyDiff(
  oldObj: Record<string, unknown> | undefined,
  newObj: Record<string, unknown> | undefined,
  parentPath: string[] = []
): PropertyDiff[] {
  const diffs: PropertyDiff[] = []

  // Handle edge cases
  if (!oldObj && !newObj) return diffs
  if (!oldObj && newObj) {
    // Everything in newObj is added
    Object.keys(newObj).forEach(key => {
      diffs.push({
        path: [...parentPath, key],
        type: 'added',
        newValue: newObj[key],
      })
    })
    return diffs
  }
  if (oldObj && !newObj) {
    // Everything in oldObj is removed
    Object.keys(oldObj).forEach(key => {
      diffs.push({
        path: [...parentPath, key],
        type: 'removed',
        oldValue: oldObj[key],
      })
    })
    return diffs
  }

  // At this point, both oldObj and newObj are defined
  if (!oldObj || !newObj) return diffs // TypeScript guard

  // Get all unique keys from both objects
  const allKeys = new Set([...Object.keys(oldObj), ...Object.keys(newObj)])

  for (const key of allKeys) {
    const oldValue = oldObj[key]
    const newValue = newObj[key]
    const currentPath = [...parentPath, key]

    if (!(key in oldObj)) {
      // Property was added
      diffs.push({
        path: currentPath,
        type: 'added',
        newValue,
      })
    } else if (!(key in newObj)) {
      // Property was removed
      diffs.push({
        path: currentPath,
        type: 'removed',
        oldValue,
      })
    } else if (!deepEqual(oldValue, newValue)) {
      // Property was modified
      // If both are objects, recurse
      if (
        typeof oldValue === 'object' &&
        oldValue !== null &&
        !Array.isArray(oldValue) &&
        typeof newValue === 'object' &&
        newValue !== null &&
        !Array.isArray(newValue)
      ) {
        const nestedDiffs = computePropertyDiff(
          oldValue as Record<string, unknown>,
          newValue as Record<string, unknown>,
          currentPath
        )
        diffs.push(...nestedDiffs)
      } else {
        // Primitive or array change
        diffs.push({
          path: currentPath,
          type: 'modified',
          oldValue,
          newValue,
        })
      }
    }
    // If values are equal, we don't add a diff entry
  }

  return diffs
}

/**
 * Format a property path as a readable string
 */
export function formatPropertyPath(path: string[]): string {
  if (path.length === 0) return ''
  if (path.length === 1) return path[0]

  return path.reduce((acc, segment, i) => {
    if (i === 0) return segment
    // Check if segment is numeric (array index)
    if (/^\d+$/.test(segment)) {
      return `${acc}[${segment}]`
    }
    return `${acc}.${segment}`
  })
}

/**
 * Format a value for display in the diff
 * Truncates long strings and stringifies objects
 */
export function formatDiffValue(value: unknown, maxLength = 100): string {
  if (value === null) return 'null'
  if (value === undefined) return 'undefined'

  if (typeof value === 'string') {
    if (value.length > maxLength) {
      return `"${value.substring(0, maxLength)}..."`
    }
    return `"${value}"`
  }

  if (typeof value === 'object') {
    const json = JSON.stringify(value, null, 2)
    if (json.length > maxLength) {
      return `${json.substring(0, maxLength)}...`
    }
    return json
  }

  return String(value)
}

/**
 * Group diffs by their top-level property for better organization
 */
export function groupDiffsByProperty(diffs: PropertyDiff[]): Map<string, PropertyDiff[]> {
  const grouped = new Map<string, PropertyDiff[]>()

  for (const diff of diffs) {
    const topLevelKey = diff.path[0] || '(root)'
    const existing = grouped.get(topLevelKey) || []
    existing.push(diff)
    grouped.set(topLevelKey, existing)
  }

  return grouped
}
