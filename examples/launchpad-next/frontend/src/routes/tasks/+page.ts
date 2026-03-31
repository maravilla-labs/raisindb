import type { PageLoad } from './$types';
import { query } from '$lib/raisin';
import { localeClause } from '$lib/stores/locale';

export interface KanbanBoard {
  id: string;
  path: string;
  name: string;
  properties: {
    title: string;
    description?: string;
    columns?: Array<{
      id: string;
      title: string;
      cards: Array<{ uuid: string; title: string }>;
    }>;
  };
}

export const load: PageLoad = async () => {
  // Query all kanban boards in the launchpad workspace
  const boards = await query<KanbanBoard>(`
    SELECT id, path, name, properties
    FROM launchpad
    WHERE node_type = 'launchpad:Page'
      AND archetype = 'launchpad:KanbanBoard'
      ${localeClause()}
    ORDER BY properties->>'title'
  `);

  return { boards };
};
