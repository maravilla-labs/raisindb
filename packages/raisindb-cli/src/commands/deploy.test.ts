import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import fs from 'fs';
import os from 'os';
import path from 'path';

// Mock the HTTP api layer so the reconciler can be driven deterministically.
const getWorkspaceRaw = vi.fn();
const putWorkspaceRaw = vi.fn();
vi.mock('../api.js', () => ({
  getWorkspaceRaw: (...a: unknown[]) => getWorkspaceRaw(...a),
  putWorkspaceRaw: (...a: unknown[]) => putWorkspaceRaw(...a),
}));
vi.mock('../config.js', () => ({ getDefaultRepo: () => 'default' }));

import { reconcileWorkspaceAllowedTypes } from './deploy.js';

function pkgDir(): string {
  return fs.mkdtempSync(path.join(os.tmpdir(), 'raisindb-deploy-test-'));
}
function writeWs(dir: string, file: string, body: string) {
  const full = path.join(dir, 'workspaces', file);
  fs.mkdirSync(path.dirname(full), { recursive: true });
  fs.writeFileSync(full, body);
}

const EVENTS_YAML = `name: events
title: Events
allowed_node_types:
  - studio:Folder
  - studio:Event
allowed_root_node_types:
  - studio:Folder
  - studio:Event
`;

describe('reconcileWorkspaceAllowedTypes', () => {
  let dir: string;
  beforeEach(() => {
    dir = pkgDir();
    getWorkspaceRaw.mockReset();
    putWorkspaceRaw.mockReset().mockResolvedValue(undefined);
  });
  afterEach(() => fs.rmSync(dir, { recursive: true, force: true }));

  it('reconciles both allowed-type fields on an existing, drifted workspace', async () => {
    writeWs(dir, 'events.yaml', EVENTS_YAML);
    // Existing workspace: node types partially updated, root types stale.
    getWorkspaceRaw.mockResolvedValue({
      name: 'events',
      description: 'keep me',
      allowed_node_types: ['studio:Folder', 'studio:Event'],
      allowed_root_node_types: ['studio:Folder'],
    });

    await reconcileWorkspaceAllowedTypes(dir, 'studio');

    expect(putWorkspaceRaw).toHaveBeenCalledTimes(1);
    const [repo, name, body] = putWorkspaceRaw.mock.calls[0];
    expect(repo).toBe('studio');
    expect(name).toBe('events');
    expect(body.allowed_root_node_types).toEqual(['studio:Folder', 'studio:Event']);
    expect(body.allowed_node_types).toEqual(['studio:Folder', 'studio:Event']);
    expect(body.description).toBe('keep me'); // other fields preserved
  });

  it('does nothing when the workspace is already up to date', async () => {
    writeWs(dir, 'events.yaml', EVENTS_YAML);
    getWorkspaceRaw.mockResolvedValue({
      name: 'events',
      allowed_node_types: ['studio:Folder', 'studio:Event'],
      allowed_root_node_types: ['studio:Folder', 'studio:Event'],
    });

    await reconcileWorkspaceAllowedTypes(dir, 'studio');
    expect(putWorkspaceRaw).not.toHaveBeenCalled();
  });

  it('skips a workspace that does not exist yet (install seeds new ones)', async () => {
    writeWs(dir, 'events.yaml', EVENTS_YAML);
    getWorkspaceRaw.mockResolvedValue(null);

    await reconcileWorkspaceAllowedTypes(dir, 'studio');
    expect(putWorkspaceRaw).not.toHaveBeenCalled();
  });

  it('is a no-op when there is no workspaces/ directory', async () => {
    await reconcileWorkspaceAllowedTypes(dir, 'studio');
    expect(getWorkspaceRaw).not.toHaveBeenCalled();
    expect(putWorkspaceRaw).not.toHaveBeenCalled();
  });

  it('derives the workspace name from the filename when the YAML omits it', async () => {
    writeWs(dir, 'locations.yaml', 'allowed_root_node_types:\n  - studio:Location\n');
    getWorkspaceRaw.mockResolvedValue({
      name: 'locations',
      allowed_root_node_types: ['studio:Folder'],
    });

    await reconcileWorkspaceAllowedTypes(dir, 'studio');
    expect(getWorkspaceRaw).toHaveBeenCalledWith('studio', 'locations');
    expect(putWorkspaceRaw).toHaveBeenCalledTimes(1);
    expect(putWorkspaceRaw.mock.calls[0][2].allowed_root_node_types).toEqual(['studio:Location']);
  });
});
