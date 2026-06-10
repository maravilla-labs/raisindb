/**
 * list-items - list the launch checklist items, optionally filtered.
 *
 * Input:  { status?: "todo"|"done" }
 * Output: { items: [{ path, title, status, owner, order, notes }] }
 */
async function handler(input) {
  const { status } = input || {};

  // NOTE: in the function runtime raisin.sql.query returns the row array
  // directly (unlike the client SDK's executeSql, which returns { rows }).
  const rows = await raisin.sql.query(
    "SELECT path, properties FROM 'projects' WHERE CHILD_OF('/checklist') AND node_type = 'raisin:Node' ORDER BY path",
    [],
  );
  const items = [];
  for (const row of rows) {
    const p = row.properties || {};
    if (status && p.status !== status) continue;
    items.push({
      path: row.path,
      title: p.title,
      status: p.status,
      owner: p.owner || null,
      order: p.order ?? null,
      notes: p.notes || '',
    });
  }
  items.sort((a, b) => (a.order ?? 99) - (b.order ?? 99));

  console.log('[list-items] returning', items.length, 'items');
  return { items };
}
