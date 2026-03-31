/**
 * Move Kanban Card AI Tool
 *
 * Handles card movement within and between Kanban boards.
 */
async function handleMoveCard(input) {
  const { board_path, card_uuid, to_column_id, to_position, target_board_path } = input;
  const WORKSPACE = 'launchpad';

  if (!board_path) {
    return { success: false, error: 'board_path is required' };
  }
  if (!card_uuid) {
    return { success: false, error: 'card_uuid is required' };
  }
  if (!to_column_id) {
    return { success: false, error: 'to_column_id is required' };
  }

  try {
    // Determine if cross-board move
    const isCrossBoard = target_board_path && target_board_path !== board_path;

    if (isCrossBoard) {
      return await crossBoardMove(board_path, target_board_path, card_uuid, to_column_id, to_position);
    } else {
      return await sameBoardMove(board_path, card_uuid, to_column_id, to_position);
    }
  } catch (err) {
    console.error('[kanban-move-card] Error:', err);
    return { success: false, error: err.message || String(err) };
  }

  async function sameBoardMove(boardPath, cardUuid, targetColumnId, targetPosition) {
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

    // Find and remove card from source column
    let movedCard = null;
    let fromColumnId = null;

    for (const col of columns) {
      if (!col.cards) continue;
      const index = col.cards.findIndex(c => c.uuid === cardUuid);
      if (index !== -1) {
        movedCard = col.cards.splice(index, 1)[0];
        fromColumnId = col.id;
        break;
      }
    }

    if (!movedCard) {
      return { success: false, error: `Card not found: ${cardUuid}` };
    }

    // Find target column
    const targetColumn = columns.find(c => c.id === targetColumnId);
    if (!targetColumn) {
      return { success: false, error: `Target column not found: ${targetColumnId}` };
    }

    // Initialize cards array if needed
    if (!targetColumn.cards) {
      targetColumn.cards = [];
    }

    // Insert at target position
    if (typeof targetPosition === 'number' && targetPosition >= 0) {
      targetColumn.cards.splice(targetPosition, 0, movedCard);
    } else {
      targetColumn.cards.push(movedCard);
    }

    // Save updated board
    await raisin.sql.query(`
      UPDATE launchpad
      SET properties = $1::jsonb
      WHERE path = $2
    `, [JSON.stringify({ ...props, columns }), boardPath]);

    return {
      success: true,
      card: movedCard,
      from_column_id: fromColumnId,
      to_column_id: targetColumnId,
      cross_board: false
    };
  }

  async function crossBoardMove(sourceBoardPath, targetBoardPath, cardUuid, targetColumnId, targetPosition) {
    // Fetch source board
    const sourceResult = await raisin.sql.query(
      `SELECT * FROM launchpad WHERE path = $1`, [sourceBoardPath]
    );
    const sourceRows = Array.isArray(sourceResult) ? sourceResult : (sourceResult?.rows || []);
    const sourceBoard = sourceRows[0];

    if (!sourceBoard) {
      return { success: false, error: 'Source board not found' };
    }

    // Fetch target board
    const targetResult = await raisin.sql.query(
      `SELECT * FROM launchpad WHERE path = $1`, [targetBoardPath]
    );
    const targetRows = Array.isArray(targetResult) ? targetResult : (targetResult?.rows || []);
    const targetBoard = targetRows[0];

    if (!targetBoard) {
      return { success: false, error: 'Target board not found' };
    }

    const sourceProps = typeof sourceBoard.properties === 'string'
      ? JSON.parse(sourceBoard.properties)
      : sourceBoard.properties;

    const targetProps = typeof targetBoard.properties === 'string'
      ? JSON.parse(targetBoard.properties)
      : targetBoard.properties;

    const sourceColumns = sourceProps?.columns || [];
    const targetColumns = targetProps?.columns || [];

    // Find and remove card from source
    let movedCard = null;
    let fromColumnId = null;

    for (const col of sourceColumns) {
      if (!col.cards) continue;
      const index = col.cards.findIndex(c => c.uuid === cardUuid);
      if (index !== -1) {
        movedCard = col.cards.splice(index, 1)[0];
        fromColumnId = col.id;
        break;
      }
    }

    if (!movedCard) {
      return { success: false, error: `Card not found in source board: ${cardUuid}` };
    }

    // Find target column
    const targetColumn = targetColumns.find(c => c.id === targetColumnId);
    if (!targetColumn) {
      return { success: false, error: `Target column not found: ${targetColumnId}` };
    }

    // Initialize cards array if needed
    if (!targetColumn.cards) {
      targetColumn.cards = [];
    }

    // Insert at target
    if (typeof targetPosition === 'number' && targetPosition >= 0) {
      targetColumn.cards.splice(targetPosition, 0, movedCard);
    } else {
      targetColumn.cards.push(movedCard);
    }

    // Update both boards
    await raisin.sql.query(`
      UPDATE launchpad
      SET properties = $1::jsonb
      WHERE path = $2
    `, [JSON.stringify({ ...sourceProps, columns: sourceColumns }), sourceBoardPath]);

    await raisin.sql.query(`
      UPDATE launchpad
      SET properties = $1::jsonb
      WHERE path = $2
    `, [JSON.stringify({ ...targetProps, columns: targetColumns }), targetBoardPath]);

    return {
      success: true,
      card: movedCard,
      from_column_id: fromColumnId,
      to_column_id: targetColumnId,
      cross_board: true
    };
  }
}
