import fs from 'fs';
import os from 'os';
import path from 'path';
import yaml from 'yaml';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import {
  hasSkipPaths,
  loadPackagePolicy,
  parsePackagePolicy,
  policyForPath,
  type PackagePolicy,
} from './package-policy.js';

/** A customer's policy, verbatim in shape: editors own everything by default. */
const CUSTOMER = `
defaults:
  mode: skip
filters:
  - root: /functions
    mode: replace
  - root: /apps/local
    mode: replace
  - root: /automations
    mode: replace
`;

function parse(text: string): PackagePolicy {
  const policy = parsePackagePolicy(yaml.parse(text));
  if (!policy) throw new Error('not a policy');
  return policy;
}

describe('policyForPath (port of SyncConfig::get_mode_and_source_for_path)', () => {
  const policy = parse(CUSTOMER);

  it('editorial content falls to the skip default', () => {
    expect(policyForPath(policy, 'stories', 'baden-rhinos/teams/herren')).toEqual({
      mode: 'skip',
      source: null,
    });
    expect(policyForPath(policy, 'assets', '/website/teams/logo.jpg').mode).toBe('skip');
  });

  it('package-owned roots are replaced', () => {
    expect(policyForPath(policy, 'functions', 'lib/rhinos/x')).toEqual({
      mode: 'replace',
      source: '/functions',
    });
    expect(policyForPath(policy, 'apps', 'local/people').mode).toBe('replace');
  });

  it('matching is a plain prefix test, as on the server', () => {
    expect(policyForPath(policy, 'apps', 'localX').mode).toBe('replace');
    expect(policyForPath(policy, 'apps', 'libs/other').mode).toBe('skip');
  });

  it('the LAST matching filter wins', () => {
    const p = parse(`
defaults: { mode: replace }
filters:
  - root: /stories
    mode: skip
  - root: /stories/site/generated
    mode: replace
`);
    expect(policyForPath(p, 'stories', 'site/generated/a').mode).toBe('replace');
    expect(policyForPath(p, 'stories', 'site/about').mode).toBe('skip');

    const reversed = parse(`
filters:
  - root: /stories/site/generated
    mode: replace
  - root: /stories
    mode: skip
`);
    expect(policyForPath(reversed, 'stories', 'site/generated/a').mode).toBe('skip');
  });

  it('a filter without a mode falls through, and the default default is replace', () => {
    const p = parse(`
filters:
  - root: /stories
    mode: skip
  - root: /stories/x
`);
    expect(policyForPath(p, 'stories', 'x/y').mode).toBe('skip');
    expect(policyForPath(p, 'tags', 'a').mode).toBe('replace');
  });
});

describe('parsePackagePolicy / loadPackagePolicy', () => {
  let dir: string;
  beforeEach(() => {
    dir = fs.mkdtempSync(path.join(os.tmpdir(), 'pkg-policy-'));
  });
  afterEach(() => fs.rmSync(dir, { recursive: true, force: true }));

  it('a legacy CLI config of the same name is not a policy', () => {
    expect(parsePackagePolicy({ server: 'http://x', repository: 'r' })).toBeNull();
  });

  it('loads the policy beside manifest.yaml', () => {
    fs.writeFileSync(path.join(dir, '.raisin-sync.yaml'), CUSTOMER);
    const policy = loadPackagePolicy(dir);
    expect(policy?.defaults.mode).toBe('skip');
    expect(hasSkipPaths(policy)).toBe(true);
  });

  it('no file, no policy', () => {
    expect(loadPackagePolicy(dir)).toBeNull();
    expect(hasSkipPaths(null)).toBe(false);
  });
});
