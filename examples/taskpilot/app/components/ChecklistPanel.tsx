/**
 * Live checklist: useSql with a realtime subscription on the `projects`
 * workspace (refetch on node events), plus a refetch whenever a chat turn
 * settles (refreshSeq) so tool-driven changes show up even if an event is
 * missed.
 */
import { useEffect } from 'react';
import { useSql } from '../lib/raisin';

interface ItemRow {
  path: string;
  properties: {
    title?: string;
    status?: string;
    owner?: string;
    order?: number;
    notes?: string;
  };
}

const ITEMS_SQL =
  "SELECT path, properties FROM 'projects' WHERE CHILD_OF('/checklist') AND node_type = 'raisin:Node'";

export function ChecklistPanel({ refreshSeq }: { refreshSeq: number }) {
  // NOTE: subscription path semantics — a plain path matches only that
  // exact node; '/checklist/**' matches all descendants (the items).
  const { data, isLoading, error, refetch } = useSql<ItemRow>(ITEMS_SQL, [], {
    realtime: { workspace: 'projects', path: '/checklist/**' },
  });

  useEffect(() => {
    if (refreshSeq > 0) void refetch();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [refreshSeq]);

  const items = (data ?? [])
    .slice()
    .sort((a, b) => (a.properties.order ?? 99) - (b.properties.order ?? 99));
  const done = items.filter((i) => i.properties.status === 'done').length;

  return (
    <section className="panel checklist-panel" data-testid="checklist">
      <div className="panel-head">
        <h2 className="panel-title">Launch checklist</h2>
        {items.length > 0 && (
          <span className="counter" data-testid="checklist-counter">
            {done}/{items.length} done
          </span>
        )}
      </div>
      {error && <p className="error-banner">{error.message}</p>}
      {isLoading && items.length === 0 && <p className="muted">Loading items…</p>}
      <ul className="item-list">
        {items.map((item) => {
          const p = item.properties;
          const isDone = p.status === 'done';
          return (
            <li
              key={item.path}
              className={`item-row ${isDone ? 'done' : ''}`}
              data-testid="item-row"
              data-path={item.path}
              data-status={p.status}
            >
              <span className={`status-dot ${isDone ? 'done' : 'todo'}`}>
                {isDone ? '✓' : ''}
              </span>
              <span className="item-main">
                <span className="item-title">{p.title ?? item.path}</span>
                {p.notes ? <span className="item-notes">{p.notes}</span> : null}
              </span>
              {p.owner ? <span className="item-owner">{p.owner}</span> : null}
            </li>
          );
        })}
      </ul>
    </section>
  );
}
