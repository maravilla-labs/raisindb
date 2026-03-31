/**
 * Kanban Cards AI Tool
 *
 * Handles card-level CRUD operations for Kanban boards.
 */
async function handleKanbanCards(input) {
  const { operation, board_path, column_id, card_uuid, title, description, note, position } = input;
  const WORKSPACE = 'launchpad';

  if (!board_path) {
    return { success: false, error: 'board_path is required' };
  }

  try {
    switch (operation) {
      case 'add':
        return await addCard(board_path, column_id, title, description, note, position);
      case 'update':
        return await updateCard(board_path, card_uuid, title, description, note);
      case 'delete':
        return await deleteCard(board_path, card_uuid);
      default:
        return { success: false, error: `Unknown operation: ${operation}` };
    }
  } catch (err) {
    console.error('[kanban-cards] Error:', err);
    return { success: false, error: err.message || String(err) };
  }

  async function addCard(boardPath, colId, cardTitle, cardDesc, cardNote, cardPosition) {
    if (!colId) {
      return { success: false, error: 'column_id is required for add operation' };
    }
    if (!cardTitle) {
      return { success: false, error: 'title is required for add operation' };
    }

    // Fetch board
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

    const columns = props?.columns || [];
    const column = columns.find(c => c.id === colId);
    if (!column) {
      return { success: false, error: `Column not found: ${colId}` };
    }

    // Generate UUID for new card
    const newCard = {
      uuid: generateUUID(),
      element_type: 'launchpad:KanbanCard',
      title: cardTitle,
      description: cardDesc || '',
      note: cardNote || ''
    };

    // Initialize cards array if needed
    if (!column.cards) {
      column.cards = [];
    }

    // Insert at position or end
    if (typeof cardPosition === 'number' && cardPosition >= 0) {
      column.cards.splice(cardPosition, 0, newCard);
    } else {
      column.cards.push(newCard);
    }

    // Save updated board
    await raisin.sql.query(`
      UPDATE launchpad
      SET properties = $1::jsonb
      WHERE path = $2
    `, [JSON.stringify({ ...props, columns }), boardPath]);

    return { success: true, card: newCard };
  }

  async function updateCard(boardPath, cardUuid, newTitle, newDesc, newNote) {
    if (!cardUuid) {
      return { success: false, error: 'card_uuid is required for update operation' };
    }

    // Fetch board
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

    const columns = props?.columns || [];
    let foundCard = null;

    for (const col of columns) {
      if (!col.cards) continue;
      const card = col.cards.find(c => c.uuid === cardUuid);
      if (card) {
        if (newTitle !== undefined) card.title = newTitle;
        if (newDesc !== undefined) card.description = newDesc;
        if (newNote !== undefined) card.note = newNote;
        foundCard = card;
        break;
      }
    }

    if (!foundCard) {
      return { success: false, error: `Card not found: ${cardUuid}` };
    }

    await raisin.sql.query(`
      UPDATE launchpad
      SET properties = $1::jsonb
      WHERE path = $2
    `, [JSON.stringify({ ...props, columns }), boardPath]);

    return { success: true, card: foundCard };
  }

  async function deleteCard(boardPath, cardUuid) {
    if (!cardUuid) {
      return { success: false, error: 'card_uuid is required for delete operation' };
    }

    // Fetch board
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

    const columns = props?.columns || [];
    let deleted = false;

    for (const col of columns) {
      if (!col.cards) continue;
      const index = col.cards.findIndex(c => c.uuid === cardUuid);
      if (index !== -1) {
        col.cards.splice(index, 1);
        deleted = true;
        break;
      }
    }

    if (!deleted) {
      return { success: false, error: `Card not found: ${cardUuid}` };
    }

    await raisin.sql.query(`
      UPDATE launchpad
      SET properties = $1::jsonb
      WHERE path = $2
    `, [JSON.stringify({ ...props, columns }), boardPath]);

    return { success: true, deleted_uuid: cardUuid };
  }
}

/**
 * Generate a UUID v4
 */
function generateUUID() {
  return 'xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx'.replace(/[xy]/g, function(c) {
    const r = Math.random() * 16 | 0;
    const v = c === 'x' ? r : (r & 0x3 | 0x8);
    return v.toString(16);
  });
}
