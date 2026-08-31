// Tree-mode (sync_config.folder_scope: "tree") tests. `node --test tree.test.mjs`.
//
// The load-bearing one is `walk and delta agree on every path`, written as a
// PROPERTY over a fixture mailbox tree rather than as an example: it drives
// opList exactly the way full.rs drives it (stack of {folder_id, prefix},
// prefix accumulated from item.name) and asserts that the folder path the walk
// builds is a byte-identical prefix of the relative_path opGetChanges emits for
// every message in that mailbox. A disagreement of one character relocates every
// node under the folder on every run — the bug that already bit google-drive and
// ms-graph.

import test from 'node:test'
import assert from 'node:assert/strict'

import { handler } from './index.js'
import { mailboxChain, mailboxDelimiter, segment, skipByAttribute } from './mailboxes.js'

// A "."-delimited server, which is the case the old "/"-then-"." guess got
// wrong in the direction that matters (it returned null and flattened the tree).
const BOXES = [
  { name: 'INBOX', path: 'INBOX', flags: ['\\HasChildren'] },
  { name: 'Projects', path: 'INBOX.Projects', flags: ['\\HasChildren'] },
  { name: 'Acme', path: 'INBOX.Projects.Acme', flags: ['\\HasNoChildren'] },
  { name: 'R&D/Legal', path: 'INBOX.Projects.R&D/Legal', flags: ['\\HasNoChildren'] },
  { name: 'Archive', path: 'INBOX.Archive', flags: ['\\HasNoChildren'] },
  { name: 'Old', path: 'INBOX.Archive.Old', flags: ['\\Noselect', '\\HasChildren'] },
  { name: '2019', path: 'INBOX.Archive.Old.2019', flags: ['\\HasNoChildren'] },
  { name: 'Trash', path: 'Trash', flags: ['\\Trash'] },
  { name: 'All Mail', path: 'INBOX.All', flags: ['\\All'] },
  { name: 'Everything', path: 'INBOX.All.Everything', flags: ['\\HasNoChildren'] },
]

/** One message per mailbox, uid derived from the path so collisions are visible. */
function messagesFor(mailbox, sinceUid) {
  const uid = 5 // deliberately the SAME uid in every mailbox
  if (sinceUid >= uid) return []
  return [
    {
      uid,
      subject: 'Same subject everywhere',
      from: 'ada@example.org',
      to: 'bob@example.org',
      date: '2026-01-01T00:00:00Z',
      flags: [],
      message_id: `<${mailbox}>`,
      headers: {},
    },
  ]
}

function stubImap({ boxes = BOXES, multi = false } = {}) {
  const calls = { fetchSince: [], listMailboxes: 0 }
  globalThis.raisin = {
    imap: {
      listMailboxes() {
        calls.listMailboxes++
        return boxes
      },
      fetchSince(conn, sinceUid, opts) {
        calls.fetchSince.push({ mailbox: opts.mailbox, sinceUid, limit: opts.limit })
        const messages = messagesFor(opts.mailbox, sinceUid).slice(0, opts.limit)
        return {
          messages,
          highestUid: messages.length ? messages[messages.length - 1].uid : sinceUid,
          uidvalidity: 100,
        }
      },
      fetchMessage(conn, uid, opts) {
        return { uid, subject: 'x', flags: [], text: `body of ${opts.mailbox}#${uid}` }
      },
    },
    email: { providers: () => ({ enabled: false, providers: [] }) },
  }
  if (multi) globalThis.raisin.imap.fetchSinceMulti = () => []
  return calls
}

const CRED = { username: 'ada@example.org', password: 'app-pw' }

const treeMount = (sync = {}) => ({
  mount_id: 'm1',
  mount_path: '/mail',
  remote_root: 'INBOX',
  api_config: { host: 'imap.example.org', port: 993, tls: true },
  sync_config: { folder_scope: 'tree', mailbox: 'INBOX', ...sync },
})

const call = (operation, mount, params = {}) =>
  handler({ operation, mount, credential: CRED, params })

