// SPDX-License-Identifier: BSL-1.1

import { describe, it, expect } from 'vitest'
import { danglingGroups } from './Mounts'
import type { Integration, VirtualMount } from '../api/integrations'

const connector = (path: string, accountIds: string[]): Integration =>
  ({
    path,
    title: path,
    connected_accounts: accountIds.map((id) => ({ id, label: `${id}@example.test` })),
  }) as unknown as Integration

const mount = (id: string, integration_ref: string, account_ref?: string): VirtualMount =>
  ({ id, title: id, mount_path: `/${id}`, integration_ref, account_ref }) as unknown as VirtualMount

describe('danglingGroups', () => {
  it('collects every mount that names one removed connection', () => {
    const groups = danglingGroups(
      [
        mount('inbox', '/i/ms365', 'gone'),
        mount('sent', '/i/ms365', 'gone'),
        mount('drafts', '/i/ms365', 'gone'),
      ],
      [connector('/i/ms365', ['live'])],
    )
    expect(groups).toHaveLength(1)
    expect(groups[0].accountRef).toBe('gone')
    expect(groups[0].mounts.map((m) => m.id)).toEqual(['inbox', 'sent', 'drafts'])
  })

  /**
   * The safety property. Two removed connections were two mailboxes, so they
   * must never share one dropdown — that is how one person's mail lands under
   * another's path.
   */
  it('never merges mounts that named different removed connections', () => {
    const groups = danglingGroups(
      [mount('a', '/i/ms365', 'gone-1'), mount('b', '/i/ms365', 'gone-2')],
      [connector('/i/ms365', [])],
    )
    expect(groups).toHaveLength(2)
    expect(groups.map((g) => g.accountRef).sort()).toEqual(['gone-1', 'gone-2'])
  })

  it('keeps connectors apart even when the dangling id happens to match', () => {
    const groups = danglingGroups(
      [mount('a', '/i/ms365', 'x'), mount('b', '/i/hue', 'x')],
      [connector('/i/ms365', []), connector('/i/hue', [])],
    )
    expect(groups).toHaveLength(2)
  })

  it('ignores live references and unset ones', () => {
    expect(
      danglingGroups(
        [mount('live', '/i/ms365', 'live'), mount('unset', '/i/ms365')],
        [connector('/i/ms365', ['live'])],
      ),
    ).toHaveLength(0)
  })

  /**
   * Until the connector has loaded, a live reference and a dangling one look
   * identical — flagging then would put a repair prompt on healthy mounts.
   */
  it('reports nothing while the connector is still loading', () => {
    expect(danglingGroups([mount('a', '/i/ms365', 'whatever')], [])).toHaveLength(0)
  })
})
