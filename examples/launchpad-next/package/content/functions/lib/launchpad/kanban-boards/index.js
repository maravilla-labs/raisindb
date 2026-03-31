/**
 * Kanban Boards AI Tool
 *
 * Handles board-level CRUD operations for Kanban boards.
 */
async function handleKanbanBoards(input) {
  const { operation, board_path, board_id, parent_path, title, slug, description, columns } = input;
  const WORKSPACE = 'launchpad';

  try {
    switch (operation) {
      case 'list':
        return await listBoards();
      case 'get':
        return await getBoard(board_path, board_id);
      case 'create':
        return await createBoard(parent_path, title, slug, description, columns);
      case 'update':
        return await updateBoard(board_path, title, description);
      case 'delete':
        return await deleteBoard(board_path);
      default:
        return { success: false, error: `Unknown operation: ${operation}` };
    }
  } catch (err) {
    console.error('[kanban-boards] Error:', err);
    return { success: false, error: err.message || String(err) };
  }

  async function listBoards() {
    const result = await raisin.sql.query(`
      SELECT id, path, name, properties->>'title' as title,
             properties->>'description' as description,
             properties->>'slug' as slug
      FROM launchpad
      WHERE archetype = 'launchpad:KanbanBoard'
      ORDER BY created_at DESC
    `);
    const rows = Array.isArray(result) ? result : (result?.rows || []);
    return {
      success: true,
      boards: rows.map(r => ({
        id: r.id,
        path: r.path,
        name: r.name,
        title: r.title,
        description: r.description,
        slug: r.slug
      }))
    };
  }

  async function getBoard(boardPath, boardId) {
    if (!boardPath && !boardId) {
      return { success: false, error: 'Either board_path or board_id is required' };
    }

    let board;
    if (boardPath) {
      const result = await raisin.sql.query(
        `SELECT * FROM launchpad WHERE path = $1`, [boardPath]
      );
      const rows = Array.isArray(result) ? result : (result?.rows || []);
      board = rows[0] || null;
    } else {
      const result = await raisin.sql.query(
        `SELECT * FROM launchpad WHERE id = $1`, [boardId]
      );
      const rows = Array.isArray(result) ? result : (result?.rows || []);
      board = rows[0] || null;
    }

    if (!board) {
      return { success: false, error: 'Board not found' };
    }

    const props = typeof board.properties === 'string'
      ? JSON.parse(board.properties)
      : board.properties;

    return {
      success: true,
      board: {
        id: board.id,
        path: board.path,
        name: board.name,
        title: props?.title,
        description: props?.description,
        slug: props?.slug,
        columns: props?.columns || []
      }
    };
  }

  async function createBoard(parentPath, boardTitle, boardSlug, boardDesc, initialColumns) {
    if (!parentPath) {
      return { success: false, error: 'parent_path is required' };
    }
    if (!boardTitle) {
      return { success: false, error: 'title is required' };
    }
    if (!boardSlug) {
      return { success: false, error: 'slug is required' };
    }

    // Default columns if none provided
    const defaultColumns = [
      { id: 'col-backlog', title: 'Backlog', cards: [] },
      { id: 'col-in-progress', title: 'In Progress', cards: [] },
      { id: 'col-done', title: 'Done', cards: [] }
    ];

    const columnsToUse = initialColumns && initialColumns.length > 0
      ? initialColumns.map(c => ({ id: c.id, title: c.title, cards: c.cards || [] }))
      : defaultColumns;

    const boardPath = `${parentPath}/${boardSlug}`;

    await raisin.sql.query(`
      INSERT INTO launchpad (path, node_type, archetype, properties)
      VALUES ($1, 'launchpad:Page', 'launchpad:KanbanBoard', $2::jsonb)
    `, [boardPath, JSON.stringify({
      title: boardTitle,
      slug: boardSlug,
      description: boardDesc || '',
      columns: columnsToUse
    })]);

    // Fetch the created board to get its ID
    const result = await raisin.sql.query(
      `SELECT id FROM launchpad WHERE path = $1`, [boardPath]
    );
    const rows = Array.isArray(result) ? result : (result?.rows || []);
    const newId = rows[0]?.id;

    return {
      success: true,
      board: {
        id: newId,
        path: boardPath,
        name: boardSlug,
        title: boardTitle,
        description: boardDesc || '',
        slug: boardSlug,
        columns: columnsToUse
      }
    };
  }

  async function updateBoard(boardPath, newTitle, newDesc) {
    if (!boardPath) {
      return { success: false, error: 'board_path is required' };
    }

    // Fetch current board
    const result = await raisin.sql.query(
      `SELECT * FROM launchpad WHERE path = $1`, [boardPath]
    );
    const rows = Array.isArray(result) ? result : (result?.rows || []);
    const board = rows[0];

    if (!board) {
      return { success: false, error: 'Board not found' };
    }

    const props = typeof board.properties === 'string'
      ? JSON.parse(board.properties)
      : board.properties;

    const updatedProperties = { ...props };
    if (newTitle !== undefined) updatedProperties.title = newTitle;
    if (newDesc !== undefined) updatedProperties.description = newDesc;

    await raisin.sql.query(`
      UPDATE launchpad
      SET properties = $1::jsonb
      WHERE path = $2
    `, [JSON.stringify(updatedProperties), boardPath]);

    return {
      success: true,
      board: {
        id: board.id,
        path: boardPath,
        title: updatedProperties.title,
        description: updatedProperties.description,
        slug: updatedProperties.slug
      }
    };
  }

  async function deleteBoard(boardPath) {
    if (!boardPath) {
      return { success: false, error: 'board_path is required' };
    }

    await raisin.sql.query(`DELETE FROM launchpad WHERE path = $1`, [boardPath]);
    return { success: true, deleted_path: boardPath };
  }
}