/** Drive opList the way full.rs does: a stack of (folder_id, prefix). */
function walk(mount) {
  const paths = new Map() // folder external_id -> relative path built by the walk
  const stack = [[mount.remote_root, '']]
  while (stack.length) {
    const [folderId, prefix] = stack.pop()
    const page = call('list', mount, { folder_id: folderId })
    for (const item of page.items) {
      const rel = prefix ? `${prefix}/${item.name}` : item.name
      assert.equal(item.is_folder, true, 'the IMAP walk yields mailboxes only')
      paths.set(item.external_id, rel)
      stack.push([item.external_id, rel])
    }
  }
  return paths
}

/**
 * The delta now carries BOTH folders and messages (see `treeFolderChanges`), so
 * every assertion about messages has to say so. Splitting them here rather than
 * relaxing each assertion keeps "the delta emits exactly these messages" as
 * strict as it was.
 */
const msgs = (items) => items.filter((c) => !c.item.is_folder)
const dirs = (items) => items.filter((c) => c.item.is_folder)

/** Page the delta the way delta.rs does, until has_more is false. */
function drain(mount, { rounds = 20 } = {}) {
  let token = null
  const items = []
  const tokens = []
  for (let i = 0; i < rounds; i++) {
    const page = call('get_changes', mount, { since_token: token })
    items.push(...page.items)
    token = page.next_token
    tokens.push(token)
    assert.ok(token, 'next_token is never null')
    if (page.has_more === false) return { items, token, pages: i + 1, tokens }
  }
  assert.fail(`delta never reported has_more:false after ${rounds} pages`)
}

// ---- the property ----------------------------------------------------------

test('walk and delta agree on every path, byte for byte', () => {
  stubImap()
  const mount = treeMount()
  const folders = walk(mount)
  const { items } = drain(mount)

  assert.ok(msgs(items).length > 0, 'the fixture must produce messages')
  for (const change of msgs(items)) {
    const mailbox = change.item.metadata.mailbox
    const rel = change.relative_path
    const cut = rel.lastIndexOf('/')
    const folderPart = cut === -1 ? '' : rel.slice(0, cut)
    if (mailbox === 'INBOX') {
      // The root mailbox's messages sit at the mount root, exactly as folder
      // mode places them.
      assert.equal(folderPart, '', 'root-mailbox messages carry no folder prefix')
      continue
    }
    assert.equal(
      folderPart,
      folders.get(mailbox),
      `delta path for ${mailbox} must equal the folder path the walk built`
    )
  }
})

test('a "/" in a mailbox NAME cannot invent a path level', () => {
  stubImap()
  const folders = walk(treeMount())
  // "R&D/Legal" on a "."-delimited server is ONE mailbox. Left alone it would
  // read as two path levels on one side and one on the other.
  assert.equal(folders.get('INBOX.Projects.R&D/Legal'), 'Projects/R&D-Legal')
  assert.equal(segment('R&D/Legal'), 'R&D-Legal')
})

test('the delimiter is read back from the path, not guessed', () => {
  assert.equal(mailboxDelimiter({ path: 'INBOX.Projects', name: 'Projects' }), '.')
  assert.equal(mailboxDelimiter({ path: 'INBOX/Projects', name: 'Projects' }), '/')
  // A server using something else was flattened entirely by the old guess.
  assert.equal(mailboxDelimiter({ path: 'INBOX\\Projects', name: 'Projects' }), '\\')
  // What the binding will send once MailboxInfo carries it wins outright.
  assert.equal(mailboxDelimiter({ path: 'a-b', name: 'b', delimiter: '-' }), '-')
  assert.equal(mailboxDelimiter({ path: 'INBOX', name: 'INBOX' }), null)
})

test('a mailbox outside the mount root is skipped, never placed at the root', () => {
  assert.equal(mailboxChain('Other.Thing', '.', 'INBOX'), null)
  assert.deepEqual(mailboxChain('INBOX', '.', 'INBOX'), [])
  const folders = walk(treeMount())
  for (const rel of folders.values()) assert.ok(!rel.startsWith('/'))
  assert.equal([...folders.keys()].some((p) => p === 'Trash'), false)
})

// ---- Gmail's All Mail ------------------------------------------------------

