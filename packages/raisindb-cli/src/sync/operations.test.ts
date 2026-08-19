import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import fs from 'fs';
import os from 'os';
import path from 'path';

vi.mock('../auth.js', () => ({
  getToken: () => 'test-token',
}));

// Imported after the mock so operations.ts picks up the stubbed getToken.
const { pullFile, pushFile, processLocalChanges } = await import('./operations.js');

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

  describe('pushFile creates missing nodes', () => {
    const roleYaml = [
      'node_type: raisin:Role',
      'properties:',
      '  role_id: "content_editor"',
      '  name: "Content Editor"',
      '',
    ].join('\n');

    /** PUT 404s (node absent), POST to the parent succeeds. */
    const missingThenCreated = () =>
      vi
        .fn()
        .mockResolvedValueOnce({ ok: false, status: 404, statusText: 'Not Found' })
        .mockResolvedValueOnce({ ok: true, status: 201 });

    it('creates a flat {name}.yaml node instead of failing with 404', async () => {
      // The regression: this fallback was gated on `filename === '.node.yaml'`,
      // so a flat role file 404'd on every push, forever.
      write('_raisin__access_control/roles/editor.yaml', roleYaml);

      const fetchMock = missingThenCreated();
      vi.stubGlobal('fetch', fetchMock);

      const result = await pushFile(
        '_raisin__access_control/roles/editor.yaml',
        baseOptions()
      );

      expect(result.success).toBe(true);
      expect(fetchMock).toHaveBeenCalledTimes(2);

      const [createUrl, createInit] = fetchMock.mock.calls[1];
      expect(createInit.method).toBe('POST');
      // POSTed to the PARENT of the node, not the node itself.
      expect(createUrl).toContain('/raisin:access_control/roles');
      expect(createUrl).not.toContain('/roles/editor');

      const body = JSON.parse(createInit.body);
      expect(body.name).toBe('editor');
      expect(body.path).toBe('/roles/editor');
      expect(body.node_type).toBe('raisin:Role');
      // Display name must NOT relocate the node.
      expect(body.properties.name).toBe('Content Editor');
      expect(body.properties.role_id).toBe('content_editor');
    });

    it('creates a .node.yml node (the .yml spelling was never handled)', async () => {
      write('_raisin__access_control/roles/viewer/.node.yml', roleYaml);

      const fetchMock = missingThenCreated();
      vi.stubGlobal('fetch', fetchMock);

      const result = await pushFile(
        '_raisin__access_control/roles/viewer/.node.yml',
        baseOptions()
      );

      expect(result.success).toBe(true);
      const body = JSON.parse(fetchMock.mock.calls[1][1].body);
      expect(body.name).toBe('viewer');
      expect(body.path).toBe('/roles/viewer');
    });

    it('still creates the .node.yaml form', async () => {
      write('_raisin__access_control/roles/author/.node.yaml', roleYaml);

      const fetchMock = missingThenCreated();
      vi.stubGlobal('fetch', fetchMock);

      const result = await pushFile(
        '_raisin__access_control/roles/author/.node.yaml',
        baseOptions()
      );

      expect(result.success).toBe(true);
      const body = JSON.parse(fetchMock.mock.calls[1][1].body);
      // The folder name wins; `properties.name: "Content Editor"` must not.
      expect(body.name).toBe('author');
      expect(body.path).toBe('/roles/author');
    });

    it('honours a top-level name: but ignores properties.name', async () => {
      write(
        'ws/things/thing.yaml',
        'name: renamed\nnode_type: test:Thing\nproperties:\n  name: "Display Only"\n'
      );

      const fetchMock = missingThenCreated();
      vi.stubGlobal('fetch', fetchMock);

      const result = await pushFile('ws/things/thing.yaml', baseOptions());

      expect(result.success).toBe(true);
      const [putUrl] = fetchMock.mock.calls[0];
      // The PUT url and the created node must agree on the name, or the next
      // push 404s again.
      expect(putUrl).toContain('/things/renamed');
      const body = JSON.parse(fetchMock.mock.calls[1][1].body);
      expect(body.name).toBe('renamed');
      expect(body.path).toBe('/things/renamed');
    });

    it('keeps a required name property when the file has no properties block', async () => {
      // A flat file IS its properties. `name` here must stay a property --
      // raisin:Role requires one -- and must NOT relocate the node.
      write(
        '_raisin__access_control/roles/editor.yaml',
        'node_type: raisin:Role\nrole_id: editor\nname: Editor\n'
      );

      const fetchMock = missingThenCreated();
      vi.stubGlobal('fetch', fetchMock);

      const result = await pushFile(
        '_raisin__access_control/roles/editor.yaml',
        baseOptions()
      );

      expect(result.success).toBe(true);
      const [putUrl] = fetchMock.mock.calls[0];
      expect(putUrl).toContain('/roles/editor');
      expect(putUrl).not.toContain('/roles/Editor');

      const body = JSON.parse(fetchMock.mock.calls[1][1].body);
      expect(body.name).toBe('editor');
      expect(body.path).toBe('/roles/editor');
      expect(body.properties.name).toBe('Editor');
      expect(body.properties.role_id).toBe('editor');
      expect(body.properties.node_type).toBeUndefined();
    });

    it('does not strip node_type into properties on a flat file', async () => {
      write('ws/things/widget.yaml', 'node_type: test:Widget\ncolour: red\n');

      const fetchMock = vi.fn().mockResolvedValue({ ok: true, status: 200 });
      vi.stubGlobal('fetch', fetchMock);

      await pushFile('ws/things/widget.yaml', baseOptions());

      const body = JSON.parse(fetchMock.mock.calls[0][1].body);
      expect(body.properties.colour).toBe('red');
      expect(body.properties.node_type).toBeUndefined();
    });
  });

  describe('pushFile: asset metadata', () => {
    const metaYaml = [
      'node_type: raisin:Asset',
      'title: Company Logo',
      'description: Primary brand mark',
      'properties:',
      '  alt_text: The logo',
      '',
    ].join('\n');

    /** GET returns the live asset node, PUT succeeds. */
    const assetOnServer = (props: Record<string, unknown>) =>
      vi
        .fn()
        .mockResolvedValueOnce({
          ok: true,
          status: 200,
          json: async () => ({ properties: props }),
        })
        .mockResolvedValueOnce({ ok: true, status: 200 });

    it('applies title/description/properties to the sibling asset node', async () => {
      write('launchpad/images/.node.logo.png.yaml', metaYaml);

      const fetchMock = assetOnServer({
        title: 'logo.png',
        file: { key: 'blob/abc', name: 'logo.png' },
        file_size: 1234,
      });
      vi.stubGlobal('fetch', fetchMock);

      const result = await pushFile(
        'launchpad/images/.node.logo.png.yaml',
        baseOptions()
      );

      expect(result.success).toBe(true);

      // Targets the ASSET node, not a node named after the metadata file.
      const [getUrl] = fetchMock.mock.calls[0];
      expect(getUrl).toContain('/launchpad/images/logo.png');
      expect(getUrl).not.toContain('.node.');

      const body = JSON.parse(fetchMock.mock.calls[1][1].body);
      expect(body.properties.title).toBe('Company Logo');
      expect(body.properties.description).toBe('Primary brand mark');
      expect(body.properties.alt_text).toBe('The logo');
    });

    it('preserves the binary-derived properties', async () => {
      // The whole point of the read-modify-write: PUT replaces ALL properties,
      // so dropping `file` here would unbind the asset from its bytes.
      write('launchpad/images/.node.logo.png.yaml', metaYaml);

      const fetchMock = assetOnServer({
        file: { key: 'blob/abc', name: 'logo.png' },
        file_type: 'image/png',
        file_size: 1234,
        content_hash: 'deadbeef',
      });
      vi.stubGlobal('fetch', fetchMock);

      await pushFile('launchpad/images/.node.logo.png.yaml', baseOptions());

      const body = JSON.parse(fetchMock.mock.calls[1][1].body);
      expect(body.properties.file).toEqual({ key: 'blob/abc', name: 'logo.png' });
      expect(body.properties.file_type).toBe('image/png');
      expect(body.properties.file_size).toBe(1234);
      expect(body.properties.content_hash).toBe('deadbeef');
    });

    it('never lets an authored file/content_hash overwrite the real one', async () => {
      write(
        'launchpad/images/.node.logo.png.yaml',
        'title: X\nproperties:\n  file: bogus\n  content_hash: bogus\n'
      );

      const fetchMock = assetOnServer({
        file: { key: 'blob/real' },
        content_hash: 'realhash',
      });
      vi.stubGlobal('fetch', fetchMock);

      await pushFile('launchpad/images/.node.logo.png.yaml', baseOptions());

      const body = JSON.parse(fetchMock.mock.calls[1][1].body);
      expect(body.properties.file).toEqual({ key: 'blob/real' });
      expect(body.properties.content_hash).toBe('realhash');
    });

    it('defaults the title to the filename', async () => {
      write('launchpad/images/.node.logo.png.yaml', 'node_type: raisin:Asset\n');

      const fetchMock = assetOnServer({ file: { key: 'blob/abc' } });
      vi.stubGlobal('fetch', fetchMock);

      await pushFile('launchpad/images/.node.logo.png.yaml', baseOptions());

      const body = JSON.parse(fetchMock.mock.calls[1][1].body);
      expect(body.properties.title).toBe('logo.png');
    });

    it('explains itself when the asset node does not exist yet', async () => {
      write('launchpad/images/.node.logo.png.yaml', metaYaml);

      const fetchMock = vi
        .fn()
        .mockResolvedValue({ ok: false, status: 404, statusText: 'Not Found' });
      vi.stubGlobal('fetch', fetchMock);

      const result = await pushFile(
        'launchpad/images/.node.logo.png.yaml',
        baseOptions()
      );

      expect(result.success).toBe(false);
      expect(result.details).toContain('logo.png');
      // Must not have attempted a write.
      expect(fetchMock).toHaveBeenCalledTimes(1);
    });
  });

  describe('processLocalChanges ordering', () => {
    it('pushes a binary before the metadata that describes it', async () => {
      // The watcher reports in arbitrary order. Metadata updates a node the
      // BINARY creates, so leading with it fails "no asset node yet" for no
      // reason other than event order.
      write('launchpad/images/logo.png', 'PNGDATA');
      write('launchpad/images/.node.logo.png.yaml', 'title: Logo\n');

      const seen: string[] = [];
      vi.stubGlobal(
        'fetch',
        vi.fn(async (url: string, init?: { method?: string }) => {
          seen.push(`${init?.method || 'GET'} ${url}`);
          return { ok: true, status: 200, json: async () => ({ properties: {} }) };
        })
      );

      // Deliberately listed metadata-first, the order that used to break.
      await processLocalChanges(
        [
          { type: 'add', path: 'launchpad/images/.node.logo.png.yaml' },
          { type: 'add', path: 'launchpad/images/logo.png' },
        ] as never,
        baseOptions()
      );

      // The binary goes up as a multipart POST to ?override_existing=true;
      // the metadata is the GET/PUT pair on the asset node.
      const upload = seen.findIndex(
        (c) => c.startsWith('POST') && c.includes('override_existing')
      );
      const metaRead = seen.findIndex(
        (c) => c.startsWith('GET') && !c.includes('override_existing')
      );
      const metaWrite = seen.findIndex((c) => c.startsWith('PUT'));

      expect(upload).toBeGreaterThanOrEqual(0);
      expect(metaRead).toBeGreaterThan(upload);
      expect(metaWrite).toBeGreaterThan(metaRead);
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
