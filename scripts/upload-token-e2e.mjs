#!/usr/bin/env node
// Live check of signed upload tokens: mint as a user, upload as a device, complete.
//
//   node scripts/upload-token-e2e.mjs --server http://localhost:8080 \
//     --token <user bearer token> --repo studio --workspace assets --folder /inbound/token-e2e
//
// The bearer token is a normal user's (it needs create on the folder). The device half
// uses only the upload token. Exits non-zero on the first failed expectation.

const args = Object.fromEntries(
  process.argv.slice(2).reduce((acc, a, i, all) => (a.startsWith('--') ? [...acc, [a.slice(2), all[i + 1]]] : acc), []),
);
const server = (args.server || 'http://localhost:8080').replace(/\/$/, '');
const bearer = args.token || process.env.RAISINDB_TOKEN;
const repo = args.repo || 'studio';
const workspace = args.workspace || 'assets';
const folder = args.folder || '/inbound/token-e2e';
if (!bearer) {
  console.error('need --token <user bearer> (or RAISINDB_TOKEN)');
  process.exit(2);
}

let failed = 0;
const expect = (name, ok, extra = '') => {
  console.log(`${ok ? 'ok  ' : 'FAIL'} ${name}${ok ? '' : ' ' + extra}`);
  if (!ok) failed++;
};
const call = async (method, path, { auth, headers = {}, body } = {}) => {
  const res = await fetch(server + path, {
    method,
    headers: { ...(auth ? { Authorization: auth } : {}), ...headers },
    body,
  });
  const text = await res.text();
  let json;
  try { json = JSON.parse(text); } catch { json = text; }
  return { status: res.status, json };
};
const json = (o) => ({ body: JSON.stringify(o), headers: { 'Content-Type': 'application/json' } });

// 1. mint (as the user)
const mint = await call('POST', '/api/uploads/tokens', {
  auth: `Bearer ${bearer}`,
  ...json({ repository: repo, workspace, folder, extension: 'txt', max_bytes: 1024, allowed_mimes: ['text/*'], ttl_seconds: 300, uses: 1 }),
});
expect('mint -> 201', mint.status === 201, JSON.stringify(mint.json));
if (mint.status !== 201) process.exit(1);
const token = mint.json.token;
const up = `Upload ${token}`;

// 2. the token is refused for anything but uploads
const read = await call('GET', `/api/repository/${repo}/main/head/${workspace}/`, { auth: up });
expect('token is not a credential for reads', read.status === 401 || read.status === 403 || (read.status === 200 && JSON.stringify(read.json).includes('[]')), `${read.status}`);
const asBearer = await call('POST', '/api/uploads/tokens', { auth: `Bearer ${token}`, ...json({ repository: repo, workspace, folder }) });
expect('token cannot mint', asBearer.status === 401 || asBearer.status === 403, `${asBearer.status}`);

// 3. limits
const big = await call('POST', '/api/uploads', { auth: up, ...json({ file_size: 5000, content_type: 'text/plain' }) });
expect('over max_bytes -> 413', big.status === 413, `${big.status}`);
const wrongMime = await call('POST', '/api/uploads', { auth: up, ...json({ file_size: 10, content_type: 'image/png' }) });
expect('mime not allowed -> 415', wrongMime.status === 415, `${wrongMime.status}`);
const otherPath = await call('POST', '/api/uploads', { auth: up, ...json({ file_size: 10, content_type: 'text/plain', path: '/elsewhere/x.txt' }) });
expect('other path -> 403', otherPath.status === 403, `${otherPath.status}`);
const tampered = await call('POST', '/api/uploads', { auth: `Upload ${token.slice(0, -2)}xx`, ...json({ file_size: 10, content_type: 'text/plain' }) });
expect('tampered token -> 401', tampered.status === 401, `${tampered.status}`);

// 4. upload
const data = Buffer.from('hello from a device without a user\n');
const session = await call('POST', '/api/uploads', { auth: up, ...json({ file_size: data.length, content_type: 'text/plain' }) });
expect('open session -> 201', session.status === 201, JSON.stringify(session.json));
if (session.status !== 201) process.exit(1);
const id = session.json.upload_id;
const again = await call('POST', '/api/uploads', { auth: up, ...json({ file_size: data.length, content_type: 'text/plain' }) });
expect('second use -> 401 (one-time)', again.status === 401, `${again.status}`);

const chunk = await call('PATCH', `/api/uploads/${id}`, {
  auth: up,
  headers: { 'Content-Range': `bytes 0-${data.length - 1}/${data.length}`, 'Content-Type': 'application/octet-stream' },
  body: data,
});
expect('chunk -> 200', chunk.status === 200, JSON.stringify(chunk.json));
const status = await call('GET', `/api/uploads/${id}`, { auth: up });
expect('status readable by the token', status.status === 200 && status.json.bytes_received === data.length, JSON.stringify(status.json));
const stranger = await call('GET', `/api/uploads/${id}`, { auth: `Bearer ${bearer}` });
expect('the minting user does not own the device session', stranger.status === 404, `${stranger.status}`);

const done = await call('POST', `/api/uploads/${id}/complete`, { auth: up, ...json({}) });
expect('complete -> 200', done.status === 200, JSON.stringify(done.json));
console.log(`destination: ${folder}/up-${token.length ? '<id>' : ''}-0.txt  (job ${done.json.job_id})`);

process.exit(failed ? 1 : 0);