test('\\All, \\Trash and \\Junk take their whole subtree out of the tree', () => {
  stubImap()
  const folders = walk(treeMount())
  // All Mail re-lists every message in the account; importing it would double
  // the whole mailbox under a second path.
  assert.equal(folders.has('INBOX.All'), false)
  assert.equal(folders.has('INBOX.All.Everything'), false, 'the SUBTREE goes too')
  assert.equal(skipByAttribute(['\\All']), true)
  assert.equal(skipByAttribute(['\\Trash']), true)
  assert.equal(skipByAttribute(['\\Junk']), true)
  // Keyed on the attribute, never the localised name.
  assert.equal(skipByAttribute(['\\HasNoChildren']), false)

  const { items } = drain(treeMount())
  for (const c of msgs(items)) assert.notEqual(c.item.metadata.mailbox, 'INBOX.All')
  // …and no folder node for it either, on the delta side as on the walk's.
  for (const c of dirs(items)) assert.ok(!c.item.external_id.startsWith('INBOX.All'))
})

test('a \\Noselect mailbox stays in the hierarchy but is never fetched', () => {
  const calls = stubImap()
  const folders = walk(treeMount())
  // Dropping it would orphan 2019, which the walk could then never reach.
  assert.equal(folders.get('INBOX.Archive.Old'), 'Archive/Old')
  assert.equal(folders.get('INBOX.Archive.Old.2019'), 'Archive/Old/2019')
  drain(treeMount())
  assert.equal(
    calls.fetchSince.some((c) => c.mailbox === 'INBOX.Archive.Old'),
    false,
    'SELECTing a \\Noselect mailbox is a protocol error'
  )
})

// ---- the id space ----------------------------------------------------------

test('the message id is namespaced by mailbox, so uid 5 twice is two nodes', () => {
  stubImap()
  const { items } = drain(treeMount())
  const ids = msgs(items).map((c) => c.item.external_id)
  assert.equal(new Set(ids).size, ids.length, 'every fixture mailbox holds uid 5')
  assert.ok(ids.includes('INBOX.Projects.Acme|100.5'))
  // Folder mode keeps the bare uid — no existing mount changes id space.
  stubImap()
  const folderModeMount = { ...treeMount(), sync_config: { mailbox: 'INBOX' } }
  const page = handler({
    operation: 'get_changes',
    mount: folderModeMount,
    credential: CRED,
    params: { since_token: null },
  })
  assert.equal(page.items[0].item.external_id, '5')
  assert.equal(page.items[0].relative_path, 'Same subject everywhere')
})

test('get_content resolves the mailbox from the id, not from the mount', () => {
  stubImap()
  const out = call('get_content', treeMount(), { item_id: 'INBOX.Archive|100.5' })
  assert.equal(out.content, 'body of INBOX.Archive#5')
  // Folder mode's bare uid still reads the mount's own mailbox.
  const folderOut = handler({
    operation: 'get_content',
    mount: { ...treeMount(), sync_config: { mailbox: 'INBOX' } },
    credential: CRED,
    params: { item_id: '5' },
  })
  assert.equal(folderOut.content, 'body of INBOX#5')
})

// ---- the rotation ----------------------------------------------------------

test('the rotation index advances, wraps, and reaches every mailbox', () => {
  const calls = stubImap()
  const { pages, items, tokens } = drain(treeMount())
  // 6 selectable mailboxes, 5 per call: no single call may hold them all.
  assert.ok(pages > 1, 'a bounded slice means more than one page')
  const visited = new Set(calls.fetchSince.map((c) => c.mailbox))
  assert.equal(visited.size, 6, 'every selectable mailbox was advanced')
  assert.equal(msgs(items).length, 6)
  // The folder page rides on the FIRST page of the round only.
  assert.equal(dirs(items).length, 6, 'every mailbox below the root, \\Noselect included')
  // The index lives in the CURSOR. Without that a busy mailbox starves the rest
  // forever and the run still reports ok.
  const cur = JSON.parse(tokens[0].slice('rsn-imaptree-1:'.length))
  assert.equal(cur.v, 1)
  assert.equal(cur.r, 5)
  assert.equal(cur.p, 5)
  const last = JSON.parse(tokens[tokens.length - 1].slice('rsn-imaptree-1:'.length))
  assert.equal(last.p, 0, 'a completed round resets the counter')
  assert.equal(last.r, 0, 'and wraps')
  // The tail of a round advances only what the round has left, so no mailbox is
  // logged into twice in one round.
  assert.equal(calls.fetchSince.length, 6)
})

