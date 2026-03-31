import type { PageLoad } from './$types';
import { queryOne, type PageNode } from '$lib/raisin';
import { localeClause } from '$lib/stores/locale';
import { error } from '@sveltejs/kit';

export const load: PageLoad = async ({ params }) => {
  const boardId = params.boardId;

  // Query the specific board by name
  const board = await queryOne<PageNode>(`
    SELECT id, path, name, node_type, archetype, properties
    FROM launchpad
    WHERE name = $1
      AND node_type = 'launchpad:Page'
      AND archetype = 'launchpad:KanbanBoard'
      ${localeClause()}
    LIMIT 1
  `, [boardId]);

  if (!board) {
    throw error(404, `Board "${boardId}" not found`);
  }

  return { board, boardId };
};
