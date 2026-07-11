import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import fs from 'fs';
import path from 'path';
import os from 'os';
import yaml from 'yaml';
import { createAdapter } from './create.js';
import { adapterFiles } from '../templates/adapter.js';

function makeTempDir(): string {
  return fs.mkdtempSync(path.join(os.tmpdir(), 'raisindb-adapter-test-'));
}

describe('adapterFiles', () => {
  it('emits the expected file tree with encoded system workspace path', () => {
    const paths = adapterFiles({
      name: 'dropbox',
      provider: 'dropbox',
      description: 'test',
    }).map((f) => f.path);

    expect(paths).toContain('manifest.yaml');
    expect(paths).toContain('README.md');
    expect(paths).toContain('content/functions/adapters/dropbox/index.js');
    expect(paths).toContain('content/functions/adapters/dropbox/.node.yaml');
    expect(paths).toContain(
      'content/_raisin__system/integrations/dropbox/.node.yaml'
    );
  });

  it('produces a manifest that parses and is a non-builtin integration package', () => {
    const files = adapterFiles({
      name: 'box',
      provider: 'box',
      description: 'Box connector',
    });
    const manifestEntry = files.find((f) => f.path === 'manifest.yaml')!;
    const manifest = yaml.parse(manifestEntry.content);

    expect(manifest.name).toBe('box-adapter');
    expect(manifest.category).toBe('integrations');
    expect(manifest.builtin).toBe(false);
    expect(manifest.provides.functions).toContain('/adapters/box');
    expect(manifest.provides.content).toContain(
      'raisin:system/integrations/box'
    );
  });

  it('generates an integration template that is disabled and ships no client secret', () => {
    const files = adapterFiles({
      name: 'box',
      provider: 'box-provider',
      description: 'x',
    });
    const node = files.find((f) =>
      f.path.endsWith('integrations/box/.node.yaml')
    )!;
    const parsed = yaml.parse(node.content);

    expect(parsed.node_type).toBe('raisin:Integration');
    expect(parsed.properties.enabled).toBe(false);
    expect(parsed.properties.provider_type).toBe('box-provider');
    expect(parsed.properties.adapter_function).toBe('/adapters/box');
    // Frozen invariant: no secret material ships in the package. (Comments may
    // reference the field name; assert no actual secret value is set.)
    expect(parsed.properties.client_secret_encrypted).toBeUndefined();
    expect(parsed.properties.client_id).toBeUndefined();
    expect(parsed.properties.connected_accounts).toBeUndefined();
  });

  it('generates an adapter node with a raised timeout and restrictive network policy', () => {
    const files = adapterFiles({ name: 'box', provider: 'box', description: 'x' });
    const node = files.find((f) =>
      f.path.endsWith('adapters/box/.node.yaml')
    )!;
    const parsed = yaml.parse(node.content);

    expect(parsed.properties.resource_limits.timeout_ms).toBe(120000);
    expect(parsed.properties.entry_file).toBe('index.js:handler');
    // Placeholder host only - not the real provider, forcing the author to widen it.
    expect(parsed.properties.network_policy.allowed_urls).toContain(
      'https://api.example.com/**'
    );
  });

  it('generated adapter index.js exposes single-arg handler with conservative capabilities', () => {
    const files = adapterFiles({ name: 'box', provider: 'box', description: 'x' });
    const index = files.find((f) => f.path.endsWith('adapters/box/index.js'))!.content;

    expect(index).toContain('function handler(input)');
    expect(index).toContain('can_read: true');
    expect(index).toContain('can_write: false');
    expect(index).toContain('supports_changes: false');
    // Error convention: Error with a `code` property.
    expect(index).toContain('e.code = code');
  });
});

describe('createAdapter', () => {
  let tmpDir: string;
  let logSpy: ReturnType<typeof vi.spyOn>;

  beforeEach(() => {
    tmpDir = makeTempDir();
    logSpy = vi.spyOn(console, 'log').mockImplementation(() => {});
  });

  afterEach(() => {
    fs.rmSync(tmpDir, { recursive: true, force: true });
    logSpy.mockRestore();
  });

  it('renders a scaffold to --dir and the manifest parses', async () => {
    const dir = path.join(tmpDir, 'out');
    await createAdapter('dropbox', { dir });

    const manifestPath = path.join(dir, 'manifest.yaml');
    expect(fs.existsSync(manifestPath)).toBe(true);
    const manifest = yaml.parse(fs.readFileSync(manifestPath, 'utf-8'));
    expect(manifest.name).toBe('dropbox-adapter');

    expect(
      fs.existsSync(path.join(dir, 'content/functions/adapters/dropbox/index.js'))
    ).toBe(true);
    expect(
      fs.existsSync(
        path.join(dir, 'content/_raisin__system/integrations/dropbox/.node.yaml')
      )
    ).toBe(true);
  });

  it('honors --provider for the integration provider_type', async () => {
    const dir = path.join(tmpDir, 'out2');
    await createAdapter('mydrive', { dir, provider: 'acme-cloud' });

    const node = yaml.parse(
      fs.readFileSync(
        path.join(dir, 'content/_raisin__system/integrations/mydrive/.node.yaml'),
        'utf-8'
      )
    );
    expect(node.properties.provider_type).toBe('acme-cloud');
  });

  it('rejects invalid adapter names', async () => {
    await expect(
      createAdapter('Not Valid!', { dir: path.join(tmpDir, 'x') })
    ).rejects.toThrow(/Invalid adapter name/);
  });

  it('refuses to overwrite an existing scaffold', async () => {
    const dir = path.join(tmpDir, 'out3');
    await createAdapter('box', { dir });
    await expect(createAdapter('box', { dir })).rejects.toThrow(/already exists/);
  });
});
