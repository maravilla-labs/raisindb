/**
 * list-staff - list staff members and their availability.
 *
 * Input:  {} (no filters)
 * Output: { staff: [{ path, name, role, email, reachable, available_days: [..] }] }
 *         reachable = the email has a registered chat account (message-staff works)
 */
async function handler() {
  // raisin.sql.query returns the row array directly in the function runtime
  const rows = await raisin.sql.query(
    "SELECT path, properties FROM 'staffing' WHERE CHILD_OF('/staff') AND node_type = 'raisin:Node' ORDER BY path",
    [],
  );

  // Which staff emails have a registered chat account? Only those can be
  // reached with the message-staff tool.
  const users = await raisin.sql.query(
    "SELECT properties FROM 'raisin:access_control' WHERE node_type = 'raisin:User'",
    [],
  );
  const reachableEmails = new Set();
  for (const u of users) {
    const email = (u.properties || {}).email;
    if (email) reachableEmails.add(String(email).toLowerCase());
  }

  const staff = rows.map((row) => {
    const p = row.properties || {};
    const email = p.email || null;
    return {
      path: row.path,
      name: p.title,
      role: p.role,
      email,
      reachable: !!(email && reachableEmails.has(String(email).toLowerCase())),
      available_days: p.available_days || [],
    };
  });

  console.log('[list-staff] returning', staff.length, 'staff members');
  return { staff };
}
