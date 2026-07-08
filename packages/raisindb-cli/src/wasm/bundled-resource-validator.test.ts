import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import fs from 'fs';
import path from 'path';
import os from 'os';
import { validateBundledResources } from './bundled-resource-validator.js';

function makeTempDir(): string {
  return fs.mkdtempSync(path.join(os.tmpdir(), 'raisindb-bundled-test-'));
}

function write(base: string, relPath: string, content: string | Buffer) {
  const full = path.join(base, relPath);
  fs.mkdirSync(path.dirname(full), { recursive: true });
  fs.writeFileSync(full, content);
}

const ASSET_NODE = (storageKey: string) => `node_type: raisin:Asset
properties:
  title: Logo
  file:
    uuid: mig-logo
    name: ${storageKey}
    mime_type: image/png
    url: ${storageKey}
    is_loaded: true
    is_external: false
    metadata:
      storage_key: ${storageKey}
`;

describe('validateBundledResources', () => {
  let dir: string;
  beforeEach(() => {
    dir = makeTempDir();
  });
  afterEach(() => {
    fs.rmSync(dir, { recursive: true, force: true });
  });

  it('passes when the bundled binary sits beside the node', () => {
    write(dir, 'content/assets/assets/logo/.node.yaml', ASSET_NODE('logo.png'));
    write(dir, 'content/assets/assets/logo/logo.png', Buffer.from([1, 2, 3]));

    const results = validateBundledResources(dir);
    expect(Object.keys(results)).toHaveLength(0);
  });

  it('warns when the referenced bundled binary is missing', () => {
    write(dir, 'content/assets/assets/logo/.node.yaml', ASSET_NODE('logo.png'));
    // no logo.png bundled

    const results = validateBundledResources(dir);
    const entries = Object.values(results);
    expect(entries).toHaveLength(1);
    expect(entries[0].warnings).toHaveLength(1);
    expect(entries[0].warnings[0].error_code).toBe('MISSING_BUNDLED_BINARY');
    expect(entries[0].warnings[0].field_path).toBe('properties.file');
    expect(entries[0].errors).toHaveLength(0); // warning only, does not block
  });

  it('ignores external resources', () => {
    write(
      dir,
      'content/assets/assets/remote/.node.yaml',
      `node_type: raisin:Asset
properties:
  title: Remote
  file:
    uuid: r1
    url: https://cdn.example.com/remote.png
    is_external: true
`
    );
    const results = validateBundledResources(dir);
    expect(Object.keys(results)).toHaveLength(0);
  });

  it('ignores absolute-URL references even when is_external is unset', () => {
    write(
      dir,
      'content/assets/assets/http/.node.yaml',
      `node_type: raisin:Asset
properties:
  file:
    uuid: r2
    url: https://cdn.example.com/x.png
`
    );
    const results = validateBundledResources(dir);
    expect(Object.keys(results)).toHaveLength(0);
  });

  it('does nothing when there is no content/ directory', () => {
    write(dir, 'manifest.yaml', 'name: pkg\nversion: 0.1.0\n');
    expect(validateBundledResources(dir)).toEqual({});
  });
});
