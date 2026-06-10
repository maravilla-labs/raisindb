/**
 * pick-candidates - find the staff members who could take a shift.
 *
 * Used by the /flows/fill-shift workflow as its first step. A candidate is
 * a staff member who is AVAILABLE on the shift's day AND REACHABLE, i.e.
 * their email belongs to a registered identity user (raisin:User node) -
 * that user's home path is what the workflow's human task is assigned to.
 *
 * Input:  { shift_path: string }  e.g. "/shifts/sun-evening"
 * Output: { shift_path, shift_title, day, start, end, location,
 *           candidates: [{ name, email, user_path }] }
 *         candidates are in stable order (staff node path order).
 */
async function handler(input) {
  const { shift_path } = input || {};
  if (!shift_path || shift_path.indexOf('/shifts/') !== 0) {
    throw new Error('shift_path is required and must start with /shifts/, got: ' + shift_path);
  }

  // raisin.sql.query returns the row array directly in the function runtime
  const shiftRows = await raisin.sql.query(
    "SELECT path, properties FROM 'staffing' WHERE path = $1",
    [shift_path],
  );
  if (!shiftRows.length) {
    throw new Error('shift not found: ' + shift_path);
  }
  const shift = shiftRows[0].properties || {};

  // Staff board (stable order by node path)
  const staffRows = await raisin.sql.query(
    "SELECT path, properties FROM 'staffing' WHERE CHILD_OF('/staff') AND node_type = 'raisin:Node' ORDER BY path",
    [],
  );

  // Registered identity users: email -> home path (the inbox-task assignee)
  const users = await raisin.sql.query(
    "SELECT path, properties FROM 'raisin:access_control' WHERE node_type = 'raisin:User'",
    [],
  );
  const userPathByEmail = {};
  for (const u of users) {
    const email = (u.properties || {}).email;
    if (email) userPathByEmail[String(email).toLowerCase()] = u.path;
  }

  const candidates = [];
  for (const row of staffRows) {
    const p = row.properties || {};
    const days = p.available_days || [];
    if (days.indexOf(shift.day) === -1) continue;
    const email = p.email ? String(p.email).toLowerCase() : null;
    if (!email || !userPathByEmail[email]) continue; // not reachable - no inbox
    candidates.push({
      name: p.title,
      email,
      user_path: userPathByEmail[email],
    });
  }

  console.log(
    '[pick-candidates]',
    shift_path,
    '(' + shift.day + ')',
    '->',
    candidates.length,
    'candidates:',
    candidates.map(function (c) { return c.name; }).join(', ') || '(none)',
  );

  return {
    shift_path,
    shift_title: shift.title,
    day: shift.day,
    start: shift.start,
    end: shift.end,
    location: shift.location,
    candidates,
  };
}