test('a second poll with nothing new is one quiet round, and the cursor survives', () => {
  stubImap()
  const mount = treeMount()
  const first = drain(mount)
  let token = first.token
  const items = []
  for (let i = 0; i < 5; i++) {
    const page = call('get_changes', mount, { since_token: token })
    items.push(...page.items)
    token = page.next_token
    if (page.has_more === false) break
  }
  assert.equal(msgs(items).length, 0, 'no message is re-emitted')
  const cur = JSON.parse(token.slice('rsn-imaptree-1:'.length))
  assert.equal(cur.m['INBOX.Projects.Acme'].uid, 5, 'the per-mailbox cursor is kept')
})

// ---- cursor family ---------------------------------------------------------

test('flipping folder_scope re-baselines instead of resuming the wrong cursor', () => {
  stubImap()
  // A folder-mode token handed to tree mode: unusable, so every mailbox starts
  // from uid 0 — the one-time re-import the new id space forces anyway.
  const page = call('get_changes', treeMount(), { since_token: '100:5' })
  assert.ok(page.next_token.startsWith('rsn-imaptree-1:'))
  assert.equal(page.items.length > 0, true)

  // And the other way: a tree token in folder mode must not parse as a uid.
  stubImap()
  const folderPage = handler({
    operation: 'get_changes',
    mount: { ...treeMount(), sync_config: { mailbox: 'INBOX' } },
    credential: CRED,
    params: { since_token: 'rsn-imaptree-1:{"v":1,"r":0,"p":0,"m":{}}' },
  })
  assert.equal(folderPage.items.length, 1, 'a full re-list, not a resume at a bogus uid')
})

// ---- UIDVALIDITY -----------------------------------------------------------

test('a UIDVALIDITY reset re-enumerates ONE mailbox, not the account', () => {
  const calls = stubImap()
  globalThis.raisin.imap.fetchSince = (conn, sinceUid, opts) => {
    calls.fetchSince.push({ mailbox: opts.mailbox, sinceUid })
    // Only Archive reset its UID space.
    const uv = opts.mailbox === 'INBOX.Archive' ? 999 : 100
    const messages = messagesFor(opts.mailbox, sinceUid).slice(0, opts.limit)
    return {
      messages,
      highestUid: messages.length ? 5 : sinceUid,
      uidvalidity: uv,
    }
  }
  const mount = treeMount()
  const cursor =
    'rsn-imaptree-1:' +
    JSON.stringify({
      v: 1,
      r: 0,
      p: 0,
      m: {
        INBOX: { uv: 100, uid: 5 },
        'INBOX.Archive': { uv: 100, uid: 5 },
        'INBOX.Projects': { uv: 100, uid: 5 },
      },
    })
  calls.fetchSince.length = 0
  call('get_changes', mount, { since_token: cursor })
  const rewound = calls.fetchSince.filter((c) => c.sinceUid === 0).map((c) => c.mailbox)
  assert.ok(rewound.includes('INBOX.Archive'), 'the reset mailbox re-enumerates')
  assert.equal(
    rewound.filter((m) => m === 'INBOX' || m === 'INBOX.Projects').length,
    0,
    'a global comparison would have re-fetched the whole account to repair one mailbox'
  )
})

// ---- the baseline ----------------------------------------------------------

test('baseline_only seeds a watermark per mailbox and emits nothing', () => {
  const calls = stubImap()
  const page = call('get_changes', treeMount(), { since_token: null, baseline_only: true })
  assert.deepEqual(page.items, [], 'capture_delta_baseline throws items away anyway')
  assert.equal(page.has_more, false)
  // One fetch of ONE message per mailbox names the watermark. Answering the
  // baseline with real pages would fetch the entire tree and discard it.
  assert.ok(calls.fetchSince.every((c) => c.limit === 1))
  const cur = JSON.parse(page.next_token.slice('rsn-imaptree-1:'.length))
  assert.equal(cur.m['INBOX.Projects.Acme'].uid, 5)
  assert.equal(cur.m['INBOX.Projects.Acme'].uv, 100)

  // …and the next real poll therefore emits nothing, which is the same
  // "from now on" folder mode has had since its first walk.
  const next = call('get_changes', treeMount(), { since_token: page.next_token })
  assert.deepEqual(msgs(next.items), [])
})

// ---- the ceiling -----------------------------------------------------------

