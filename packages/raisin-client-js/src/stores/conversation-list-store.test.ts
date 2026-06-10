import { describe, expect, it } from 'vitest';
import { buildChatsSubscriptionFilters } from './conversation-list-store';

describe('buildChatsSubscriptionFilters', () => {
  it('builds a recursive path filter so child conversation events match', () => {
    const filters = buildChatsSubscriptionFilters('/users/internal/alice');

    // The server matcher has no implicit prefix matching: a plain path only
    // matches the exact node, so the recursive `/**` suffix is required for
    // events on conversations created under the chats folder.
    expect(filters.path).toBe('/users/internal/alice/inbox/chats/**');
    expect(filters.workspace).toBe('raisin:access_control');
  });

  it('subscribes to both created and updated events (for unread counts)', () => {
    const filters = buildChatsSubscriptionFilters('/users/internal/alice');
    expect(filters.event_types).toContain('node:created');
    expect(filters.event_types).toContain('node:updated');
  });

  it('normalizes a workspace-prefixed user home', () => {
    const filters = buildChatsSubscriptionFilters(
      '/raisin:access_control/users/internal/alice',
    );
    expect(filters.path).toBe('/users/internal/alice/inbox/chats/**');
  });
});
