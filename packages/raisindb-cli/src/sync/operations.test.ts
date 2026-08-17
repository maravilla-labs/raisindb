import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import fs from 'fs';
import os from 'os';
import path from 'path';

vi.mock('../auth.js', () => ({
  getToken: () => 'test-token',
}));

// Imported after the mock so operations.ts picks up the stubbed getToken.
const { pullFile, pushFile } = await import('./operations.js');

const config = {
  version: 1,
  server: 'http://localhost:8080',
  repository: 'studio',
  branch: 'main',
  remote_path: '/',
  conflict_strategy: 'prompt' as const,
  ignore: [],
};

describe('sync operations and {env:...} tokens', () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'raisindb-sync-ops-'));
  });

  afterEach(() => {
    fs.rmSync(tmpDir, { recursive: true, force: true });
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  const write = (relPath: string, content: string) => {
    const full = path.join(tmpDir, relPath);
    fs.mkdirSync(path.dirname(full), { recursive: true });
    fs.writeFileSync(full, content, 'utf-8');
  };

  const baseOptions = () => ({
    packageDir: tmpDir,
    contentBase: tmpDir,
    config,
  });

  describe('pushFile', () => {
    it('sends resolved values to the server', async () => {
      write('stories/site/.node.yaml', 'properties:\n  base_url: "{env:PREVIEW_SERVER}"\n');

      const fetchMock = vi.fn().mockResolvedValue({ ok: true, status: 200 });
      vi.stubGlobal('fetch', fetchMock);

      const result = await pushFile('stories/site/.node.yaml', {
        ...baseOptions(),
        env: { values: { PREVIEW_SERVER: 'https://preview.example.ch' }, sources: [] },
      });

      expect(result.success).toBe(true);
      const body = JSON.parse(fetchMock.mock.calls[0][1].body);
      expect(body.properties.base_url).toBe('https://preview.example.ch');
    });

    it('fails the file without contacting the server when a token is unresolved', async () => {
      write('stories/site/.node.yaml', 'properties:\n  base_url: "{env:MISSING_VAR}"\n');

      const fetchMock = vi.fn();
      vi.stubGlobal('fetch', fetchMock);

      const result = await pushFile('stories/site/.node.yaml', {
        ...baseOptions(),
        env: { values: {}, sources: [] },
      });

      expect(result.success).toBe(false);
      expect(result.error).toMatch(/MISSING_VAR/);
      expect(fetchMock).not.toHaveBeenCalled();
    });

    it('applies inline defaults', async () => {
      write(
        'stories/site/.node.yaml',
        'properties:\n  base_url: "{env:PREVIEW_SERVER:-http://localhost:5173}"\n'
      );

      const fetchMock = vi.fn().mockResolvedValue({ ok: true, status: 200 });
      vi.stubGlobal('fetch', fetchMock);

      const result = await pushFile('stories/site/.node.yaml', {
        ...baseOptions(),
        env: { values: {}, sources: [] },
      });

      expect(result.success).toBe(true);
      const body = JSON.parse(fetchMock.mock.calls[0][1].body);
      expect(body.properties.base_url).toBe('http://localhost:5173');
    });
  });

  describe('pullFile', () => {
    const serverResponse = () => ({
      ok: true,
      status: 200,
      json: async () => ({ properties: { base_url: 'http://localhost:5173' } }),
    });

    it('refuses to overwrite a local file containing tokens', async () => {
      const original = 'properties:\n  base_url: "{env:PREVIEW_SERVER}"\n';
      write('stories/site/.node.yaml', original);
      vi.stubGlobal('fetch', vi.fn().mockResolvedValue(serverResponse()));

      const result = await pullFile('stories/site/.node.yaml', baseOptions());

      expect(result.success).toBe(false);
      expect(result.error).toMatch(/\{env:\.\.\.\} tokens/);
      expect(fs.readFileSync(path.join(tmpDir, 'stories/site/.node.yaml'), 'utf-8')).toBe(
        original
      );
    });

    it('overwrites when forced', async () => {
      write('stories/site/.node.yaml', 'properties:\n  base_url: "{env:PREVIEW_SERVER}"\n');
      vi.stubGlobal('fetch', vi.fn().mockResolvedValue(serverResponse()));

      const result = await pullFile('stories/site/.node.yaml', {
        ...baseOptions(),
        force: true,
      });

      expect(result.success).toBe(true);
      const written = fs.readFileSync(path.join(tmpDir, 'stories/site/.node.yaml'), 'utf-8');
      expect(written).not.toContain('{env:');
      expect(written).toContain('http://localhost:5173');
    });

    it('pulls a token-free file normally', async () => {
      write('stories/site/.node.yaml', 'properties:\n  base_url: http://old\n');
      vi.stubGlobal('fetch', vi.fn().mockResolvedValue(serverResponse()));

      const result = await pullFile('stories/site/.node.yaml', baseOptions());

      expect(result.success).toBe(true);
      expect(
        fs.readFileSync(path.join(tmpDir, 'stories/site/.node.yaml'), 'utf-8')
      ).toContain('http://localhost:5173');
    });
  });
});