test('too many mailboxes is a config_error, never a silent truncation', () => {
  const many = [{ name: 'INBOX', path: 'INBOX', flags: [] }]
  for (let i = 0; i < 60; i++) {
    many.push({ name: `f${i}`, path: `INBOX.f${i}`, flags: ['\\HasNoChildren'] })
  }
  stubImap({ boxes: many })
  // A truncated mailbox set is a PARTIAL `seen`, and reconcile would then delete
  // every mailbox it never heard about along with everything under it.
  assert.throws(
    () => call('list', treeMount(), {}),
    (e) => e.code === 'config_error' && /ceiling/.test(e.message)
  )
  assert.throws(
    () => call('get_changes', treeMount(), { since_token: null }),
    (e) => e.code === 'config_error'
  )
})

// ---- folder mode is untouched ---------------------------------------------

test('folder mode is byte-for-byte what it was', () => {
  stubImap()
  const mount = { ...treeMount(), sync_config: { mailbox: 'INBOX' } }
  const page = handler({ operation: 'get_changes', mount, credential: CRED, params: {} })
  assert.equal(page.next_token, '100:5')
  assert.equal(page.has_more, undefined, 'no has_more: the legacy paging rules still apply')
  assert.equal(page.items[0].item.external_id, '5')
  // And `list` still enumerates the whole account's mailbox tree.
  // The engine passes folder_id = mount.remote_root on the first call.
  const list = handler({
    operation: 'list',
    mount,
    credential: CRED,
    params: { folder_id: 'INBOX' },
  })
  assert.deepEqual(
    list.items.map((i) => i.external_id).sort(),
    ['INBOX.Archive', 'INBOX.Projects'],
    'children of INBOX, including \\Noselect and the ones tree mode filters out'
  )
  // Folder mode still parents a mailbox by suffix alone, so the fixture's
  // "[Gmail]-shaped" INBOX.All (whose display name is not the path tail) is
  // listed as top-level — the same answer the old "/"-then-"." guess gave, and
  // the reason the ROOT-relative fallback lives in mailboxChain rather than in
  // mailboxParentPath: changing the flat listing's shape would relocate folder
  // nodes on every mount that exists today.
  const top = handler({ operation: 'list', mount, credential: CRED, params: {} })
  assert.deepEqual(top.items.map((i) => i.external_id).sort(), ['INBOX', 'INBOX.All', 'Trash'])
})

test('anything but the literal string "tree" is folder mode', () => {
  stubImap()
  for (const scope of [undefined, '', 'folder', 'Tree', 'TREE', true, 'subtree']) {
    const mount = { ...treeMount(), sync_config: { mailbox: 'INBOX', folder_scope: scope } }
    const page = handler({ operation: 'get_changes', mount, credential: CRED, params: {} })
    assert.equal(page.next_token, '100:5', `folder_scope=${String(scope)} must stay folder mode`)
  }
})

// ---- the walk's root, which is NOT the same string as the mount's mailbox ----

test('the walk still finds its root when remote_root and sync_config.mailbox differ', () => {
  stubImap()
  // Exactly what the shipped bundle produces: the entry sets remote_root INBOX,
  // and the operator's answer to the Mailbox prompt lands on
  // sync_config.mailbox. full.rs seeds its stack with remote_root, so the first
  // list call arrives with folder_id "INBOX" against a mount rooted at
  // INBOX.Projects. Looking that id up among the mailboxes BELOW the root finds
  // nothing, and the empty first page meant NO folder node was ever created
  // while the delta kept emitting messages beneath them.
  const mount = {
    ...treeMount(),
    remote_root: 'INBOX',
    sync_config: { folder_scope: 'tree', mailbox: 'INBOX.Projects' },
  }
  const page = call('list', mount, { folder_id: 'INBOX' })
  assert.deepEqual(
    page.items.map((i) => i.external_id).sort(),
    ['INBOX.Projects.Acme', 'INBOX.Projects.R&D/Legal']
  )
  // And the delta agrees with it: chains are relative to the SAME root.
  const { items } = drain(mount)
  const acme = items.find((c) => c.item.metadata.mailbox === 'INBOX.Projects.Acme')
  assert.equal(acme.relative_path, 'Acme/100.5')
})

// ---- the baseline is bounded, because it is the one call with no retry ------

