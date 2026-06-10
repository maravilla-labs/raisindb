/**
 * list-shifts - list shifts on the staffing board, optionally filtered.
 *
 * Input:  { day?: "friday"|"saturday"|"sunday", status?: "open"|"filled" }
 * Output: { shifts: [{ path, title, day, start, end, location, outdoor,
 *                      status, assignee }] }
 */
async function handler(input) {
  const { day, status } = input || {};

  // NOTE: in the function runtime raisin.sql.query returns the row array
  // directly (unlike the client SDK's executeSql, which returns { rows }).
  const rows = await raisin.sql.query(
    "SELECT path, properties FROM 'staffing' WHERE CHILD_OF('/shifts') AND node_type = 'raisin:Node' ORDER BY path",
    [],
  );
  const shifts = [];
  for (const row of rows) {
    const p = row.properties || {};
    if (day && p.day !== day) continue;
    if (status && p.status !== status) continue;
    shifts.push({
      path: row.path,
      title: p.title,
      day: p.day,
      start: p.start,
      end: p.end,
      location: p.location,
      outdoor: p.outdoor === true,
      status: p.status,
      assignee: p.assignee || null,
    });
  }

  console.log('[list-shifts] returning', shifts.length, 'shifts');
  return { shifts };
}
