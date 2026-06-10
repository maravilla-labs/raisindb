/**
 * assign-shift - assign a staff member to a shift (or clear the assignment).
 *
 * Input:  { shift_path: string, staff_name: string | null }
 *         shift_path is the node path under /shifts, e.g. "/shifts/sat-morning".
 *         staff_name null/empty clears the assignment (re-opens the shift).
 * Output: { shift_path, assignee, status }
 */
async function handler(input) {
  const { shift_path, staff_name } = input || {};
  if (!shift_path || !shift_path.startsWith('/shifts/')) {
    throw new Error('shift_path is required and must start with /shifts/, got: ' + shift_path);
  }

  // raisin.sql.query returns the row array directly in the function runtime
  const existing = await raisin.sql.query(
    "SELECT path, properties FROM 'staffing' WHERE path = $1",
    [shift_path],
  );
  const row = existing[0];
  if (!row) {
    throw new Error('shift not found: ' + shift_path);
  }

  const props = row.properties || {};
  const assignee = staff_name && String(staff_name).trim() ? String(staff_name).trim() : null;
  props.assignee = assignee;
  props.status = assignee ? 'filled' : 'open';

  await raisin.sql.execute(
    "UPDATE 'staffing' SET properties = $1::jsonb WHERE path = $2",
    [JSON.stringify(props), shift_path],
  );

  console.log('[assign-shift]', shift_path, '->', assignee || '(cleared)');
  return { shift_path, assignee, status: props.status };
}