test('the baseline seeds a bounded slice and finishes seeding on later polls', () => {
  const calls = stubImap()
  // 6 selectable mailboxes, 5 seeded per call. Seeding all of them here would
  // be one TCP+TLS+LOGIN each inside a single invocation, and a baseline that
  // throws leaves the mount on the full-walk path forever.
  const base = call('get_changes', treeMount(), { since_token: null, baseline_only: true })
  assert.equal(calls.fetchSince.length, 5, 'bounded by the slice, not by the mailbox count')
  const seeded = JSON.parse(base.next_token.slice('rsn-imaptree-1:'.length))
  assert.equal(seeded.s, 1, 'the cursor remembers that seeding is unfinished')
  assert.equal(
    Object.values(seeded.m).filter((v) => v === null).length,
    1,
    'exactly the one mailbox the slice could not reach'
  )

  // The ordinary polls finish it, and emit nothing while they do: "from now on"
  // must not become "import this mailbox's whole history".
  let token = base.next_token
  const items = []
  for (let i = 0; i < 4; i++) {
    const page = call('get_changes', treeMount(), { since_token: token })
    items.push(...page.items)
    token = page.next_token
    if (page.has_more === false) break
  }
  assert.deepEqual(msgs(items), [], 'a seeding poll emits no message')
  const done = JSON.parse(token.slice('rsn-imaptree-1:'.length))
  assert.equal(done.s, 0, 'seeding is over once every mailbox has a watermark')
  assert.equal(Object.values(done.m).every((v) => v && v.uid === 5), true)
})

test('the per-mailbox page is the whole item budget, not a fraction of it', () => {
  const calls = stubImap()
  // client.rs keeps the NEWEST `limit` uids above the cursor and reports the
  // highest as the new watermark, so everything past `limit` is stepped over
  // and can never be asked for again. Dividing the budget by the slice size
  // lowered that cliff fivefold for nothing.
  call('get_changes', treeMount({ max_items_per_sync: 200 }), { since_token: null })
  assert.ok(calls.fetchSince.length > 0)
  for (const c of calls.fetchSince) assert.equal(c.limit, 200)
})

test('get resolves the mailbox from the id and answers with the id it was given', () => {
  stubImap()
  const out = call('get', treeMount(), { item_id: 'INBOX.Archive|100.5' })
  // Rebuilt from the message alone this is the bare "5", which is a different
  // node on a tree mount.
  assert.equal(out.external_id, 'INBOX.Archive|100.5')
  assert.equal(out.metadata.mailbox, 'INBOX.Archive')
})

// ---- UIDVALIDITY ----------------------------------------------------------

/**
 * A mailbox whose UIDVALIDITY we control, holding exactly one message.
 * "Restored from backup": the server changes UIDVALIDITY and hands the same
 * message back under a fresh UID counted from 1.
 */
function stubResettable(world) {
  const calls = []
  globalThis.raisin = {
    imap: {
      listMailboxes: () => BOXES,
      fetchSince(conn, sinceUid, opts) {
        calls.push({ mailbox: opts.mailbox, sinceUid })
        const mine = opts.mailbox === world.mailbox && sinceUid < world.uid
        const messages = mine
          ? [
              {
                uid: world.uid,
                subject: 'Restored from backup',
                from: 'ada@example.org',
                to: 'bob@example.org',
                date: '2026-01-01T00:00:00Z',
                flags: [],
                message_id: '<one@example.org>',
                headers: {},
              },
            ]
          : []
        return {
          messages,
          highestUid: messages.length ? world.uid : sinceUid,
          uidvalidity: world.uidvalidity,
        }
      },
      fetchMessage: (conn, uid, opts) => ({
        uid,
        subject: 'Restored from backup',
        flags: [],
        text: `body of ${opts.mailbox}#${uid}`,
      }),
    },
    email: { providers: () => ({ enabled: false, providers: [] }) },
  }
  return calls
}

