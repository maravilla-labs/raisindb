#!/usr/bin/env node
// Audit which repositories of one tenant still depend on open self-registration.
//
// From this release `RepoAuthConfig.allow_registration` defaults to FALSE. A repo
// with no `raisin:RepoAuthConfig` node, or with one that never stated the flag
// (Flightdeck and `raisindb cors` both create the node with only
// `cors_allowed_origins`), used to accept sign-ups and now refuses them:
// `POST /auth/{repo}/register` answers 403 REGISTRATION_DISABLED and an unknown
// address gets no magic link. Logging in as an existing identity is unaffected.
//
// For every repo this prints what the flag resolves to today and after the
// release, and the signals that the repo relies on sign-up: storefront buyers
// (`buyer_home` on ticketing / commerce records), user nodes that hold only the
// roles a sign-up gets (installed shop-like packages are listed for context). It is READ
// ONLY. For each repo marked ACTION it prints the request that opens sign-up
// explicitly.
//
// Usage (the token must be a tenant admin; the server URL selects the tenant):
//   RAISINDB_SERVER=https://<tenant>.rdb.maravilla.cloud RAISINDB_TOKEN=... \
//     node scripts/audit-registration-rollout.mjs [repo ...]

const SERVER = (process.env.RAISINDB_SERVER || '').replace(/\/+$/, '');
const TOKEN = process.env.RAISINDB_TOKEN || '';
if (!SERVER || !TOKEN) {
  console.error('Set RAISINDB_SERVER and RAISINDB_TOKEN (a tenant admin token).');
  process.exit(2);
}
const H = { Authorization: `Bearer ${TOKEN}`, 'Content-Type': 'application/json' };

/** Roles every user may end up with without an admin granting anything. */
const SIGNUP_ROLES = new Set([
  'authenticated_user',
  'viewer',
  'studio_member',
  'studio_profile_writer',
  'studio_draft_writer',
  'studio_ticket_buyer',
]);

async function get(path) {
  const res = await fetch(`${SERVER}${path}`, { headers: H });
  if (res.status === 404) return null;
  if (!res.ok) throw new Error(`GET ${path}: ${res.status} ${(await res.text()).slice(0, 200)}`);
  return res.json();
}

/** Rows of a statement, or null when it cannot run (e.g. the workspace is absent). */
async function sql(repo, statement) {
  const res = await fetch(`${SERVER}/api/sql/${encodeURIComponent(repo)}/main`, {
    method: 'POST',
    headers: H,
    body: JSON.stringify({ sql: statement, params: [] }),
  });
  if (!res.ok) return null;
  return (await res.json()).rows ?? [];
}

async function countWhere(repo, workspace, where) {
  const rows = await sql(repo, `SELECT COUNT(*) AS n FROM "${workspace}" WHERE ${where}`);
  return rows ? Number(rows[0]?.n ?? 0) : null;
}

async function audit(repo) {
  const cfg = await get(
    `/api/repository/${encodeURIComponent(repo)}/main/head/raisin:system/config/repos/${encodeURIComponent(repo)}`,
  );
  const props = cfg?.node_type === 'raisin:RepoAuthConfig' ? cfg.properties ?? {} : null;
  const flag = props?.allow_registration;
  const state =
    props == null ? 'no config node' : typeof flag === 'boolean' ? `explicit ${flag}` : 'node, flag unset';
  const before = typeof flag === 'boolean' ? flag : true;
  const after = typeof flag === 'boolean' ? flag : false;

  const packages = ((await get(`/api/repos/${encodeURIComponent(repo)}/packages`)) ?? [])
    .map((p) => p.name ?? p.id)
    .filter(Boolean);
  const shopPackages = packages.filter((n) => /commerce|ticket|shop|store|member/i.test(n));

  const buyers = {
    ticketing: await countWhere(repo, 'ticketing', "properties->>'buyer_home' IS NOT NULL"),
    commerce: await countWhere(repo, 'commerce', "properties->>'buyer_home' IS NOT NULL"),
  };

  const users =
    (await sql(
      repo,
      `SELECT path, properties->'roles' AS roles FROM "raisin:access_control" WHERE node_type = 'raisin:User'`,
    )) ?? [];
  const real = users.filter((u) => !String(u.path).startsWith('/users/system/'));
  const signupOnly = real.filter((u) => {
    const roles = Array.isArray(u.roles) ? u.roles : typeof u.roles === 'string' ? JSON.parse(u.roles) : [];
    return roles.every((r) => SIGNUP_ROLES.has(r));
  });

  // Installed packages are shown for context only: the Studio package itself
  // ships the commerce and ticketing workspaces, so their presence does not mean
  // a repo sells to the public. Buyers and sign-up-only users do.
  const relies = (buyers.ticketing ?? 0) > 0 || (buyers.commerce ?? 0) > 0 || signupOnly.length > 0;
  const verdict = before && !after && relies ? 'ACTION' : before && !after ? 'closes (no sign-up signal)' : 'no change';

  return { repo, state, before, after, buyers, users: real.length, signupOnly: signupOnly.length, shopPackages, verdict, props };
}

const repos = process.argv.slice(2).length
  ? process.argv.slice(2)
  : ((await get('/api/repositories')) ?? []).map((r) => r.repo_id ?? r.id ?? r.name);

const results = [];
for (const repo of repos) {
  try {
    results.push(await audit(repo));
  } catch (e) {
    results.push({ repo, verdict: `ERROR ${e.message}` });
  }
}

console.table(
  results.map((r) => ({
    repo: r.repo,
    config: r.state,
    'open now': r.before,
    'open after': r.after,
    'ticketing buyers': r.buyers?.ticketing ?? '-',
    'commerce buyers': r.buyers?.commerce ?? '-',
    users: r.users,
    'sign-up-only users': r.signupOnly,
    'shop packages': (r.shopPackages ?? []).join(',') || '-',
    verdict: r.verdict,
  })),
);

for (const r of results.filter((x) => x.verdict === 'ACTION')) {
  const base = `${SERVER}/api/repository/${encodeURIComponent(r.repo)}/main/head/raisin:system`;
  const commit = { message: 'Keep self-registration open', actor: 'registration-rollout' };
  console.log(`\n# ${r.repo}: open self-registration explicitly`);
  if (r.props) {
    // Merge, never replace: the node also carries cors_allowed_origins.
    console.log(
      `curl -X PUT '${base}/config/repos/${r.repo}' -H "Authorization: Bearer $RAISINDB_TOKEN" -H 'Content-Type: application/json' \\\n  -d '${JSON.stringify({ properties: { ...r.props, allow_registration: true }, commit })}'`,
    );
  } else {
    console.log(`# create /config and /config/repos folders first if missing (409 = already there)`);
    for (const [parent, name] of [[`${base}/`, 'config'], [`${base}/config`, 'repos']]) {
      console.log(
        `curl -X POST '${parent}' -H "Authorization: Bearer $RAISINDB_TOKEN" -H 'Content-Type: application/json' \\\n  -d '${JSON.stringify({ name, node_type: 'raisin:Folder', properties: {}, commit })}'`,
      );
    }
    console.log(
      `curl -X POST '${base}/config/repos' -H "Authorization: Bearer $RAISINDB_TOKEN" -H 'Content-Type: application/json' \\\n  -d '${JSON.stringify({ name: r.repo, node_type: 'raisin:RepoAuthConfig', properties: { repo_id: r.repo, allow_registration: true }, commit })}'`,
    );
  }
}
