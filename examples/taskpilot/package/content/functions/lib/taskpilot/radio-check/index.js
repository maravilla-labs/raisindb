/**
 * radio-check - read-only fun tool: checklist completion as a flight report.
 *
 * Input:  {}
 * Output: { callsign, completion_percent, done, total, message }
 */
async function handler() {
  const rows = await raisin.sql.query(
    "SELECT properties FROM 'projects' WHERE CHILD_OF('/checklist') AND node_type = 'raisin:Node'",
    [],
  );
  const total = rows.length;
  const done = rows.filter((r) => (r.properties || {}).status === 'done').length;
  const pct = total === 0 ? 0 : Math.round((done / total) * 100);

  const phases = [
    { min: 100, msg: 'Wheels up - checklist complete, cleared for launch.' },
    { min: 75, msg: 'Final approach - almost there, keep the nose steady.' },
    { min: 50, msg: 'Cruising altitude - solid progress, hold the heading.' },
    { min: 25, msg: 'Climbing through the clouds - good start, keep climbing.' },
    { min: 1, msg: 'Taxiing to the runway - first items done, throttle up.' },
    { min: 0, msg: 'Pre-flight checks pending - all items still open.' },
  ];
  const message = phases.find((p) => pct >= p.min).msg;

  console.log('[radio-check]', done + '/' + total, '(' + pct + '%)');
  return {
    callsign: 'TaskPilot One',
    completion_percent: pct,
    done,
    total,
    message,
  };
}
