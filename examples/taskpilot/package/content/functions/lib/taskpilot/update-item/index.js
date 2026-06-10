/**
 * update-item - set a checklist item's status and/or notes.
 *
 * Input:  { item_path: string, status?: "todo"|"done", notes?: string }
 * Output: { item_path, title, status, notes }
 */
async function handler(input) {
  const { item_path, status, notes } = input || {};
  if (!item_path || !item_path.startsWith('/checklist/')) {
    throw new Error('item_path is required and must start with /checklist/, got: ' + item_path);
  }
  if (status !== undefined && status !== 'todo' && status !== 'done') {
    throw new Error('status must be "todo" or "done", got: ' + status);
  }
  if (status === undefined && notes === undefined) {
    throw new Error('nothing to update - pass status and/or notes');
  }

  // raisin.sql.query returns the row array directly in the function runtime
  const existing = await raisin.sql.query(
    "SELECT path, properties FROM 'projects' WHERE path = $1",
    [item_path],
  );
  const row = existing[0];
  if (!row) {
    throw new Error('item not found: ' + item_path);
  }

  const props = row.properties || {};
  if (status !== undefined) props.status = status;
  if (notes !== undefined) props.notes = String(notes);

  await raisin.sql.execute(
    "UPDATE 'projects' SET properties = $1::jsonb WHERE path = $2",
    [JSON.stringify(props), item_path],
  );

  console.log('[update-item]', item_path, '->', props.status);
  return { item_path, title: props.title, status: props.status, notes: props.notes || '' };
}