test('a UIDVALIDITY reset moves the id and the path TOGETHER', () => {
  const world = { mailbox: 'INBOX.Archive', uidvalidity: 100, uid: 5 }
  stubResettable(world)
  const mount = treeMount()

  // The walk is the same on both sides of the reset: it enumerates mailboxes,
  // and a reset does not rename one.
  const folders = walk(mount)
  assert.equal(folders.get('INBOX.Archive'), 'Archive')

  const before = drain(mount).items.find((c) => c.item.metadata.mailbox === 'INBOX.Archive')
  assert.equal(before.item.external_id, 'INBOX.Archive|100.5')
  assert.equal(before.relative_path, 'Archive/100.5')

  // THE RESET. New UID space, same message, UID counted from 1 again.
  world.uidvalidity = 200
  world.uid = 1
  stubResettable(world)
  const after = drain(mount).items.find((c) => c.item.metadata.mailbox === 'INBOX.Archive')
  assert.equal(after.item.external_id, 'INBOX.Archive|200.1')
  assert.equal(after.relative_path, 'Archive/200.1')

  // THE BUG THIS PINS. The id changed (it always did — it carries the
  // uidvalidity), so the materializer matches nothing on __external_id and
  // CREATES. The path used to be the bare uid, "Archive/5" then "Archive/1",
  // which after a reset from a longer history lands the new node on a path an
  // old node still occupies — the old ones are not removed, because a tree
  // mount runs reconcile_deletes:false and the walk never enumerates messages.
  assert.notEqual(after.item.external_id, before.item.external_id)
  assert.notEqual(after.relative_path, before.relative_path, 'the path must move with the id')

  // And the folder half of the path is still the byte-identical prefix the walk
  // built, on both sides of the reset.
  for (const change of [before, after]) {
    const rel = change.relative_path
    assert.equal(rel.slice(0, rel.lastIndexOf('/')), folders.get('INBOX.Archive'))
  }
})

test('the path leaf IS the tail of the external_id, for every message', () => {
  // The invariant behind the test above, asserted over the whole fixture tree so
  // the two can never be respelled apart again.
  stubImap()
  const { items } = drain(treeMount())
  assert.ok(msgs(items).length > 0)
  for (const change of msgs(items)) {
    const id = change.item.external_id
    const leaf = change.relative_path.split('/').pop()
    assert.equal(id, `${change.item.metadata.mailbox}|${leaf}`)
    // and it is fetchable under that id
    assert.equal(
      call('get_content', treeMount(), { item_id: id }).content,
      `body of ${change.item.metadata.mailbox}#${change.item.metadata.uid}`
    )
  }
})

test('an id minted before the leaf was unified still fetches', () => {
  // Tree mode is new, but a build in flight may have written "|100:5". Resolving
  // that to NaN would make the node permanently unopenable.
  stubImap()
  assert.equal(
    call('get_content', treeMount(), { item_id: 'INBOX.Archive|100:5' }).content,
    'body of INBOX.Archive#5'
  )
})

// ---- the folder hierarchy must survive the ephemeral sweep -----------------

/**
 * A tree mount is `ephemeral: true` + `ttl_seconds: 86400` because IMAP has no
 * EXPUNGE feed. `ephemeral::cleanup_expired` has no is_folder exemption, and
 * the walk that made the folder nodes runs exactly once (after
 * backfill_complete only get_changes is called). So if the delta does not carry
 * folders, 24h after the backfill every raisin:Folder under the mount is
 * deleted, the next message re-creates its ancestor through upsert_deep_node as
 * a stub with NO __mount_id, and from then on every walk that stages the real
 * folder is skipped as "foreign node occupies target path".
 */
const ephemeralTreeMount = () =>
  treeMount({ ephemeral: true, ttl_seconds: 86400 })

test('the delta carries the folder tree, or the ephemeral sweep erases it', () => {
  stubImap()
  const mount = ephemeralTreeMount()
  const folders = walk(mount)
  const { items } = drain(mount)
  const emitted = dirs(items)

  assert.ok(emitted.length > 0, 'the delta must re-assert the hierarchy')
  // EXACTLY the walk's set, at EXACTLY the walk's paths. A folder the delta
  // spelled differently would be a second node, not a refresh.
  assert.deepEqual(
    emitted.map((c) => c.item.external_id).sort(),
    [...folders.keys()].sort()
  )
  for (const c of emitted) {
    assert.equal(c.item.is_folder, true)
    assert.equal(c.relative_path, folders.get(c.item.external_id))
  }
  // Including the \Noselect one, which holds no message and would therefore
  // never be re-asserted by a message-driven refresh.
  assert.ok(emitted.some((c) => c.item.external_id === 'INBOX.Archive.Old'))
})

