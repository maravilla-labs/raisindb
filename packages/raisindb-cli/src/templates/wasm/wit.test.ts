import { describe, expect, it } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { RAISIN_WIT } from './wit.js';

const here = path.dirname(fileURLToPath(import.meta.url));
const canonical = path.resolve(here, '../../../../../crates/raisin-functions/wit/raisin-function.wit');

describe('RAISIN_WIT', () => {
  it('is byte-identical to the canonical contract', () => {
    // Scaffolds that carry the WIT (TinyGo) must carry THE contract. A guest
    // built against a drifted copy fails to link against the host with an
    // error that names neither file, so this is checked here instead.
    if (!fs.existsSync(canonical)) return; // published CLI: no monorepo around
    expect(RAISIN_WIT).toBe(fs.readFileSync(canonical, 'utf-8'));
  });

  it('declares the name-routed handler export', () => {
    expect(RAISIN_WIT).toContain('export handler: func(name: string, input: string)');
  });
});
