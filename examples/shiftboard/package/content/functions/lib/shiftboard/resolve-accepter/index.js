/**
 * resolve-accepter - determine WHO accepted from the fill-shift loop output.
 *
 * The fill-shift workflow asks each candidate in order via an inbox task and
 * stops as soon as someone accepts (loop `until`). The loop output is
 * { results, count } where results[i] is the human response of candidate i:
 * { action: "accept"|"decline", comment?, completed_by, task_path }.
 * This step pairs results back with the candidate list to name the accepter
 * and the decliners, and builds the outcome summary for the manager.
 *
 * Input:  { candidates: [{name,email,user_path}], results: [...],
 *           shift_title, day, start, end, location, shift_path }
 *         results may be null/missing when a task timed out (timeout_edge).
 * Output: { accepted, accepter_name, accepter_email, asked,
 *           declined_names, summary }
 */
async function handler(input) {
  const data = input || {};
  const candidates = Array.isArray(data.candidates) ? data.candidates : [];
  const results = Array.isArray(data.results) ? data.results : [];

  let accepter = null;
  const declined = [];
  for (let i = 0; i < results.length; i++) {
    const r = results[i] || {};
    const c = candidates[i] || {};
    const who = c.name || c.email || 'unknown';
    if (r.action === 'accept' && !accepter) {
      accepter = c;
    } else if (!accepter) {
      declined.push(who);
    }
  }

  const label =
    (data.shift_title || data.shift_path || 'the shift') +
    (data.day ? ' (' + data.day + ' ' + data.start + '-' + data.end +
      (data.location ? ', ' + data.location : '') + ')' : '');

  let summary;
  if (accepter) {
    summary =
      accepter.name + ' accepted ' + label +
      ' via inbox task and has been assigned.' +
      (declined.length ? ' Declined before that: ' + declined.join(', ') + '.' : '');
  } else if (results.length === 0 && candidates.length === 0) {
    summary =
      'Could not fill ' + label +
      ': no staff member is both available that day and reachable.';
  } else {
    summary =
      'Could not fill ' + label + ': nobody accepted.' +
      (declined.length ? ' Declined: ' + declined.join(', ') + '.' : '') +
      (results.length < candidates.length
        ? ' Not every candidate responded before the deadline.'
        : '');
  }

  console.log('[resolve-accepter]', accepter ? 'accepted by ' + accepter.name : 'no accepter',
    '(asked ' + results.length + '/' + candidates.length + ')');

  return {
    accepted: !!accepter,
    accepter_name: accepter ? accepter.name : null,
    accepter_email: accepter ? accepter.email : null,
    asked: results.length,
    declined_names: declined,
    summary,
  };
}