test('the mount root itself emits no folder change', () => {
  // Its relative_path is "", which delta.rs rejects with "no name and no
  // relative_path" — and that is a hard error that fails the whole run, not one
  // skipped item.
  stubImap()
  const { items } = drain(ephemeralTreeMount())
  for (const c of dirs(items)) {
    assert.ok(c.relative_path.length > 0)
    assert.notEqual(c.item.external_id, 'INBOX')
  }
})

test('a folder is emitted before any message that lives inside it', () => {
  // The batch applies in order. A message staged ahead of its own folder is
  // exactly what makes upsert_deep_node mint the un-owned stub.
  stubImap()
  const { items } = drain(ephemeralTreeMount())
  const firstMessage = items.findIndex((c) => !c.item.is_folder)
  const lastFolder = items.map((c) => c.item.is_folder).lastIndexOf(true)
  assert.ok(firstMessage > lastFolder, 'folders precede messages in the page')
})

test('the folder page rides one page per round, not one per rotation slice', () => {
  stubImap()
  const mount = ephemeralTreeMount()
  const { items, pages } = drain(mount)
  assert.ok(pages > 1, '6 selectable mailboxes at 5 per call is more than one page')
  assert.equal(dirs(items).length, 6, 'six mailboxes, emitted once, not once per page')
})

test('a folder etag moves once per third of the TTL, so it outruns the sweep', () => {
  // An UNCHANGED etag is not enough: the materializer's skip-write returns
  // Staged::Skipped WITHOUT re-stamping __synced_at, so a folder re-emitted
  // with the etag the walk stored is a no-op and the node is still swept at
  // ttl_seconds. The etag has to move — three times per window, not on every
  // 300s poll, because each move is a revision and a node:updated event.
  const realNow = Date.now
  try {
    stubImap()
    const mount = ephemeralTreeMount()
    Date.now = () => 1_000_000_000_000
    const t0 = dirs(drain(mount).items).find((c) => c.item.external_id === 'INBOX.Archive')

    // Within the same third-of-a-day bucket: no rewrite.
    Date.now = () => 1_000_000_000_000 + 3600 * 1000
    stubImap()
    const same = dirs(drain(mount).items).find((c) => c.item.external_id === 'INBOX.Archive')
    assert.equal(same.item.etag, t0.item.etag, 'no churn inside a bucket')

    // A third of the TTL later: the node is rewritten, well before 86400s.
    Date.now = () => 1_000_000_000_000 + 30000 * 1000
    stubImap()
    const later = dirs(drain(mount).items).find((c) => c.item.external_id === 'INBOX.Archive')
    assert.notEqual(later.item.etag, t0.item.etag, 'the folder is re-stamped before it expires')

    // A mount that expires nothing pays no churn at all, and its folder etag is
    // byte-identical to the one the walk publishes.
    stubImap()
    const plain = treeMount()
    const walkEtag = call('list', plain, { folder_id: 'INBOX' }).items.find(
      (i) => i.external_id === 'INBOX.Archive'
    ).etag
    const deltaEtag = dirs(drain(plain).items).find(
      (c) => c.item.external_id === 'INBOX.Archive'
    ).item.etag
    assert.equal(deltaEtag, walkEtag)
    assert.equal(walkEtag, 'mbx:INBOX.Archive|\\HasNoChildren')
  } finally {
    Date.now = realNow
  }
})

test('the walk and the delta spell a folder etag identically', () => {
  // Two spellings would rewrite every folder on whichever run changed hands.
  stubImap()
  const mount = ephemeralTreeMount()
  const listed = new Map()
  const stack = ['INBOX']
  while (stack.length) {
    for (const i of call('list', mount, { folder_id: stack.pop() }).items) {
      listed.set(i.external_id, i.etag)
      stack.push(i.external_id)
    }
  }
  for (const c of dirs(drain(mount).items)) {
    assert.equal(c.item.etag, listed.get(c.item.external_id))
  }
})

test('the folder page never eats a small mount’s message budget', () => {
  // items.length was the loop break; with folders in the same array a mount
  // whose max_items_per_sync is smaller than its mailbox count would emit
  // folders, no mail, and still advance the rotation past the mail.
  stubImap()
  // 6 folders emitted on the first page against a budget of 3.
  const mount = treeMount({ ephemeral: true, ttl_seconds: 86400, max_items_per_sync: 3 })
  const { items } = drain(mount)
  assert.equal(dirs(items).length, 6)
  assert.equal(msgs(items).length, 6, 'every message still arrives')
})
