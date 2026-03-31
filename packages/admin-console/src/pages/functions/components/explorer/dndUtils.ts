const DROPPABLE_PREFIX = 'function-parent:'
const ROOT_PATH = '/'

export const ROOT_DROPPABLE_ID = `${DROPPABLE_PREFIX}${ROOT_PATH}`

export function getDroppableIdForParent(path: string | null | undefined): string {
  if (!path || path === '' || path === ROOT_PATH) {
    return ROOT_DROPPABLE_ID
  }
  return `${DROPPABLE_PREFIX}${path}`
}

export function getParentPathFromDroppableId(droppableId: string): string {
  if (droppableId === ROOT_DROPPABLE_ID) {
    return ROOT_PATH
  }

  if (droppableId.startsWith(DROPPABLE_PREFIX)) {
    return droppableId.slice(DROPPABLE_PREFIX.length) || ROOT_PATH
  }

  return ROOT_PATH
}
